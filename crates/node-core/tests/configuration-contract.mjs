import assert from 'node:assert/strict';

/** Exercise real callers against a recording atomic-store oracle, not a concurrency proof. */
export async function checkConfigurationContract({
  register, fetch, bind, expectedId, formId, value, stored, error,
}) {
  const calls = [];
  const bindings = [];
  let changes = 0;
  let handler;
  let current = stored;
  const iii = {
    async trigger(request) {
      calls.push(structuredClone(request));
      if (request.function_id === 'configuration::register') {
        assert.fail('atomic initialization must not call legacy registration');
      }
      if (request.function_id === 'configuration::ensure') {
        if (error) throw error;
        const empty = current == null;
        if (empty) current = structuredClone(request.payload.initial_value);
        return {
          action: empty ? 'seeded' : 'preserved',
          entry: { ...request.payload, value: current },
        };
      }
      if (request.function_id === 'configuration::get') return { value: current };
      assert.fail(`unexpected RPC: ${request.function_id}`);
    },
    registerFunction(_id, fn) { handler = fn; },
    registerTrigger(request) { bindings.push(request); },
  };
  if (error) {
    await assert.rejects(() => register(iii), (caught) => error.code === 'function_not_found'
      ? caught instanceof Error && caught.message.includes('upgrade engine')
      : caught === error);
    assert.deepEqual(calls.map((call) => call.function_id), ['configuration::ensure']);
    assert.deepEqual(current, stored);
    return;
  }
  await register(iii);
  assert.deepEqual(calls.map((call) => call.function_id), ['configuration::ensure']);
  const registration = calls[0];
  assert.equal(registration.payload.id, expectedId);
  assert.deepEqual(registration.payload.metadata, { ui_form: formId });
  assert.deepEqual(registration.payload.initial_value, value);
  assert.deepEqual(current, stored == null ? value : stored);
  // Runtime reads/reloads follow initialization, and are not seed-decision reads.
  current = value;
  assert.deepEqual(await fetch(iii), value);
  await bind(iii, async () => { changes += 1; });
  const before = changes;
  await handler({});
  assert.equal(changes, before + 1);
  assert.equal(bindings.length, 1);
  assert.equal(bindings[0].config.configuration_id, expectedId);
  for (const call of calls) {
    assert.equal(call.namespace, 'default');
    assert.equal(call.payload.id, expectedId);
  }
}

/** Every consumer runs these cases against its own initialization and reload functions. */
export const configurationCases = [
  { title: 'custom identity preserves existing configuration', env: '  project-worker-0123456789abcdef  ', stored: {} },
  { title: 'custom identity seeds a missing entry', env: 'project-worker-0123456789abcdef' },
  { title: 'empty identity uses the legacy fallback', env: '', stored: null },
  { title: 'whitespace identity uses the legacy fallback', env: ' \t ', stored: null },
  { title: 'absent identity uses the legacy fallback', stored: null },
  { title: 'false stored value is preserved', env: 'project-worker', stored: false },
  { title: 'zero stored value is preserved', env: 'project-worker', stored: 0 },
  { title: 'empty string stored value is preserved', env: 'project-worker', stored: '' },
  { title: 'missing ensure fails closed with an upgrade error', env: 'project-worker', error: { code: 'function_not_found' } },
  { title: 'an error mentioning a missing function keeps its identity', env: 'project-worker', error: { code: 'OTHER', message: 'function_not_found NOT_FOUND' } },
  { title: 'NOT_FOUND from ensure is not a seeding instruction', env: 'project-worker', error: { code: 'NOT_FOUND' } },
  { title: 'transport failures never authorize a legacy write', env: 'project-worker', error: new Error('connection lost') },
];
