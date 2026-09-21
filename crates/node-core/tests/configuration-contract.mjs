import assert from 'node:assert/strict';

/** Exercise real callers with a recording store. This is not a concurrency proof. */
export async function checkConfigurationContract({
  register, fetch, bind, expectedId, formId, value, stored, error,
  legacy = false, getError, malformed, registerError, makeError, probeUpgrade = false,
}) {
  const calls = [];
  const bindings = [];
  let changes = 0;
  let handler;
  let current = stored;
  const iii = {
    async trigger(request) {
      calls.push(structuredClone(request));
      assert.equal(request.namespace, 'default');
      assert.equal(request.payload.id, expectedId);
      if (request.function_id === 'configuration::ensure') {
        if (error) throw error;
        if (legacy) throw makeError
          ? makeError({ code: 'function_not_found', message: 'ensure absent', function_id: 'configuration::ensure' })
          : { code: 'function_not_found', message: 'ensure absent' };
        const empty = current == null;
        if (empty) current = structuredClone(request.payload.initial_value);
        return { action: empty ? 'seeded' : 'preserved', entry: { ...request.payload, value: current } };
      }
      if (request.function_id === 'configuration::get') {
        if (legacy) {
          assert.equal(request.payload.raw, true, 'legacy decision must not expand templates');
          if (getError) throw getError;
          if (malformed) return malformed.response;
          if (current === undefined) throw { code: 'NOT_FOUND', message: 'entry absent' };
        }
        return { value: current };
      }
      if (request.function_id === 'configuration::register') {
        assert.ok(legacy, 'modern initialization must not use legacy registration');
        if (registerError) throw registerError;
        if (Object.hasOwn(request.payload, 'initial_value')) {
          current = structuredClone(request.payload.initial_value);
        }
        return { ...request.payload, value: current };
      }
      assert.fail(`unexpected RPC: ${request.function_id}`);
    },
    registerFunction(_id, fn) { handler = fn; },
    registerTrigger(request) { bindings.push(request); },
  };
  const failure = error || ((getError?.code !== 'NOT_FOUND' || getError?.function_id) && getError) || registerError;
  if (failure || malformed) {
    await assert.rejects(() => register(iii), (caught) => malformed
      ? caught instanceof Error && caught.message.includes('`value`')
      : caught === failure);
    if (error) assert.ok(calls.every((call) => call.function_id === 'configuration::ensure'));
    if (!registerError) assert.ok(!calls.some((call) => call.function_id === 'configuration::register'));
    assert.deepEqual(current, stored);
    return;
  }
  if (probeUpgrade) {
    const warnings = [];
    const originalWarn = console.warn;
    console.warn = (...args) => warnings.push(args.join(' '));
    try {
      await register(iii);
      await register(iii);
    } finally {
      console.warn = originalWarn;
    }
    assert.equal(warnings.length, 1, 'compatibility warning should be once per entry');
    assert.equal(warnings[0], expectedId + ': engine lacks configuration::ensure; using non-atomic legacy initialization; upgrade to >=0.24.1 for concurrent-write safety');
    calls.splice(3);
  } else {
    await register(iii);
  }
  assert.deepEqual(calls.map((call) => call.function_id), legacy
    ? ['configuration::ensure', 'configuration::get', 'configuration::register']
    : ['configuration::ensure']);
  const registration = calls.at(-1);
  assert.equal(registration.payload.id, expectedId);
  assert.deepEqual(registration.payload.metadata, { ui_form: formId });
  assert.deepEqual(calls[0].payload.initial_value, value);
  if (legacy) {
    const expected = { ...calls[0].payload };
    if (stored != null && getError?.code !== 'NOT_FOUND') delete expected.initial_value;
    assert.deepEqual(registration.payload, expected, 'schema, metadata and seed identity must survive');
    assert.ok(!Object.hasOwn(registration.payload, 'value'));
  }
  assert.deepEqual(current, stored == null || getError?.code === 'NOT_FOUND' ? value : stored);
  // Runtime reads/reloads are separate from the seed-decision read.
  legacy = false;
  if (probeUpgrade) {
    calls.length = 0;
    await register(iii);
    assert.deepEqual(calls.map((call) => call.function_id), ['configuration::ensure']);
  }
  current = value;
  assert.deepEqual(await fetch(iii), value);
  if (bind) {
    await bind(iii, async () => { changes += 1; });
    const before = changes;
    await handler({});
    assert.equal(changes, before + 1);
    assert.equal(bindings.length, 1);
    assert.equal(bindings[0].config.configuration_id, expectedId);
  }
}

const fault = (code) => ({ code, message: 'function_not_found NOT_FOUND' });
export const configurationCases = [
  { title: 'custom identity preserves existing configuration', env: '  project-worker-0123456789abcdef  ', stored: {} },
  { title: 'custom identity seeds a missing entry', env: 'project-worker-0123456789abcdef' },
  ...['', ' \t ', undefined].map((env) => ({ title: `default identity ${JSON.stringify(env)}`, env, stored: null })),
  ...[false, 0, '', {}, { template: '${UNCHANGED:default}' }].flatMap((stored) => [false, true].map((legacy) => ({
    title: `${legacy ? 'legacy' : 'modern'} preserves ${JSON.stringify(stored)}`,
    env: 'project-worker', stored, legacy,
  }))),
  { title: 'legacy seeds exact missing entry', legacy: true },
  { title: 'SDK missing-function error permits legacy and capability is rechecked after upgrade', env: 'compat-upgrade', legacy: true, stored: {}, probeUpgrade: true },
  { title: 'legacy seeds explicit null', legacy: true, stored: null },
  ...['OTHER', 'NOT_FOUND', 'SCHEMA_INVALID', 'ADAPTER_ERROR', 'permission_denied', 'FUNCTION_NOT_FOUND', 'RESOURCE_NOT_FOUND'].map((code) => ({
    title: `${code} from ensure never authorizes fallback`, error: fault(code),
  })),
  { title: 'missing-function error from another invocation is not a capability proof', error: { code: 'function_not_found', function_id: 'other::call' } },
  { title: 'missing-entry error from another invocation never seeds', legacy: true, getError: { code: 'NOT_FOUND', function_id: 'other::call' } },
  { title: 'message-only missing function never authorizes fallback', error: new Error('function_not_found') },
  { title: 'timeout never authorizes fallback', error: new Error('timeout') },
  { title: 'disconnect never authorizes fallback', error: new Error('connection lost') },
  ...['function_not_found', 'OTHER', 'NOT_REGISTERED', 'RESOURCE_NOT_FOUND', 'ADAPTER_ERROR', 'SCHEMA_INVALID', 'permission_denied'].map((code) => ({
    title: `legacy get ${code} never seeds`, legacy: true, getError: fault(code),
  })),
  { title: 'legacy lookup transport failure never writes', legacy: true, getError: new Error('connection lost') },
  ...[{}, [], null, false, { value: undefined }].map((response, index) => ({
    title: `malformed legacy response ${index} never writes`, legacy: true, malformed: { response },
  })),
  { title: 'legacy registration failure propagates', legacy: true, registerError: fault('SCHEMA_INVALID') },
];
