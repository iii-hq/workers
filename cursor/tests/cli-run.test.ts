import type {
  CursorAcpSessionUpdate,
  CursorCliAuthStatus,
  CursorCliClient,
  CursorCliFactory,
  CursorCliLaunchOptions,
} from '../src/cli.js';
import { CursorWorker } from '../src/run.js';
import type { CursorModel, SessionRecord } from '../src/types.js';
import {
  FakeBridgeClient,
  FakeBridgeFactory,
  frames,
  MockIII,
  terminalFrames,
  testConfig,
} from './helpers.js';

describe('CursorWorker CLI ACP backend', () => {
  it('runs through normal login ACP without a Cursor API key or duplicate text', async () => {
    const iii = new MockIII();
    const normalized: unknown[] = [];
    const raw: unknown[] = [];
    const cli = new FakeCursorCliFactory(
      () =>
        new FakeCursorCliClient(async (_sessionId, _prompt, onUpdate) => {
          await onUpdate(acpUpdate('agent_thought_chunk', 'checking'));
          await onUpdate(acpUpdate('agent_message_chunk', 'cursor-'));
          await onUpdate(acpUpdate('agent_message_chunk', 'login-ok'));
          return 'end_turn';
        }),
    );
    const bridge = unusedBridge();
    const worker = cliWorker(iii, bridge, cli, normalized, raw);

    const response = await worker.executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2.5',
      prompt: 'reply exactly',
      session_id: 'cli-session',
    });

    expect(response).toMatchObject({
      session_id: 'cli-session',
      agent_id: 'cursor-acp-session',
      result: 'cursor-login-ok',
      status: 'FINISHED',
      stop_reason: 'end',
      usage: null,
      cost: null,
    });
    expect(response.run_id).toMatch(/^acp-/);
    expect(bridge.options).toHaveLength(0);
    expect(cli.clients[0]?.models).toEqual(['composer-2.5']);
    expect(cli.clients[0]?.modes).toEqual(['ask']);
    expect(cli.clients[0]?.newSessions).toEqual(['/repo']);
    expect(cli.clients[0]?.closes).toBe(1);
    expect(iii.state.get('cli-session')).toMatchObject({
      backend: 'cli-acp',
      agent_id: 'cursor-acp-session',
      status: 'done',
      active_run_id: null,
      turns: 1,
      usage: null,
      cost: null,
    });
    const textDeltas = normalized.flatMap((event) => {
      const candidate = event as { llm_event?: { type?: string; delta?: string } };
      return candidate.llm_event?.type === 'text_delta' ? [candidate.llm_event.delta] : [];
    });
    expect(textDeltas).toEqual(['cursor-', 'login-ok']);
    const complete = normalized.find(
      (event) => (event as { type?: string }).type === 'message_complete',
    );
    expect(JSON.stringify(complete).match(/cursor-login-ok/g)).toHaveLength(1);
    expect(raw.length).toBeGreaterThanOrEqual(4);
  });

  it('translates the public auto model alias to the ACP default model', async () => {
    const iii = new MockIII();
    const cli = new FakeCursorCliFactory();
    const worker = cliWorker(iii, unusedBridge(), cli);

    await expect(
      worker.executeRun({
        runtime: 'local',
        cwd: '/repo',
        model: 'auto',
        prompt: 'use automatic model selection',
        session_id: 'auto-model',
      }),
    ).resolves.toMatchObject({ status: 'FINISHED', stop_reason: 'end' });

    expect(cli.clients[0]?.models).toEqual(['default']);
    expect(iii.state.get('auto-model')).toMatchObject({ model: 'auto', status: 'done' });
  });

  it('rejects CLI-only model ids before dispatching an ACP prompt', async () => {
    const iii = new MockIII();
    const cli = new FakeCursorCliFactory();
    const worker = cliWorker(iii, unusedBridge(), cli);

    const response = await worker.executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'gpt-5.3-codex-low',
      prompt: 'this model only appears in the raw CLI catalog',
      session_id: 'cli-only-model',
    });

    expect(response).toMatchObject({ status: 'error', is_error: true });
    expect(response.error).toContain('not available through login-backed ACP');
    expect(response.error).toContain('cursor::models::list');
    expect(response.error).toContain('CLI-only parameterized IDs');
    expect(cli.clients[0]?.models).toEqual([]);
    expect(iii.state.get('cli-only-model')).toMatchObject({
      status: 'error',
      agent_created: false,
      active_run_id: null,
      send_idempotency_key: null,
      send_started: false,
      claim_id: null,
    });
  });

  it('recreates an unprompted ACP session after pre-dispatch configuration fails', async () => {
    const iii = new MockIII();
    let attempts = 0;
    const cli = new FakeCursorCliFactory(() => {
      const client = new FakeCursorCliClient();
      if (attempts === 0) {
        client.setMode = async () => {
          throw new Error('mode configuration failed');
        };
      }
      attempts += 1;
      return client;
    });
    const worker = cliWorker(iii, unusedBridge(), cli);
    const request = {
      runtime: 'local' as const,
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'retry safely',
      session_id: 'pre-dispatch-retry',
    };

    await expect(worker.executeRun(request)).resolves.toMatchObject({
      status: 'error',
      error: 'mode configuration failed',
    });
    expect(iii.state.get(request.session_id)).toMatchObject({
      agent_created: false,
      send_started: false,
      active_run_id: null,
    });

    await expect(worker.executeRun(request)).resolves.toMatchObject({
      status: 'FINISHED',
      stop_reason: 'end',
    });
    expect(cli.clients).toHaveLength(2);
    expect(cli.clients[0]?.newSessions).toEqual(['/repo']);
    expect(cli.clients[1]?.newSessions).toEqual(['/repo']);
    expect(cli.clients[1]?.loadedSessions).toEqual([]);
  });

  it('recreates a new ACP session after an immediate stop before prompt dispatch', async () => {
    const iii = new MockIII();
    const cli = new FakeCursorCliFactory();
    const worker = cliWorker(iii, unusedBridge(), cli);
    worker.register();
    const request = {
      runtime: 'local' as const,
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'retry after cancellation',
      session_id: 'pre-dispatch-stop',
    };

    const firstRun = worker.executeRun(request);
    await expect(
      iii.functions.get('cursor::stop')?.handler({ session_id: request.session_id }),
    ).resolves.toEqual({ session_id: request.session_id, stopped: true, reason: null });
    await expect(firstRun).resolves.toMatchObject({
      agent_id: null,
      run_id: null,
      status: 'CANCELLED',
      stop_reason: 'aborted',
    });
    expect(iii.state.get(request.session_id)).toMatchObject({
      status: 'cancelled',
      agent_created: false,
      active_run_id: null,
      send_started: false,
    });

    await expect(worker.executeRun(request)).resolves.toMatchObject({
      status: 'FINISHED',
      stop_reason: 'end',
    });
    expect(cli.clients).toHaveLength(2);
    expect(cli.clients[0]?.newSessions).toEqual(['/repo']);
    expect(cli.clients[1]?.newSessions).toEqual(['/repo']);
    expect(cli.clients[1]?.loadedSessions).toEqual([]);
  });

  it.each([
    ['max_tokens', 'MAX_TOKENS', 'model output limit'],
    ['max_turn_requests', 'MAX_TURN_REQUESTS', 'turn-request limit'],
  ] as const)('maps %s to a length stop while persisting an error terminal state', async (reason, status, errorText) => {
    const iii = new MockIII();
    const normalized: unknown[] = [];
    const cli = new FakeCursorCliFactory(() => new FakeCursorCliClient(async () => reason));
    const worker = cliWorker(iii, unusedBridge(), cli, normalized);

    const response = await worker.executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'produce a long answer',
      session_id: `limit-${reason}`,
    });

    expect(response).toMatchObject({
      status,
      stop_reason: 'length',
      is_error: true,
    });
    expect(response.error).toContain(errorText);
    expect(iii.state.get(`limit-${reason}`)).toMatchObject({ status: 'error' });
    expect(
      normalized.find((event) => (event as { type?: string }).type === 'message_complete'),
    ).toMatchObject({ message: { stop_reason: 'length' } });
  });

  it('continues legacy local Bridge sessions after CLI ACP becomes the default', async () => {
    const iii = new MockIII();
    const legacy = completedLocalRecord('legacy-local');
    delete legacy.backend;
    iii.state.set(legacy.session_id, legacy);
    const cli = new FakeCursorCliFactory();
    const client = new FakeBridgeClient(
      (call) => {
        if (call.method === 'ResumeAgent') return { agentId: call.request.agentId };
        if (call.method === 'GetRun') {
          return {
            run: {
              runId: call.request.runId,
              agentId: legacy.agent_id,
              status: 'RUN_LIFECYCLE_STATUS_RUNNING',
            },
          };
        }
        throw new Error(`unexpected Bridge call ${call.method}`);
      },
      () => frames(...terminalFrames('legacy-follow-up', 'legacy-bridge-ok')),
    );
    const bridge = new FakeBridgeFactory(client);
    const response = await cliWorker(iii, bridge, cli, [], [], {
      api_key: 'key_bridge',
    }).executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'resume',
      session_id: legacy.session_id,
    });

    expect(response).toMatchObject({
      status: 'FINISHED',
      result: 'legacy-bridge-ok',
      stop_reason: 'end',
    });
    expect(cli.createCalls).toBe(0);
    expect(bridge.options).toHaveLength(1);
    expect(client.calls.some((call) => call.method === 'ResumeAgent')).toBe(true);
  });

  it('rejects explicit tool configuration before launching Cursor ACP', async () => {
    const iii = new MockIII();
    const cli = new FakeCursorCliFactory();
    const response = await cliWorker(iii, unusedBridge(), cli).executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'read',
      tools: [],
      session_id: 'explicit-tools',
    });

    expect(response.error).toContain('does not support explicit tool lists');
    expect(cli.createCalls).toBe(0);
    expect(iii.state.has('explicit-tools')).toBe(false);
  });

  it('refreshes the durable claim while a quiet ACP prompt is running', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-08-21T00:00:00Z'));
    try {
      const iii = new MockIII();
      let markPromptStarted: () => void = () => undefined;
      const promptStarted = new Promise<void>((resolvePromise) => {
        markPromptStarted = resolvePromise;
      });
      let finishPrompt: (reason: 'end_turn') => void = () => undefined;
      const client = new FakeCursorCliClient(async () => {
        markPromptStarted();
        return new Promise<'end_turn'>((resolvePromise) => {
          finishPrompt = resolvePromise;
        });
      });
      const worker = cliWorker(iii, unusedBridge(), new FakeCursorCliFactory(() => client));
      const run = worker.executeRun({
        runtime: 'local',
        cwd: '/repo',
        model: 'composer-2',
        prompt: 'wait quietly',
        session_id: 'quiet-cli',
      });
      await promptStarted;
      const before = iii.state.get('quiet-cli') as SessionRecord;

      await vi.advanceTimersByTimeAsync(30_000);

      const after = iii.state.get('quiet-cli') as SessionRecord;
      expect(after.claim_started_at_ms).toBeGreaterThan(before.claim_started_at_ms ?? 0);
      expect(after.status).toBe('working');
      finishPrompt('end_turn');
      await expect(run).resolves.toMatchObject({ status: 'FINISHED' });
    } finally {
      vi.useRealTimers();
    }
  });

  it('marks synthetic active ACP runs recovery-required after process loss', async () => {
    const iii = new MockIII();
    const cli = new FakeCursorCliFactory(
      () =>
        new FakeCursorCliClient(async (_sessionId, _prompt, onUpdate) => {
          await onUpdate(acpUpdate('agent_message_chunk', 'partial'));
          throw new Error('ACP process exited');
        }),
    );
    const worker = cliWorker(iii, unusedBridge(), cli);
    const request = {
      runtime: 'local' as const,
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'run once',
      session_id: 'lost-cli',
    };

    const first = await worker.executeRun(request);
    const persisted = iii.state.get('lost-cli') as SessionRecord;
    const second = await worker.executeRun(request);

    expect(first).toMatchObject({ status: 'recovery-required', recovery_required: true });
    expect(persisted.status).toBe('recovery-required');
    expect(persisted.active_run_id).toMatch(/^acp-/);
    expect(persisted.claim_id).toBeNull();
    expect(second).toMatchObject({ status: 'recovery-required', recovery_required: true });
    expect(second.error).toContain('ACP session lost its process after prompt dispatch');
    expect(second.error).not.toContain('run id');
    expect(cli.createCalls).toBe(1);
  });

  it('persists recovery-required before close returns for an active CLI prompt', async () => {
    const iii = new MockIII();
    let markPromptStarted: () => void = () => undefined;
    const promptStarted = new Promise<void>((resolvePromise) => {
      markPromptStarted = resolvePromise;
    });
    let finishPrompt: (reason: 'end_turn') => void = () => undefined;
    const client = new FakeCursorCliClient(async () => {
      markPromptStarted();
      return new Promise<'end_turn'>((resolvePromise) => {
        finishPrompt = resolvePromise;
      });
    });
    const worker = cliWorker(iii, unusedBridge(), new FakeCursorCliFactory(() => client));
    const run = worker.executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'stay active',
      session_id: 'shutdown-cli',
    });
    await promptStarted;

    await worker.close();

    expect(iii.state.get('shutdown-cli')).toMatchObject({
      status: 'recovery-required',
      claim_id: null,
      claim_started_at_ms: null,
      active_run_id: expect.stringMatching(/^acp-/),
    });
    finishPrompt('end_turn');
    await expect(run).resolves.toMatchObject({
      status: 'recovery-required',
      recovery_required: true,
    });
  });

  it('cancels a stale CLI claim that never dispatched a prompt', async () => {
    const iii = new MockIII();
    const record = completedLocalRecord('stale-unsent-cli');
    record.backend = 'cli-acp';
    record.status = 'working';
    record.active_turn = 2;
    record.active_run_id = null;
    record.send_idempotency_key = 'send-unsent';
    record.send_started = false;
    record.claim_id = 'worker-gone';
    record.claim_started_at_ms = Date.now() - 600_000;
    record.pending_prompt_sha256 = 'prompt-hash';
    iii.state.set(record.session_id, record);
    const worker = cliWorker(iii, unusedBridge(), new FakeCursorCliFactory());
    worker.register();

    await expect(
      iii.functions.get('cursor::stop')?.handler({ session_id: record.session_id }),
    ).resolves.toEqual({ session_id: record.session_id, stopped: true, reason: null });
    expect(iii.state.get(record.session_id)).toMatchObject({
      status: 'cancelled',
      active_turn: null,
      active_run_id: null,
      send_idempotency_key: null,
      send_started: false,
      claim_id: null,
      claim_started_at_ms: null,
      pending_prompt_sha256: null,
    });
  });

  it('does not overwrite terminal state when detached stop races prompt completion', async () => {
    const iii = new MockIII();
    const record = completedLocalRecord('stop-race-cli');
    record.backend = 'cli-acp';
    record.status = 'working';
    record.active_turn = 2;
    record.active_run_id = 'acp-active';
    record.send_idempotency_key = 'send-active';
    record.send_started = true;
    record.claim_id = 'worker-gone';
    record.claim_started_at_ms = Date.now() - 600_000;
    record.pending_prompt_sha256 = 'prompt-hash';
    iii.state.set(record.session_id, record);
    const terminal: SessionRecord = {
      ...record,
      status: 'done',
      active_turn: null,
      active_run_id: null,
      last_run_id: 'acp-active',
      send_idempotency_key: null,
      send_started: false,
      claim_id: null,
      claim_started_at_ms: null,
      pending_prompt_sha256: null,
      updated_at_ms: record.updated_at_ms + 1,
    };
    const originalTrigger = iii.trigger.bind(iii);
    let reads = 0;
    iii.trigger = async (request: Record<string, unknown>) => {
      if (
        request.function_id === 'state::get' &&
        (request.payload as { key?: string }).key === record.session_id
      ) {
        reads += 1;
        if (reads === 2) iii.state.set(record.session_id, terminal);
      }
      return originalTrigger(request);
    };
    const worker = cliWorker(iii, unusedBridge(), new FakeCursorCliFactory());
    worker.register();

    await expect(
      iii.functions.get('cursor::stop')?.handler({ session_id: record.session_id }),
    ).resolves.toEqual({
      session_id: record.session_id,
      stopped: false,
      reason: 'Cursor session changed before its active prompt was marked for recovery',
    });
    expect(iii.state.get(record.session_id)).toEqual(terminal);
  });

  it('routes live stop to session/cancel and persists cancellation', async () => {
    const iii = new MockIII();
    let markPromptStarted: () => void = () => undefined;
    const promptStarted = new Promise<void>((resolvePromise) => {
      markPromptStarted = resolvePromise;
    });
    let finishPrompt: (reason: 'cancelled') => void = () => undefined;
    const client = new FakeCursorCliClient(async () => {
      markPromptStarted();
      return new Promise<'cancelled'>((resolvePromise) => {
        finishPrompt = resolvePromise;
      });
    });
    client.onCancel = () => finishPrompt('cancelled');
    const cli = new FakeCursorCliFactory(() => client);
    const worker = cliWorker(iii, unusedBridge(), cli);
    worker.register();

    const started = await iii.functions.get('cursor::start')?.handler({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'wait',
      session_id: 'cancel-cli',
    });
    await promptStarted;
    const stopped = await iii.functions.get('cursor::stop')?.handler({
      session_id: 'cancel-cli',
    });
    await vi.waitFor(() => {
      expect((iii.state.get('cancel-cli') as SessionRecord).status).toBe('cancelled');
    });

    expect(started).toEqual({ session_id: 'cancel-cli', started: true });
    expect(stopped).toEqual({ session_id: 'cancel-cli', stopped: true, reason: null });
    expect(client.cancellations).toEqual(['cursor-acp-session']);
  });

  it('honors ACP timeout_ms by cancelling the live Cursor prompt', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-08-21T00:00:00Z'));
    try {
      const iii = new MockIII();
      let markPromptStarted: () => void = () => undefined;
      const promptStarted = new Promise<void>((resolvePromise) => {
        markPromptStarted = resolvePromise;
      });
      let finishPrompt: (reason: 'cancelled') => void = () => undefined;
      const client = new FakeCursorCliClient(async () => {
        markPromptStarted();
        return new Promise<'cancelled'>((resolvePromise) => {
          finishPrompt = resolvePromise;
        });
      });
      client.onCancel = () => finishPrompt('cancelled');
      const worker = cliWorker(iii, unusedBridge(), new FakeCursorCliFactory(() => client));
      worker.register();

      const run = Promise.resolve(
        iii.functions.get('run::start_and_wait')?.handler({
          cwd: '/repo',
          model: 'composer-2',
          provider: 'cursor',
          prompt: 'wait until timeout',
          timeout_ms: 1_000,
        }),
      );
      await promptStarted;

      await vi.advanceTimersByTimeAsync(1_000);

      await expect(run).resolves.toMatchObject({ status: 'CANCELLED', stop_reason: 'aborted' });
      expect(client.cancellations).toEqual(['cursor-acp-session']);
    } finally {
      vi.useRealTimers();
    }
  });

  it('delivers cross-worker CLI cancellation through durable state exactly once', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-08-21T00:00:00Z'));
    try {
      const iii = new MockIII();
      let markPromptStarted: () => void = () => undefined;
      const promptStarted = new Promise<void>((resolvePromise) => {
        markPromptStarted = resolvePromise;
      });
      let finishPrompt: (reason: 'cancelled') => void = () => undefined;
      const client = new FakeCursorCliClient(async () => {
        markPromptStarted();
        return new Promise<'cancelled'>((resolvePromise) => {
          finishPrompt = resolvePromise;
        });
      });
      client.onCancel = () => finishPrompt('cancelled');
      const owner = cliWorker(iii, unusedBridge(), new FakeCursorCliFactory(() => client));
      const nonOwner = cliWorker(iii, unusedBridge(), new FakeCursorCliFactory());
      nonOwner.register();
      const run = owner.executeRun({
        runtime: 'local',
        cwd: '/repo',
        model: 'composer-2',
        prompt: 'wait for cancellation',
        session_id: 'cross-worker-cancel',
      });
      await promptStarted;
      const stop = () =>
        iii.functions.get('cursor::stop')?.handler({ session_id: 'cross-worker-cancel' });

      await expect(stop()).resolves.toEqual({
        session_id: 'cross-worker-cancel',
        stopped: true,
        reason: null,
      });
      await expect(stop()).resolves.toMatchObject({ stopped: true });
      expect(iii.state.get('cross-worker-cancel')).toMatchObject({ cancel_requested: true });

      await vi.advanceTimersByTimeAsync(30_000);

      await expect(run).resolves.toMatchObject({ status: 'CANCELLED', stop_reason: 'aborted' });
      expect(client.cancellations).toEqual(['cursor-acp-session']);
      expect(iii.state.get('cross-worker-cancel')).toMatchObject({
        status: 'cancelled',
        cancel_requested: false,
        active_run_id: null,
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not persist cancellation onto a replacement CLI prompt', async () => {
    const iii = new MockIII();
    const record = completedLocalRecord('cancel-replacement-race');
    Object.assign(record, {
      backend: 'cli-acp',
      status: 'working',
      active_turn: 2,
      active_run_id: 'acp-original',
      send_idempotency_key: 'send-original',
      send_started: true,
      claim_id: 'worker-original',
      claim_started_at_ms: Date.now(),
      pending_prompt_sha256: 'prompt-original',
    });
    iii.state.set(record.session_id, record);
    const replacement: SessionRecord = {
      ...record,
      active_run_id: 'acp-replacement',
      send_idempotency_key: 'send-replacement',
      claim_id: 'worker-replacement',
      claim_started_at_ms: Date.now(),
      pending_prompt_sha256: 'prompt-replacement',
      cancel_requested: false,
      updated_at_ms: record.updated_at_ms + 1,
    };
    const originalTrigger = iii.trigger.bind(iii);
    let reads = 0;
    iii.trigger = async (request: Record<string, unknown>) => {
      if (
        request.function_id === 'state::get' &&
        (request.payload as { key?: string }).key === record.session_id
      ) {
        reads += 1;
        if (reads === 2) iii.state.set(record.session_id, replacement);
      }
      return originalTrigger(request);
    };
    const nonOwner = cliWorker(iii, unusedBridge(), new FakeCursorCliFactory());
    nonOwner.register();

    await expect(
      iii.functions.get('cursor::stop')?.handler({ session_id: record.session_id }),
    ).resolves.toEqual({
      session_id: record.session_id,
      stopped: false,
      reason: 'Cursor session changed before its active prompt was asked to stop',
    });
    expect(iii.state.get(record.session_id)).toEqual(replacement);
  });

  it('does not use a stale live handle to cancel a replacement CLI prompt', async () => {
    const iii = new MockIII();
    let markCloseStarted: () => void = () => undefined;
    const closeStarted = new Promise<void>((resolvePromise) => {
      markCloseStarted = resolvePromise;
    });
    let finishClose: () => void = () => undefined;
    const closeGate = new Promise<void>((resolvePromise) => {
      finishClose = resolvePromise;
    });
    const client = new FakeCursorCliClient();
    client.close = async () => {
      client.closes += 1;
      markCloseStarted();
      await closeGate;
    };
    const worker = cliWorker(iii, unusedBridge(), new FakeCursorCliFactory(() => client));
    worker.register();
    const sessionId = 'stale-live-cancel';
    const run = worker.executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'finish before replacement',
      session_id: sessionId,
    });
    await closeStarted;
    const terminal = iii.state.get(sessionId) as SessionRecord;
    const replacement: SessionRecord = {
      ...terminal,
      status: 'working',
      active_turn: terminal.turns + 1,
      active_run_id: 'acp-replacement',
      send_idempotency_key: 'send-replacement',
      send_started: true,
      cancel_requested: false,
      claim_id: 'worker-replacement',
      claim_started_at_ms: Date.now(),
      pending_prompt_sha256: 'prompt-replacement',
      updated_at_ms: terminal.updated_at_ms + 1,
    };
    iii.state.set(sessionId, replacement);

    await expect(
      iii.functions.get('cursor::stop')?.handler({ session_id: sessionId }),
    ).resolves.toEqual({ session_id: sessionId, stopped: true, reason: null });
    expect(client.cancellations).toEqual([]);
    expect(iii.state.get(sessionId)).toMatchObject({
      active_run_id: 'acp-replacement',
      claim_id: 'worker-replacement',
      cancel_requested: true,
    });

    finishClose();
    await expect(run).resolves.toMatchObject({ status: 'FINISHED' });
  });

  it('exposes redacted auth, dynamic models, and no fabricated CLI run snapshot', async () => {
    const iii = new MockIII();
    iii.state.set('cli-status', {
      ...completedLocalRecord('cli-status'),
      backend: 'cli-acp',
      agent_id: 'cursor-acp-session',
    });
    const cli = new FakeCursorCliFactory();
    const bridgeClient = new FakeBridgeClient(
      (call) => {
        if (call.method === 'ListModels') {
          return { items: [{ id: 'bridge-cloud', displayName: 'Bridge Cloud' }] };
        }
        throw new Error(`unexpected Bridge call ${call.method}`);
      },
      () => frames(),
    );
    const bridge = new FakeBridgeFactory(bridgeClient);
    const worker = cliWorker(iii, bridge, cli, [], [], { api_key: 'key_bridge' });
    worker.register();

    await expect(iii.functions.get('cursor::auth::status')?.handler({})).resolves.toEqual({
      available: true,
      authenticated: true,
      status: 'authenticated',
      version: '2026.08.11-e8db854',
      login_command: 'cursor-agent login',
      error: null,
    });
    await expect(iii.functions.get('cursor::models::list')?.handler({})).resolves.toEqual({
      models: [model('composer-2.5', 'Composer 2.5')],
    });
    await expect(
      iii.functions.get('cursor::models::list')?.handler({ backend: 'sdk-bridge' }),
    ).resolves.toEqual({ models: [model('bridge-cloud', 'Bridge Cloud')] });
    expect(bridgeClient.calls.filter((call) => call.method === 'ListModels')).toHaveLength(1);
    await expect(
      iii.functions.get('cursor::status')?.handler({ session_id: 'cli-status' }),
    ).resolves.toMatchObject({
      record: { last_run_id: 'run-old' },
      run: null,
      agent: { metadata: { backend: 'cli-acp' } },
    });
  });
});

class FakeCursorCliClient implements CursorCliClient {
  readonly newSessions: string[] = [];
  readonly loadedSessions: Array<{ sessionId: string; cwd: string }> = [];
  readonly models: string[] = [];
  readonly modes: string[] = [];
  readonly cancellations: string[] = [];
  closes = 0;
  onCancel: (() => void) | null = null;

  constructor(
    private readonly promptImpl: (
      sessionId: string,
      prompt: string,
      onUpdate: (update: CursorAcpSessionUpdate) => Promise<void>,
    ) => Promise<
      'end_turn' | 'max_tokens' | 'max_turn_requests' | 'refusal' | 'cancelled'
    > = async () => 'end_turn',
  ) {}

  async newSession(cwd: string) {
    this.newSessions.push(cwd);
    return {
      sessionId: 'cursor-acp-session',
      models: availableAcpModels(),
      currentModelId: 'default',
    };
  }

  async loadSession(sessionId: string, cwd: string) {
    this.loadedSessions.push({ sessionId, cwd });
    return { sessionId, models: availableAcpModels(), currentModelId: 'default' };
  }

  async setModel(_sessionId: string, modelId: string): Promise<void> {
    this.models.push(modelId);
  }

  async setMode(_sessionId: string, mode: 'agent' | 'plan' | 'ask'): Promise<void> {
    this.modes.push(mode);
  }

  prompt(
    sessionId: string,
    prompt: string,
    onUpdate: (update: CursorAcpSessionUpdate) => Promise<void>,
  ) {
    return this.promptImpl(sessionId, prompt, onUpdate);
  }

  async cancel(sessionId: string): Promise<void> {
    this.cancellations.push(sessionId);
    this.onCancel?.();
  }

  async close(): Promise<void> {
    this.closes += 1;
  }
}

class FakeCursorCliFactory implements CursorCliFactory {
  readonly clients: FakeCursorCliClient[] = [];
  createCalls = 0;

  constructor(
    private readonly makeClient: () => FakeCursorCliClient = () => new FakeCursorCliClient(),
  ) {}

  async create(_options: CursorCliLaunchOptions): Promise<CursorCliClient> {
    this.createCalls += 1;
    const client = this.makeClient();
    this.clients.push(client);
    return client;
  }

  async authStatus(_options: CursorCliLaunchOptions): Promise<CursorCliAuthStatus> {
    return {
      authenticated: true,
      status: 'authenticated',
      version: '2026.08.11-e8db854',
      login_command: 'cursor-agent login',
    };
  }

  async listModels(_options: CursorCliLaunchOptions): Promise<CursorModel[]> {
    return [model('composer-2.5', 'Composer 2.5')];
  }

  async closeAll(): Promise<void> {}
  forceCloseAll(): void {}
}

function cliWorker(
  iii: MockIII,
  bridge: FakeBridgeFactory,
  cli: FakeCursorCliFactory,
  normalized: unknown[] = [],
  raw: unknown[] = [],
  configOverrides: Parameters<typeof testConfig>[0] = {},
): CursorWorker {
  return new CursorWorker(
    iii.asClient(),
    () =>
      testConfig({
        local_backend: 'cli-acp',
        api_key: ' ',
        ...configOverrides,
      }),
    async (_group, event) => normalized.push(structuredClone(event)),
    async (_group, event) => raw.push(structuredClone(event)),
    bridge,
    cli,
  );
}

function unusedBridge(): FakeBridgeFactory {
  return new FakeBridgeFactory(
    new FakeBridgeClient(
      () => {
        throw new Error('Bridge must not be used');
      },
      () => frames(),
    ),
  );
}

function acpUpdate(sessionUpdate: string, text: string): CursorAcpSessionUpdate {
  return {
    sessionId: 'cursor-acp-session',
    update: { sessionUpdate, content: { type: 'text', text } },
  };
}

function model(id: string, displayName: string): CursorModel {
  return {
    id,
    display_name: displayName,
    description: '',
    parameters: [],
    variants: [],
  };
}

function availableAcpModels(): Array<{ modelId: string; name: string }> {
  return [
    { modelId: 'default', name: 'Auto' },
    { modelId: 'composer-2', name: 'Composer 2' },
    { modelId: 'composer-2.5', name: 'Composer 2.5' },
  ];
}

function completedLocalRecord(sessionId: string): SessionRecord {
  return {
    session_id: sessionId,
    agent_id: 'legacy-bridge-agent',
    runtime: 'local',
    workspace: '/repo',
    name: null,
    model: 'composer-2',
    tools: ['read', 'grep', 'glob', 'ls'],
    repositories: [],
    work_on_current_branch: false,
    auto_create_pr: false,
    status: 'done',
    agent_created: true,
    turns: 1,
    active_turn: null,
    active_run_id: null,
    last_run_id: 'run-old',
    create_idempotency_key: 'create-old',
    send_idempotency_key: null,
    send_started: false,
    cancel_requested: false,
    claim_id: null,
    claim_started_at_ms: null,
    pending_prompt_sha256: null,
    usage: null,
    cost: null,
    updated_at_ms: 1,
  };
}
