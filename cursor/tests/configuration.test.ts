import {
  API_KEY_ENV_REFERENCE,
  BRIDGE_BIN_ENV_REFERENCE,
  CURSOR_AGENT_BIN_ENV_REFERENCE,
  bridgeLaunchOptions,
  configId,
  cursorCliLaunchOptions,
  defaultConfig,
  requireApiKey,
} from '../src/config.js';
import {
  bindConfigTrigger,
  ConfigChangeEventSchema,
  fetchRuntime,
  registerCursorConfig,
} from '../src/configuration.js';
import { MockIII } from './helpers.js';

describe('Cursor configuration', () => {
  afterEach(() => {
    delete process.env.III_CONFIG_NAME;
    delete process.env.CURSOR_SDK_BRIDGE_BIN;
  });

  it('resolves the SDK Bridge placeholder through the operator environment', () => {
    process.env.CURSOR_SDK_BRIDGE_BIN = '/opt/cursor-sdk-bridge';
    expect(
      bridgeLaunchOptions({ ...defaultConfig(), api_key: 'key_runtime' }, '/repo'),
    ).toMatchObject({ binary: '/opt/cursor-sdk-bridge', workspace: '/repo' });
    delete process.env.CURSOR_SDK_BRIDGE_BIN;
    expect(
      bridgeLaunchOptions({ ...defaultConfig(), api_key: 'key_runtime' }, '/repo'),
    ).toMatchObject({ binary: '' });
  });

  it('uses built-in environment references and a configurable id', () => {
    expect(defaultConfig()).toMatchObject({
      local_backend: 'cli-acp',
      agent_binary: CURSOR_AGENT_BIN_ENV_REFERENCE,
      api_key: API_KEY_ENV_REFERENCE,
      bridge_binary: BRIDGE_BIN_ENV_REFERENCE,
      events_stream: 'agent::events',
      raw_events_stream: 'cursor::events',
    });
    expect(() => requireApiKey(defaultConfig())).toThrow('Cursor API key is not configured');
    expect(cursorCliLaunchOptions(defaultConfig(), '/repo')).toMatchObject({
      workspace: '/repo',
      startupTimeoutMs: 30_000,
      shutdownTimeoutMs: 5_000,
      rpcTimeoutMs: 60_000,
      maxFrameBytes: 16 * 1024 * 1024,
    });
    process.env.III_CONFIG_NAME = 'cursor-team';
    expect(configId()).toBe('cursor-team');
  });

  it('registers and fetches the configuration through typed worker calls', async () => {
    const iii = new MockIII();
    iii.configValue = { ...defaultConfig(), api_key: 'key_runtime' };
    process.env.III_CONFIG_NAME = 'cursor-team';

    await registerCursorConfig(iii.asClient());
    expect(iii.triggerCalls.map((call) => call.function_id)).toEqual(['configuration::ensure']);
    const runtime = await fetchRuntime(iii.asClient());

    expect(runtime.api_key).toBe('key_runtime');
    const registration = iii.triggerCalls.find(
      (call) => call.function_id === 'configuration::ensure',
    );
    expect(registration?.payload).toMatchObject({
      id: 'cursor-team',
      name: 'Cursor',
      metadata: { ui_form: 'cursor' },
      schema: expect.objectContaining({ type: 'object' }),
    });
    expect(registration?.payload).toHaveProperty('initial_value', defaultConfig());
  });

  it('re-fetches persisted values on updates and retains the last good config on failure', async () => {
    const iii = new MockIII();
    const holder = { current: { ...defaultConfig(), api_key: 'key_old' } };
    iii.configValue = { ...defaultConfig(), api_key: 'key_first' };
    await bindConfigTrigger(iii.asClient(), holder);
    expect(holder.current.api_key).toBe('key_first');

    const reload = iii.functions.get('cursor::on-config-change');
    expect(reload?.options.metadata).toEqual({ internal: true });
    iii.configValue = { ...defaultConfig(), api_key: 'key_second' };
    await reload?.handler({ id: 'forged', value: { api_key: 'attacker' } });
    expect(holder.current.api_key).toBe('key_second');

    iii.configValue = { ...defaultConfig(), api_key: 123 };
    await expect(reload?.handler({ id: 'cursor' })).resolves.toEqual({ ok: false });
    expect(holder.current.api_key).toBe('key_second');
  });

  it('seeds a null entry but preserves a stored value even with an explicit seed', async () => {
    const iii = new MockIII();
    const seed = { ...defaultConfig(), workspace: '/seed' };
    iii.configValue = null;
    await registerCursorConfig(iii.asClient(), seed);
    expect(
      iii.triggerCalls.find((call) => call.function_id === 'configuration::ensure')?.payload,
    ).toHaveProperty('initial_value', seed);
    iii.triggerCalls.length = 0;
    iii.configValue = { ...defaultConfig(), workspace: '/compose' };
    await registerCursorConfig(iii.asClient(), seed);
    expect(
      iii.triggerCalls.find((call) => call.function_id === 'configuration::ensure')?.payload,
    ).toHaveProperty('initial_value', seed);
    expect(iii.configValue).toMatchObject({ workspace: '/compose' });
    expect(iii.triggerCalls.map((call) => call.function_id)).toEqual(['configuration::ensure']);
  });

  it('uses legacy registration immediately when ensure is unavailable', async () => {
    const iii = new MockIII();
    iii.ensureError = { code: 'function_not_found' };
    await registerCursorConfig(iii.asClient());
    expect(iii.triggerCalls.map((call) => call.function_id)).toEqual([
      'configuration::ensure', 'configuration::get', 'configuration::register',
    ]);
    expect(iii.triggerCalls[1]?.payload).toMatchObject({ raw: true });
    expect(iii.triggerCalls[2]?.payload).not.toHaveProperty('initial_value');
    expect(iii.configValue).toEqual(defaultConfig());
  });

  it('does not misclassify an unrelated message as a missing capability', async () => {
    const iii = new MockIII();
    const error = Object.assign(new Error('function_not_found'), { code: 'OTHER' });
    iii.ensureError = error;
    await expect(registerCursorConfig(iii.asClient())).rejects.toBe(error);
    expect(iii.triggerCalls.every((call) => call.function_id === 'configuration::ensure')).toBe(
      true,
    );
  });

  it('validates configuration change events', () => {
    expect(ConfigChangeEventSchema.parse({ id: 'cursor', future: true })).toMatchObject({
      id: 'cursor',
    });
    expect(() => ConfigChangeEventSchema.parse({ id: 7 })).toThrow();
  });
});
