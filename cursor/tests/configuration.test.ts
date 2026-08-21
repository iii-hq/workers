import {
  API_KEY_ENV_REFERENCE,
  BRIDGE_BIN_ENV_REFERENCE,
  CURSOR_AGENT_BIN_ENV_REFERENCE,
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

    await registerCursorConfig(iii.asClient());
    const runtime = await fetchRuntime(iii.asClient());

    expect(runtime.api_key).toBe('key_runtime');
    const registration = iii.triggerCalls.find(
      (call) => call.function_id === 'configuration::register',
    );
    expect(registration?.payload).toMatchObject({
      id: 'cursor',
      name: 'Cursor',
      initial_value: expect.objectContaining({ api_key: API_KEY_ENV_REFERENCE }),
      schema: expect.objectContaining({ type: 'object' }),
    });
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

  it('validates configuration change events', () => {
    expect(ConfigChangeEventSchema.parse({ id: 'cursor', future: true })).toMatchObject({
      id: 'cursor',
    });
    expect(() => ConfigChangeEventSchema.parse({ id: 7 })).toThrow();
  });
});
