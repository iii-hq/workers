import { EventEmitter } from 'node:events';
import type {
  CursorAcpSessionUpdate,
  CursorCliAuthStatus,
  CursorCliClient,
  CursorCliFactory,
  CursorCliLaunchOptions,
} from '../src/cli.js';
import {
  CursorProvider,
  cursorCatalogModels,
  cursorProviderDeclaration,
  cursorProviderPrompt,
} from '../src/provider.js';
import type { CursorModel } from '../src/types.js';
import { MockIII, testConfig } from './helpers.js';

describe('Cursor LLM Router provider', () => {
  it('declares an auth-owned provider and reconciles prefixed account models', async () => {
    const iii = new MockIII();
    const routerCalls: Array<Record<string, unknown>> = [];
    installRouter(iii, routerCalls);
    const cli = new FakeCursorCliFactory();
    const provider = new CursorProvider(iii.asClient(), testConfig, cli, fakeWorkspace());

    provider.register();
    await vi.waitFor(() => {
      expect(routerCalls.some((call) => call.function_id === 'router::models::reconcile')).toBe(
        true,
      );
    });

    const declaration = routerCalls.find(
      (call) => call.function_id === 'router::provider::register',
    )?.payload as Record<string, unknown>;
    expect(declaration).toMatchObject({
      id: 'cursor',
      display_name: 'Cursor',
      supports_model_listing: true,
      worker_id: 'cursor',
    });
    expect(declaration).not.toHaveProperty('credential_env_var');
    expect(declaration.system_prompt).toContain('Answer the user directly in text');
    const reconciliation = routerCalls.find(
      (call) => call.function_id === 'router::models::reconcile',
    )?.payload as { provider: string; models: Array<Record<string, unknown>>; token?: string };
    expect(reconciliation.provider).toBe('cursor');
    expect(reconciliation.token).toBe('cursor-registration-token');
    expect(reconciliation.models).toEqual([
      expect.objectContaining({
        id: 'cursor/auto',
        provider: 'cursor',
        supports_tools: false,
      }),
      expect.objectContaining({
        id: 'cursor/composer-2.5',
        provider: 'cursor',
        supports_tools: false,
      }),
    ]);
    expect(iii.functions.has('provider::cursor::stream')).toBe(true);
    expect(iii.functions.has('provider::cursor::abort')).toBe(true);
    expect(iii.functions.has('provider::cursor::refresh_models')).toBe(true);
    expect(iii.functions.get('provider::cursor::stream')?.options.request_format).toMatchObject({
      additionalProperties: {},
      properties: { session_id: { type: 'string' } },
    });
    expect(iii.functions.get('provider::cursor::abort')?.options.request_format).toMatchObject({
      additionalProperties: {},
    });
    await provider.close();
  });

  it('re-registers after router ready arrives during a catalog refresh', async () => {
    const iii = new MockIII();
    const routerCalls: Array<Record<string, unknown>> = [];
    installRouter(iii, routerCalls);
    const cli = new FakeCursorCliFactory();
    let releaseFirst: (() => void) | undefined;
    let listCalls = 0;
    cli.listModelsImpl = async () => {
      listCalls += 1;
      if (listCalls === 1) {
        await new Promise<void>((resolve) => {
          releaseFirst = resolve;
        });
      }
      return models();
    };
    const provider = new CursorProvider(iii.asClient(), testConfig, cli, fakeWorkspace());
    provider.register();
    await vi.waitFor(() => expect(listCalls).toBe(1));

    await expect(
      iii.functions.get('provider::cursor::on_router_ready')?.handler({}),
    ).resolves.toEqual({ ok: true });
    releaseFirst?.();

    await vi.waitFor(() => {
      expect(
        routerCalls.filter((call) => call.function_id === 'router::provider::register'),
      ).toHaveLength(2);
      expect(listCalls).toBe(2);
    });
    await provider.close();
  });

  it('persists an accepted registration token without re-registering after a transient state failure', async () => {
    const iii = new MockIII();
    let stateSetAttempts = 0;
    const baseTrigger = iii.trigger.bind(iii);
    iii.trigger = async (request: Record<string, unknown>) => {
      if (request.function_id === 'state::set' && stateSetAttempts++ === 0) {
        throw new Error('state temporarily unavailable');
      }
      return baseTrigger(request);
    };
    const routerCalls: Array<Record<string, unknown>> = [];
    installRouter(iii, routerCalls);
    const provider = new CursorProvider(
      iii.asClient(),
      testConfig,
      new FakeCursorCliFactory(),
      fakeWorkspace(),
    );

    provider.register();
    await vi.waitFor(
      () => {
        expect(
          routerCalls.filter((call) => call.function_id === 'router::models::reconcile'),
        ).toHaveLength(1);
      },
      { timeout: 2_000 },
    );

    expect(
      routerCalls.filter((call) => call.function_id === 'router::provider::register'),
    ).toHaveLength(1);
    expect(stateSetAttempts).toBe(2);
    await provider.close();
  });

  it('preserves a registration token minted during a concurrent stale state read', async () => {
    const iii = new MockIII();
    const baseTrigger = iii.trigger.bind(iii);
    let stateReads = 0;
    let releaseRegistration: (() => void) | undefined;
    let markRegistrationStarted: (() => void) | undefined;
    const registrationStarted = new Promise<void>((resolve) => {
      markRegistrationStarted = resolve;
    });
    let releaseStaleRead: (() => void) | undefined;
    let markStaleReadStarted: (() => void) | undefined;
    const staleReadStarted = new Promise<void>((resolve) => {
      markStaleReadStarted = resolve;
    });
    const reconciliations: Array<Record<string, unknown>> = [];
    iii.trigger = async (request: Record<string, unknown>) => {
      if (request.function_id === 'state::get') {
        stateReads += 1;
        if (stateReads === 1) return null;
        markStaleReadStarted?.();
        return new Promise((resolve) => {
          releaseStaleRead = () => resolve(null);
        });
      }
      if (request.function_id === 'router::provider::register') {
        markRegistrationStarted?.();
        await new Promise<void>((resolve) => {
          releaseRegistration = resolve;
        });
        return { registration_token: 'concurrent-registration-token' };
      }
      if (request.function_id === 'router::models::reconcile') {
        reconciliations.push(structuredClone(request.payload as Record<string, unknown>));
        return { provider: 'cursor', count: 2 };
      }
      return baseTrigger(request);
    };
    const provider = new CursorProvider(
      iii.asClient(),
      testConfig,
      new FakeCursorCliFactory(),
      fakeWorkspace(),
    );

    const declaration = provider.declareOnce();
    await registrationStarted;
    const refresh = provider.refreshModels();
    await staleReadStarted;
    releaseRegistration?.();
    await declaration;
    releaseStaleRead?.();
    await refresh;

    expect(reconciliations).toEqual([
      expect.objectContaining({ token: 'concurrent-registration-token' }),
    ]);
    await provider.close();
  });

  it('retries persistence when the router returns an in-memory token after repeated state failures', async () => {
    const iii = new MockIII();
    let stateSetAttempts = 0;
    const baseTrigger = iii.trigger.bind(iii);
    iii.trigger = async (request: Record<string, unknown>) => {
      if (request.function_id === 'state::set') {
        stateSetAttempts += 1;
        if (stateSetAttempts <= 6) throw new Error('state still unavailable');
      }
      return baseTrigger(request);
    };
    const routerCalls: Array<Record<string, unknown>> = [];
    installRouter(iii, routerCalls);
    const provider = new CursorProvider(
      iii.asClient(),
      testConfig,
      new FakeCursorCliFactory(),
      fakeWorkspace(),
    );

    provider.register();
    await vi.waitFor(
      () => {
        expect(iii.state.get('registration_token')).toBe('cursor-registration-token');
        expect(
          routerCalls.filter((call) => call.function_id === 'router::models::reconcile'),
        ).toHaveLength(1);
      },
      { timeout: 6_000 },
    );

    expect(
      routerCalls.filter((call) => call.function_id === 'router::provider::register'),
    ).toHaveLength(2);
    expect(stateSetAttempts).toBe(7);
    await provider.close();
  });

  it('reloads an absent token after another provider instance claims registration', async () => {
    const iii = new MockIII();
    const baseTrigger = iii.trigger.bind(iii);
    const declarations: Array<Record<string, unknown>> = [];
    let reconciled = false;
    iii.trigger = async (request: Record<string, unknown>) => {
      if (request.function_id === 'router::provider::register') {
        declarations.push(structuredClone(request.payload as Record<string, unknown>));
        if (declarations.length === 1) {
          iii.state.set('registration_token', 'winner-registration-token');
          throw new Error('provider registration is already owned');
        }
        return { registration_token: 'winner-registration-token' };
      }
      if (request.function_id === 'router::models::reconcile') {
        reconciled = true;
        return { provider: 'cursor', count: 2 };
      }
      return baseTrigger(request);
    };
    const provider = new CursorProvider(
      iii.asClient(),
      testConfig,
      new FakeCursorCliFactory(),
      fakeWorkspace(),
    );

    provider.register();
    await vi.waitFor(() => expect(reconciled).toBe(true), { timeout: 2_000 });

    expect(declarations).toHaveLength(2);
    expect(declarations[0]).not.toHaveProperty('token');
    expect(declarations[1]).toHaveProperty('token', 'winner-registration-token');
    await provider.close();
  });

  it('persists a registration token returned during graceful shutdown', async () => {
    const iii = new MockIII();
    const baseTrigger = iii.trigger.bind(iii);
    let completeRegistration: ((value: unknown) => void) | undefined;
    let markRegistrationStarted: (() => void) | undefined;
    const registrationStarted = new Promise<void>((resolve) => {
      markRegistrationStarted = resolve;
    });
    iii.trigger = async (request: Record<string, unknown>) => {
      if (request.function_id === 'router::provider::register') {
        markRegistrationStarted?.();
        return new Promise((resolve) => {
          completeRegistration = resolve;
        });
      }
      return baseTrigger(request);
    };
    const provider = new CursorProvider(
      iii.asClient(),
      testConfig,
      new FakeCursorCliFactory(),
      fakeWorkspace(),
    );
    provider.register();
    await registrationStarted;

    const closing = provider.close();
    completeRegistration?.({ registration_token: 'shutdown-registration-token' });
    await closing;

    expect(iii.state.get('registration_token')).toBe('shutdown-registration-token');
  });

  it('streams a login-backed ACP response through the provider channel', async () => {
    const iii = new MockIII();
    installRouter(iii, []);
    const cli = new FakeCursorCliFactory(async (_sessionId, prompt, onUpdate) => {
      expect(prompt).toContain('System instructions:\nAnswer precisely.');
      expect(prompt).toContain('user: reply exactly');
      await onUpdate(update('agent_thought_chunk', 'brief thought'));
      await onUpdate(update('agent_message_chunk', 'cursor-provider-ok'));
      return 'end_turn';
    });
    const removed: string[] = [];
    const provider = new CursorProvider(iii.asClient(), testConfig, cli, {
      create: async () => '/tmp/cursor-provider-test',
      remove: async (path) => {
        removed.push(path);
      },
    });
    provider.register();
    const writer = new FakeWriter();

    const response = await iii.functions.get('provider::cursor::stream')?.handler({
      writer_ref: writer,
      system_prompt: 'Answer precisely.',
      model: 'cursor/auto',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'reply exactly' }] }],
      tools: [{ name: 'agent_trigger' }],
      max_output_tokens: 100,
      resolution_key: 'cursor-provider-request',
    });

    expect(response).toEqual({ ok: true });
    expect(cli.clients).toHaveLength(1);
    expect(cli.clients[0]?.models).toEqual(['default']);
    expect(cli.clients[0]?.modes).toEqual(['ask']);
    expect(cli.clients[0]?.newSessions).toEqual(['/tmp/cursor-provider-test']);
    expect(cli.clients[0]?.closes).toBe(1);
    expect(removed).toEqual(['/tmp/cursor-provider-test']);
    expect(writer.closed).toBe(true);
    expect(writer.frames.map((frame) => frame.type)).toEqual([
      'ping',
      'start',
      'thinking_start',
      'thinking_delta',
      'thinking_end',
      'text_start',
      'text_delta',
      'text_end',
      'stop',
      'done',
    ]);
    expect(writer.frames.find((frame) => frame.type === 'text_delta')).toMatchObject({
      delta: 'cursor-provider-ok',
    });
    expect(writer.frames.at(-1)).toMatchObject({
      type: 'done',
      message: {
        provider: 'cursor',
        model: 'cursor/auto',
        stop_reason: 'end',
        content: [
          { type: 'thinking', text: 'brief thought' },
          { type: 'text', text: 'cursor-provider-ok' },
        ],
        warnings: [expect.stringContaining('tools')],
      },
    });
    await provider.close();
  });

  it('returns an auth terminal and removes the catalog when Cursor is logged out', async () => {
    const iii = new MockIII();
    const routerCalls: Array<Record<string, unknown>> = [];
    installRouter(iii, routerCalls);
    const cli = new FakeCursorCliFactory();
    cli.authenticated = false;
    const provider = new CursorProvider(iii.asClient(), testConfig, cli, fakeWorkspace());
    provider.register();
    const writer = new FakeWriter();

    await iii.functions.get('provider::cursor::stream')?.handler({
      writer_ref: writer,
      model: 'cursor/auto',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'hello' }] }],
      resolution_key: 'logged-out',
    });
    await vi.waitFor(() => {
      expect(
        routerCalls.some(
          (call) =>
            call.function_id === 'router::models::reconcile' &&
            Array.isArray((call.payload as { models?: unknown }).models) &&
            (call.payload as { models: unknown[] }).models.length === 0,
        ),
      ).toBe(true);
    });

    expect(writer.frames).toEqual([
      { type: 'ping' },
      expect.objectContaining({
        type: 'error',
        error: expect.objectContaining({
          error_kind: 'auth_expired',
          error_message: 'Cursor is not logged in; run cursor-agent login',
        }),
      }),
    ]);
    expect(cli.clients).toHaveLength(0);
    await provider.close();
  });

  it('contains provider-channel errors and cancels the active ACP session', async () => {
    const iii = new MockIII();
    installRouter(iii, []);
    let finish: (() => void) | undefined;
    const cli = new FakeCursorCliFactory(async (_sessionId, _prompt, onUpdate) => {
      await onUpdate(update('agent_message_chunk', 'started'));
      await new Promise<void>((resolve) => {
        finish = resolve;
      });
      return 'cancelled';
    });
    const provider = new CursorProvider(iii.asClient(), testConfig, cli, fakeWorkspace());
    provider.register();
    const writer = new FakeWriter();

    const stream = iii.functions.get('provider::cursor::stream')?.handler({
      writer_ref: writer,
      model: 'cursor/auto',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'hello' }] }],
      resolution_key: 'channel-error',
    });
    await vi.waitFor(() => expect(cli.clients).toHaveLength(1));
    await vi.waitFor(() =>
      expect(writer.frames.some((frame) => frame.type === 'text_delta')).toBe(true),
    );

    writer.stream.emit('error', new Error('channel disconnected'));

    await vi.waitFor(() =>
      expect(cli.clients[0]?.cancellations).toEqual(['cursor-provider-session']),
    );
    finish?.();
    await expect(stream).resolves.toEqual({ ok: true });
    expect(writer.closed).toBe(true);
    await provider.close();
  });

  it('cancels the active ACP session when the provider channel closes normally', async () => {
    const iii = new MockIII();
    installRouter(iii, []);
    let finish: (() => void) | undefined;
    const cli = new FakeCursorCliFactory(async (_sessionId, _prompt, onUpdate) => {
      await onUpdate(update('agent_message_chunk', 'started'));
      await new Promise<void>((resolve) => {
        finish = resolve;
      });
      return 'cancelled';
    });
    const provider = new CursorProvider(iii.asClient(), testConfig, cli, fakeWorkspace());
    provider.register();
    const writer = new FakeWriter();

    const stream = iii.functions.get('provider::cursor::stream')?.handler({
      writer_ref: writer,
      model: 'cursor/auto',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'hello' }] }],
      resolution_key: 'channel-close',
    });
    await vi.waitFor(() =>
      expect(writer.frames.some((frame) => frame.type === 'text_delta')).toBe(true),
    );

    writer.stream.emit('close');

    await vi.waitFor(() =>
      expect(cli.clients[0]?.cancellations).toEqual(['cursor-provider-session']),
    );
    finish?.();
    await expect(stream).resolves.toEqual({ ok: true });
    await provider.close();
  });

  it('does not start an ACP session after the provider channel has already closed', async () => {
    const iii = new MockIII();
    installRouter(iii, []);
    const cli = new FakeCursorCliFactory();
    const provider = new CursorProvider(iii.asClient(), testConfig, cli, fakeWorkspace());
    provider.register();
    const writer = new FakeWriter(true);

    const stream = iii.functions.get('provider::cursor::stream')?.handler({
      writer_ref: writer,
      model: 'cursor/auto',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'hello' }] }],
      resolution_key: 'closed-before-session',
    });
    await expect(stream).resolves.toEqual({ ok: true });
    expect(cli.clients).toHaveLength(0);
    await provider.close();
  });

  it('keeps pre-content transient failures retryable by omitting start', async () => {
    const iii = new MockIII();
    installRouter(iii, []);
    const cli = new FakeCursorCliFactory(async () => {
      throw new Error('rate limit exceeded');
    });
    const provider = new CursorProvider(iii.asClient(), testConfig, cli, fakeWorkspace());
    provider.register();
    const writer = new FakeWriter();

    await iii.functions.get('provider::cursor::stream')?.handler({
      writer_ref: writer,
      model: 'cursor/auto',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'hello' }] }],
    });

    expect(writer.frames.map((frame) => frame.type)).toEqual(['ping', 'error']);
    expect(writer.frames[1]).toMatchObject({ error: { error_kind: 'rate_limited' } });
    await provider.close();
  });

  it('maps the ACP turn-request ceiling to a length stop', async () => {
    const iii = new MockIII();
    installRouter(iii, []);
    const cli = new FakeCursorCliFactory(async () => 'max_turn_requests');
    const provider = new CursorProvider(iii.asClient(), testConfig, cli, fakeWorkspace());
    provider.register();
    const writer = new FakeWriter();

    await iii.functions.get('provider::cursor::stream')?.handler({
      writer_ref: writer,
      model: 'cursor/auto',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'hello' }] }],
    });

    expect(writer.frames.find((frame) => frame.type === 'stop')).toMatchObject({
      stop_reason: 'length',
    });
    expect(writer.frames.at(-1)).toMatchObject({
      type: 'done',
      message: { stop_reason: 'length', native_stop_reason: 'max_turn_requests' },
    });
    await provider.close();
  });

  it('reports an ACP refusal as a permanent error terminal', async () => {
    const iii = new MockIII();
    installRouter(iii, []);
    const cli = new FakeCursorCliFactory(async () => 'refusal');
    const provider = new CursorProvider(iii.asClient(), testConfig, cli, fakeWorkspace());
    provider.register();
    const writer = new FakeWriter();

    await iii.functions.get('provider::cursor::stream')?.handler({
      writer_ref: writer,
      model: 'cursor/auto',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'hello' }] }],
    });

    expect(writer.frames.map((frame) => frame.type)).toEqual(['ping', 'error']);
    expect(writer.frames[1]).toMatchObject({
      error: {
        stop_reason: 'error',
        error_kind: 'permanent',
        error_message: 'Cursor stopped with refusal',
      },
    });
    await provider.close();
  });

  it('accepts an empty canonical request id for provider abort', async () => {
    const iii = new MockIII();
    installRouter(iii, []);
    let finish: (() => void) | undefined;
    const cli = new FakeCursorCliFactory(async () => {
      await new Promise<void>((resolve) => {
        finish = resolve;
      });
      return 'cancelled';
    });
    const provider = new CursorProvider(iii.asClient(), testConfig, cli, fakeWorkspace());
    provider.register();
    const writer = new FakeWriter();
    const stream = iii.functions.get('provider::cursor::stream')?.handler({
      writer_ref: writer,
      model: 'cursor/auto',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'hello' }] }],
      resolution_key: '',
    });
    await vi.waitFor(() => expect(cli.clients).toHaveLength(1));

    await expect(
      iii.functions.get('provider::cursor::abort')?.handler({ request_id: '' }),
    ).resolves.toEqual({ aborted: true });
    expect(cli.clients[0]?.cancellations).toEqual(['cursor-provider-session']);
    finish?.();
    await expect(stream).resolves.toEqual({ ok: true });
    await provider.close();
  });

  it('serializes model refreshes so a stale login result cannot win', async () => {
    const iii = new MockIII();
    const routerCalls: Array<Record<string, unknown>> = [];
    installRouter(iii, routerCalls);
    const cli = new FakeCursorCliFactory();
    let releaseFirst: (() => void) | undefined;
    let firstStarted = false;
    cli.listModelsImpl = async () => {
      firstStarted = true;
      await new Promise<void>((resolve) => {
        releaseFirst = resolve;
      });
      return models();
    };
    const provider = new CursorProvider(iii.asClient(), testConfig, cli, fakeWorkspace());
    await provider.declareOnce();
    routerCalls.length = 0;

    const first = provider.refreshModels();
    await vi.waitFor(() => expect(firstStarted).toBe(true));
    cli.listModelsImpl = async () => models();
    cli.authenticated = false;
    const second = provider.refreshModels();
    releaseFirst?.();
    await Promise.all([first, second]);

    const reconciliations = routerCalls.filter(
      (call) => call.function_id === 'router::models::reconcile',
    );
    expect((reconciliations.at(-1)?.payload as { models: unknown[] }).models).toEqual([]);
    await provider.close();
  });

  it('waits for active stream cleanup during shutdown and rejects late streams', async () => {
    const iii = new MockIII();
    installRouter(iii, []);
    let finish: (() => void) | undefined;
    const cli = new FakeCursorCliFactory(async (_sessionId, _prompt, onUpdate) => {
      await onUpdate(update('agent_message_chunk', 'started'));
      await new Promise<void>((resolve) => {
        finish = resolve;
      });
      return 'cancelled';
    });
    const removed: string[] = [];
    const provider = new CursorProvider(iii.asClient(), testConfig, cli, {
      create: async () => '/tmp/cursor-provider-shutdown',
      remove: async (path) => {
        removed.push(path);
      },
    });
    provider.register();
    const handler = iii.functions.get('provider::cursor::stream')?.handler;
    const stream = handler?.({
      writer_ref: new FakeWriter(),
      model: 'cursor/auto',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'hello' }] }],
      resolution_key: 'shutdown',
    });
    await vi.waitFor(() => expect(cli.clients).toHaveLength(1));

    const closing = provider.close();
    await vi.waitFor(() =>
      expect(cli.clients[0]?.cancellations).toEqual(['cursor-provider-session']),
    );
    finish?.();
    await closing;
    await expect(stream).resolves.toEqual({ ok: true });
    expect(removed).toEqual(['/tmp/cursor-provider-shutdown']);

    await expect(
      handler?.({
        writer_ref: new FakeWriter(),
        model: 'cursor/auto',
        messages: [],
      }),
    ).rejects.toThrow('Cursor provider is closed');
  });

  it('maps catalogs and transcript content without inventing provider credentials', () => {
    expect(cursorProviderDeclaration()).not.toHaveProperty('credential_env_var');
    expect(cursorCatalogModels(models())).toEqual([
      expect.objectContaining({ id: 'cursor/auto', supports_vision: false }),
      expect.objectContaining({ id: 'cursor/composer-2.5', supports_vision: false }),
    ]);
    expect(
      cursorProviderPrompt({
        messages: [
          { role: 'user', content: [{ type: 'text', text: 'first' }] },
          {
            role: 'function_result',
            function_call_id: 'call-1',
            content: [{ type: 'text', text: 'result' }],
          },
        ],
      }),
    ).toContain('function_result: [function result call-1] result');
  });
});

class FakeWriter {
  readonly frames: Array<Record<string, unknown>> = [];
  readonly stream = new EventEmitter();
  closed = false;

  constructor(private readonly closeOnSend = false) {}

  sendMessage(message: string): void {
    if (this.closeOnSend) this.stream.emit('close');
    this.frames.push(JSON.parse(message) as Record<string, unknown>);
  }

  close(): void {
    this.closed = true;
  }
}

class FakeCursorCliClient implements CursorCliClient {
  readonly models: string[] = [];
  readonly modes: string[] = [];
  readonly newSessions: string[] = [];
  readonly cancellations: string[] = [];
  closes = 0;

  constructor(
    private readonly run: (
      sessionId: string,
      prompt: string,
      onUpdate: (update: CursorAcpSessionUpdate) => Promise<void>,
    ) => Promise<'end_turn' | 'max_tokens' | 'max_turn_requests' | 'refusal' | 'cancelled'>,
  ) {}

  async newSession(cwd: string) {
    this.newSessions.push(cwd);
    return {
      sessionId: 'cursor-provider-session',
      models: [
        { modelId: 'default', name: 'Auto' },
        { modelId: 'composer-2.5', name: 'Composer 2.5' },
      ],
      currentModelId: 'default',
    };
  }

  async loadSession(sessionId: string, cwd: string) {
    return this.newSession(`${sessionId}:${cwd}`);
  }

  async setModel(_sessionId: string, model: string): Promise<void> {
    this.models.push(model);
  }

  async setMode(_sessionId: string, mode: 'agent' | 'plan' | 'ask'): Promise<void> {
    this.modes.push(mode);
  }

  prompt(
    sessionId: string,
    prompt: string,
    onUpdate: (update: CursorAcpSessionUpdate) => Promise<void>,
  ) {
    return this.run(sessionId, prompt, onUpdate);
  }

  async cancel(sessionId: string): Promise<void> {
    this.cancellations.push(sessionId);
  }

  async close(): Promise<void> {
    this.closes += 1;
  }
}

class FakeCursorCliFactory implements CursorCliFactory {
  readonly clients: FakeCursorCliClient[] = [];
  authenticated = true;
  listModelsImpl: (options: CursorCliLaunchOptions) => Promise<CursorModel[]> = async () =>
    models();

  constructor(
    private readonly run: ConstructorParameters<typeof FakeCursorCliClient>[0] = async (
      _sessionId,
      _prompt,
      onUpdate,
    ) => {
      await onUpdate(update('agent_message_chunk', 'ok'));
      return 'end_turn';
    },
  ) {}

  async create(_options: CursorCliLaunchOptions): Promise<CursorCliClient> {
    const client = new FakeCursorCliClient(this.run);
    this.clients.push(client);
    return client;
  }

  async authStatus(_options: CursorCliLaunchOptions): Promise<CursorCliAuthStatus> {
    return {
      authenticated: this.authenticated,
      status: this.authenticated ? 'authenticated' : 'unauthenticated',
      version: '2026.08.11-test',
      login_command: 'cursor-agent login',
    };
  }

  async listModels(options: CursorCliLaunchOptions): Promise<CursorModel[]> {
    return this.listModelsImpl(options);
  }

  async closeAll(): Promise<void> {}

  forceCloseAll(): void {}
}

function models(): CursorModel[] {
  return [
    { id: 'auto', display_name: 'Auto', description: '', parameters: [], variants: [] },
    {
      id: 'composer-2.5',
      display_name: 'Composer 2.5',
      description: '',
      parameters: [],
      variants: [],
    },
  ];
}

function update(type: string, text: string): CursorAcpSessionUpdate {
  return {
    sessionId: 'cursor-provider-session',
    update: { sessionUpdate: type, content: { type: 'text', text } },
  };
}

function fakeWorkspace() {
  return {
    create: async () => '/tmp/cursor-provider-test',
    remove: async () => undefined,
  };
}

function installRouter(iii: MockIII, calls: Array<Record<string, unknown>>): void {
  const originalTrigger = iii.trigger.bind(iii);
  iii.trigger = async (request: Record<string, unknown>) => {
    if (String(request.function_id).startsWith('router::')) {
      calls.push(structuredClone(request));
      if (request.function_id === 'router::provider::register') {
        return { ok: true, id: 'cursor', registration_token: 'cursor-registration-token' };
      }
      if (request.function_id === 'router::models::reconcile') {
        return {
          provider: 'cursor',
          count: ((request.payload as { models?: unknown[] }).models ?? []).length,
        };
      }
    }
    return originalTrigger(request);
  };
}
