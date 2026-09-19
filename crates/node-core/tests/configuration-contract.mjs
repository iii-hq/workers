import assert from 'node:assert/strict';

/** Exercise real worker functions against a recording configuration service. */
export async function checkConfigurationContract({
  register, fetch, bind, expectedId, formId, value, stored, error,
}) {
  const calls = [];
  const bindings = [];
  let changes = 0;
  let handler;
  const iii = {
    async trigger(request) {
      calls.push(structuredClone(request));
      if (request.function_id === 'configuration::get') {
        if (error) throw error;
        return { value: stored };
      }
      return {};
    },
    registerFunction(_id, fn) { handler = fn; },
    registerTrigger(request) { bindings.push(request); },
  };
  if (error && error.code !== 'NOT_FOUND') {
    await assert.rejects(() => register(iii), (caught) => caught === error);
    assert.equal(calls.some((call) => call.function_id === 'configuration::register'), false);
    return;
  }
  await register(iii);
  const registration = calls.find((call) => call.function_id === 'configuration::register');
  assert.equal(registration.payload.id, expectedId);
  assert.deepEqual(registration.payload.metadata, { ui_form: formId });
  if (stored != null) {
    assert.equal(Object.hasOwn(registration.payload, 'initial_value'), false);
  } else {
    assert.deepEqual(registration.payload.initial_value, value);
  }
  // Stop simulating a missing entry: the next read is the live value.
  error = undefined;
  stored = value;
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

export const configurationCases = [
  { title: 'custom identity preserves existing configuration', env: '  project-worker-0123456789abcdef  ', stored: {} },
  { title: 'custom identity seeds a missing entry', env: 'project-worker-0123456789abcdef', error: { code: 'NOT_FOUND' } },
  { title: 'empty identity uses the legacy fallback', env: '', stored: null },
  { title: 'whitespace identity uses the legacy fallback', env: ' \t ', stored: null },
  { title: 'absent identity uses the legacy fallback', stored: null },
  { title: 'false stored value is not treated as absent', env: 'project-worker', stored: false },
  { title: 'function_not_found never authorizes overwriting configuration', env: 'project-worker', error: { code: 'function_not_found' } },
  { title: 'transport failures never authorize overwriting configuration', env: 'project-worker', error: new Error('connection lost') },
];
