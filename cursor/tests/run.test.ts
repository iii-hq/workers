import { createHash } from 'node:crypto';
import { BridgeRpcError, BridgeTransportError } from '../src/bridge.js';
import { makeEmitter } from '../src/events.js';
import { CursorWorker, extractPrompt, RunPayloadSchema } from '../src/run.js';
import type { RunStreamMessageWire, SessionRecord } from '../src/types.js';
import {
  clone,
  FakeBridgeClient,
  FakeBridgeFactory,
  frames,
  MockIII,
  terminalFrames,
  testConfig,
} from './helpers.js';

describe('CursorWorker run lifecycle', () => {
  it('runs a sandboxed local agent with a durable mapping and normalized events', async () => {
    const iii = new MockIII();
    const client = successfulClient();
    const factory = new FakeBridgeFactory(client);
    const normalized: unknown[] = [];
    const raw: unknown[] = [];
    const worker = makeWorker(iii, factory, normalized, raw);

    const response = await worker.executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'inspect',
      session_id: 'session-one',
    });

    expect(response).toMatchObject({
      session_id: 'session-one',
      run_id: 'run-one',
      result: 'done',
      status: 'FINISHED',
      stop_reason: 'end',
      is_error: false,
    });
    const create = client.calls.find((call) => call.method === 'CreateAgent');
    expect(create?.request).toMatchObject({
      options: {
        model: { id: 'composer-2' },
        apiKey: 'key_test_secret',
        mode: 'AGENT_MODE_OPTION_AGENT',
        tools: { names: ['read', 'grep', 'glob', 'ls'] },
        local: { cwd: ['/repo'], sandboxOptions: { enabled: true } },
      },
    });
    expect((create?.request.options as { agentId: string }).agentId).toMatch(/^agent-/);
    expect(JSON.stringify(create?.request)).not.toContain('autoReview');
    expect(create?.request).not.toHaveProperty('idempotencyKey');
    const send = client.calls.find((call) => call.method === 'Send');
    expect(send?.request).not.toHaveProperty('idempotencyKey');
    const record = iii.state.get('session-one') as SessionRecord;
    expect(record).toMatchObject({
      status: 'done',
      active_run_id: null,
      last_run_id: 'run-one',
      send_idempotency_key: null,
      turns: 1,
    });
    expect(raw).toHaveLength(3);
    expect(
      normalized.filter((event) => (event as { type?: string }).type === 'message_complete'),
    ).toHaveLength(1);
    expect(
      normalized.find((event) => (event as { type?: string }).type === 'message_complete'),
    ).toMatchObject({ body_streamed: true });
    expect(factory.options[0]?.workspace).toBe('/repo');
    expect(client.closes).toBe(1);
  });

  it('reuses one cloud Send idempotency key across a retry', async () => {
    const iii = new MockIII();
    let streamAttempt = 0;
    const client = new FakeBridgeClient(unaryResponse, () => {
      streamAttempt += 1;
      if (streamAttempt === 1) {
        return throwingStream(new BridgeTransportError('connection reset'));
      }
      return frames(...terminalFrames('run-cloud'));
    });
    const factory = new FakeBridgeFactory(client);
    const worker = makeWorker(iii, factory);

    const response = await worker.executeRun({
      runtime: 'cloud',
      repositories: [{ url: 'https://github.com/acme/repo' }],
      prompt: 'fix tests',
      session_id: 'cloud-one',
    });

    const sends = client.calls.filter((call) => call.method === 'Send');
    expect(sends).toHaveLength(2);
    expect(sends[0]?.request.idempotencyKey).toBe(sends[1]?.request.idempotencyKey);
    expect(String(sends[0]?.request.idempotencyKey)).toMatch(/^iii-cursor-send-/);
    const create = client.calls.find((call) => call.method === 'CreateAgent');
    expect(create?.request).toMatchObject({
      idempotencyKey: expect.stringMatching(/^iii-cursor-create-/),
      options: {
        cloud: {
          workOnCurrentBranch: false,
          autoCreatePr: false,
        },
      },
    });
    expect((create?.request.options as { agentId: string }).agentId).toMatch(/^bc-/);
    expect(response.status).toBe('FINISHED');
    expect(response.usage).toBeNull();
    expect(response.cost).toBeNull();
  });

  it('namespaces cloud idempotency keys per durable state domain', async () => {
    const request = {
      runtime: 'cloud' as const,
      repositories: [{ url: 'https://github.com/acme/repo' }],
      prompt: 'fix tests',
      session_id: 'shared-caller-session',
    };
    const firstClient = successfulClient('run-first');
    const secondClient = successfulClient('run-second');

    await makeWorker(new MockIII(), new FakeBridgeFactory(firstClient)).executeRun(request);
    await makeWorker(new MockIII(), new FakeBridgeFactory(secondClient)).executeRun(request);

    const firstCreate = firstClient.calls.find((call) => call.method === 'CreateAgent');
    const secondCreate = secondClient.calls.find((call) => call.method === 'CreateAgent');
    const firstSend = firstClient.calls.find((call) => call.method === 'Send');
    const secondSend = secondClient.calls.find((call) => call.method === 'Send');
    expect(firstCreate?.request.idempotencyKey).not.toBe(secondCreate?.request.idempotencyKey);
    expect(firstSend?.request.idempotencyKey).not.toBe(secondSend?.request.idempotencyKey);
  });

  it('persists the terminal model selected by Cursor for a cloud run', async () => {
    const iii = new MockIII();
    const client = new FakeBridgeClient(unaryResponse, (call) => {
      if (call.method === 'Send') {
        return frames({
          sdkMessage: { type: 'system', message: { run_id: 'run-model' } },
        });
      }
      return frames(
        {
          result: {
            agentId: 'agent',
            runId: 'run-model',
            status: 'RUN_LIFECYCLE_STATUS_FINISHED',
            result: {
              agentId: 'agent',
              runId: 'run-model',
              status: 'RUN_LIFECYCLE_STATUS_FINISHED',
              result: 'done',
              model: { id: 'cursor-selected-model' },
            },
          },
        },
        { done: { agentId: 'agent', runId: 'run-model' } },
      );
    });
    const events: unknown[] = [];

    const response = await makeWorker(iii, new FakeBridgeFactory(client), events).executeRun({
      runtime: 'cloud',
      repositories: [{ url: 'https://github.com/acme/repo' }],
      prompt: 'use the default model',
      session_id: 'cloud-default-model',
    });

    const complete = events.find(
      (event) => (event as { type?: string }).type === 'message_complete',
    ) as { message: { model: string } };
    expect(response.status).toBe('FINISHED');
    expect(complete.message.model).toBe('cursor-selected-model');
    expect((iii.state.get('cloud-default-model') as SessionRecord).model).toBe(
      'cursor-selected-model',
    );
  });

  it('allows only one local Send when two worker processes race the same new session', async () => {
    const iii = new MockIII();
    const originalTrigger = iii.trigger.bind(iii);
    let initialGets = 0;
    let releaseGets: () => void = () => undefined;
    const bothLoaded = new Promise<void>((resolvePromise) => {
      releaseGets = resolvePromise;
    });
    iii.trigger = async (request: Record<string, unknown>) => {
      if (request.function_id === 'state::get' && initialGets < 2) {
        initialGets += 1;
        if (initialGets === 2) releaseGets();
        await bothLoaded;
      }
      return originalTrigger(request);
    };
    const firstClient = successfulClient('run-first');
    const secondClient = successfulClient('run-second');
    const request = {
      runtime: 'local' as const,
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'mutate once',
      session_id: 'contended-local',
    };

    const responses = await Promise.all([
      makeWorker(iii, new FakeBridgeFactory(firstClient)).executeRun(request),
      makeWorker(iii, new FakeBridgeFactory(secondClient)).executeRun(request),
    ]);

    expect(
      [...firstClient.calls, ...secondClient.calls].filter((call) => call.method === 'Send'),
    ).toHaveLength(1);
    expect(responses.filter((response) => response.busy)).toHaveLength(1);
    expect((iii.state.get('contended-local') as SessionRecord).turns).toBe(1);
  });

  it('recovers an initial durable claim after a crash before agent creation', async () => {
    const iii = new MockIII();
    const originalTrigger = iii.trigger.bind(iii);
    let crashInjected = false;
    let failCrashLoad = false;
    iii.trigger = async (request: Record<string, unknown>) => {
      if (request.function_id === 'state::compare-and-set' && !crashInjected) {
        await originalTrigger(request);
        crashInjected = true;
        failCrashLoad = true;
        throw new Error('process exited after persisting the initial claim');
      }
      if (request.function_id === 'state::get' && failCrashLoad) {
        failCrashLoad = false;
        throw new Error('process is gone');
      }
      return originalTrigger(request);
    };
    const client = successfulClient('run-recovered');
    const factory = new FakeBridgeFactory(client);
    const request = {
      runtime: 'local' as const,
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'recover the first turn',
      session_id: 'initial-claim-crash',
    };

    const first = await makeWorker(iii, factory).executeRun(request);
    const orphan = iii.state.get('initial-claim-crash') as SessionRecord;
    const recovered = await makeWorker(iii, factory).executeRun(request);

    expect(first.is_error).toBe(true);
    expect(orphan).toMatchObject({
      status: 'working',
      active_turn: 1,
      active_run_id: null,
      send_started: false,
      pending_prompt_sha256: createHash('sha256').update(request.prompt).digest('hex'),
    });
    expect(orphan.send_idempotency_key).toMatch(/^iii-cursor-send-/);
    expect(orphan.claim_id).not.toBeNull();
    expect(recovered.status).toBe('FINISHED');
    expect(client.calls.filter((call) => call.method === 'Send')).toHaveLength(1);
  });

  it('honors retry metadata and exposes the full request id on a failed cloud Send', async () => {
    const iii = new MockIII();
    const error = new BridgeRpcError(
      'resource_exhausted',
      'rate limited',
      {
        sdk_error_code: 'RATE_LIMIT_EXCEEDED',
        request_id: 'request-id-from-cursor',
        retry_after_seconds: 0,
      },
      [],
    );
    const client = new FakeBridgeClient(unaryResponse, () => throwingStream(error));
    const response = await makeWorker(iii, new FakeBridgeFactory(client)).executeRun({
      runtime: 'cloud',
      repositories: [{ url: 'https://github.com/acme/repo' }],
      prompt: 'fix tests',
      session_id: 'cloud-rate-limit',
    });

    expect(client.calls.filter((call) => call.method === 'Send')).toHaveLength(2);
    expect(response.error).toContain('request-id-from-cursor');
    expect(response.error_details).toMatchObject({
      transport_code: 'resource_exhausted',
      sdk_error_code: 'RATE_LIMIT_EXCEEDED',
      request_id: 'request-id-from-cursor',
      retry_after_seconds: 0,
    });
  });

  it('refreshes the durable claim during a long cloud retry delay', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(600_000);
    try {
      const iii = new MockIII();
      let sendAttempt = 0;
      const client = new FakeBridgeClient(unaryResponse, (call) => {
        if (call.method === 'Send') {
          sendAttempt += 1;
          if (sendAttempt === 1) {
            return throwingStream(
              new BridgeRpcError(
                'resource_exhausted',
                'rate limited',
                {
                  sdk_error_code: 'RATE_LIMIT_EXCEEDED',
                  retry_after_seconds: 301,
                },
                [],
              ),
            );
          }
        }
        return frames(...terminalFrames('run-retried'));
      });
      const run = makeWorker(iii, new FakeBridgeFactory(client)).executeRun({
        runtime: 'cloud',
        repositories: [{ url: 'https://github.com/acme/repo' }],
        prompt: 'retry later',
        session_id: 'long-retry',
      });
      for (let attempt = 0; attempt < 20; attempt += 1) {
        if (client.calls.some((call) => call.method === 'Send')) break;
        await Promise.resolve();
      }
      const claimedAt = (iii.state.get('long-retry') as SessionRecord).claim_started_at_ms;

      await vi.advanceTimersByTimeAsync(30_000);

      expect((iii.state.get('long-retry') as SessionRecord).claim_started_at_ms).toBeGreaterThan(
        claimedAt ?? 0,
      );
      await vi.advanceTimersByTimeAsync(271_000);
      expect((await run).status).toBe('FINISHED');
    } finally {
      vi.useRealTimers();
    }
  });

  it('persists the full cloud Create identity across an ambiguous failure', async () => {
    const iii = new MockIII();
    let creates = 0;
    const client = new FakeBridgeClient(
      (call) => {
        if (call.method === 'CreateAgent') {
          creates += 1;
          if (creates === 1) throw new BridgeTransportError('response lost');
        }
        return unaryResponse(call);
      },
      () => frames(...terminalFrames('run-created')),
    );
    const worker = makeWorker(iii, new FakeBridgeFactory(client));
    const original = {
      runtime: 'cloud' as const,
      repositories: [{ url: 'https://github.com/acme/repo' }],
      model: 'composer-2',
      name: 'named agent',
      prompt: 'create safely',
      session_id: 'cloud-create-retry',
    };

    const first = await worker.executeRun(original);
    const rejected = await worker.executeRun({ ...original, name: undefined });
    const recovered = await worker.executeRun(original);

    const createCalls = client.calls.filter((call) => call.method === 'CreateAgent');
    expect(first.is_error).toBe(true);
    expect(rejected.error).toContain('name must match the pending Cursor CreateAgent request');
    expect(recovered.status).toBe('FINISHED');
    expect(createCalls).toHaveLength(2);
    expect(createCalls[0]?.request.idempotencyKey).toBe(createCalls[1]?.request.idempotencyKey);
    expect((iii.state.get('cloud-create-retry') as SessionRecord).name).toBe('named agent');
  });

  it('never replays an ambiguous local Send', async () => {
    const iii = new MockIII();
    const client = new FakeBridgeClient(unaryResponse, () =>
      throwingStream(new Error('socket closed')),
    );
    const factory = new FakeBridgeFactory(client);
    const events: unknown[] = [];
    const worker = makeWorker(iii, factory, events);
    const request = {
      runtime: 'local' as const,
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'change code',
      session_id: 'local-ambiguous',
    };

    const first = await worker.executeRun(request);
    const second = await worker.executeRun(request);

    expect(first.recovery_required).toBe(true);
    expect(second.recovery_required).toBe(true);
    expect(client.calls.filter((call) => call.method === 'Send')).toHaveLength(1);
    expect((iii.state.get('local-ambiguous') as SessionRecord).status).toBe('recovery-required');
    expect(
      events.filter((event) => (event as { type?: string }).type === 'message_complete'),
    ).toHaveLength(1);
  });

  it('observes a persisted active run before sending anything else', async () => {
    const iii = new MockIII();
    iii.state.set('recover', activeRecord('recover', 'recover this', 'run-active'));
    const client = new FakeBridgeClient(unaryResponse, (call) => {
      if (call.method !== 'ObserveRun') throw new Error(`unexpected stream ${call.method}`);
      return frames(...terminalFrames('run-active', 'recovered'));
    });
    const worker = makeWorker(iii, new FakeBridgeFactory(client));

    const response = await worker.executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'recover this',
      session_id: 'recover',
    });

    expect(response.result).toBe('recovered');
    expect(client.calls.some((call) => call.method === 'ResumeAgent')).toBe(true);
    expect(client.calls.some((call) => call.method === 'ObserveRun')).toBe(true);
    expect(client.calls.some((call) => call.method === 'Send')).toBe(false);
    expect((iii.state.get('recover') as SessionRecord).active_run_id).toBeNull();
  });

  it('reissues a persisted cancellation before observing an active run after restart', async () => {
    const iii = new MockIII();
    const record = activeRecord('recover-cancel', 'cancel this', 'run-cancel');
    record.cancel_requested = true;
    iii.state.set(record.session_id, record);
    const client = new FakeBridgeClient(
      (call) => (call.method === 'CancelRun' ? {} : unaryResponse(call)),
      () => frames(...terminalFrames('run-cancel', '', 'RUN_LIFECYCLE_STATUS_CANCELLED')),
    );

    const response = await makeWorker(iii, new FakeBridgeFactory(client)).executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'cancel this',
      session_id: 'recover-cancel',
    });

    const cancelIndex = client.calls.findIndex((call) => call.method === 'CancelRun');
    const observeIndex = client.calls.findIndex((call) => call.method === 'ObserveRun');
    expect(cancelIndex).toBeGreaterThan(-1);
    expect(observeIndex).toBeGreaterThan(cancelIndex);
    expect(response.status).toBe('CANCELLED');
    expect((iii.state.get('recover-cancel') as SessionRecord).cancel_requested).toBe(false);
  });

  it('allows only one worker to observe and finalize a persisted active run', async () => {
    const iii = new MockIII();
    iii.state.set('active-race', activeRecord('active-race', 'resume me', 'run-active'));
    holdFirstStateReads(iii, 2);
    const firstClient = new FakeBridgeClient(unaryResponse, () =>
      frames(...terminalFrames('run-active', 'recovered')),
    );
    const secondClient = new FakeBridgeClient(unaryResponse, () =>
      frames(...terminalFrames('run-active', 'recovered')),
    );
    const request = {
      runtime: 'local' as const,
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'resume me',
      session_id: 'active-race',
    };

    const responses = await Promise.all([
      makeWorker(iii, new FakeBridgeFactory(firstClient)).executeRun(request),
      makeWorker(iii, new FakeBridgeFactory(secondClient)).executeRun(request),
    ]);

    expect(
      [...firstClient.calls, ...secondClient.calls].filter((call) => call.method === 'ObserveRun'),
    ).toHaveLength(1);
    expect(responses.filter((response) => response.busy)).toHaveLength(1);
    expect((iii.state.get('active-race') as SessionRecord).last_run_id).toBe('run-active');
  });

  it('refreshes the active-run claim while WaitLiveRun is still pending', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(600_000);
    try {
      const iii = new MockIII();
      iii.state.set('wait-heartbeat', activeRecord('wait-heartbeat', 'wait', 'run-wait'));
      let finishWait: (value: unknown) => void = () => undefined;
      const waiting = new Promise<unknown>((resolvePromise) => {
        finishWait = resolvePromise;
      });
      const client = new FakeBridgeClient(
        (call) => (call.method === 'WaitLiveRun' ? waiting : unaryResponse(call)),
        () => throwingStream(new BridgeTransportError('observe disconnected')),
      );
      const run = makeWorker(iii, new FakeBridgeFactory(client)).executeRun({
        runtime: 'local',
        cwd: '/repo',
        model: 'composer-2',
        prompt: 'wait',
        session_id: 'wait-heartbeat',
      });
      for (let attempt = 0; attempt < 20; attempt += 1) {
        if (client.calls.some((call) => call.method === 'WaitLiveRun')) break;
        await Promise.resolve();
      }
      const claimedAt = (iii.state.get('wait-heartbeat') as SessionRecord).claim_started_at_ms;

      await vi.advanceTimersByTimeAsync(30_000);

      expect(
        (iii.state.get('wait-heartbeat') as SessionRecord).claim_started_at_ms,
      ).toBeGreaterThan(claimedAt ?? 0);
      finishWait({
        result: {
          agentId: 'agent-existing',
          runId: 'run-wait',
          status: 'RUN_LIFECYCLE_STATUS_FINISHED',
          result: 'waited',
        },
      });
      expect((await run).result).toBe('waited');
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not publish non-durable Send deltas and de-duplicates durable Observe replay', async () => {
    const iii = new MockIII();
    const emit = makeEmitter(iii.asClient(), () => 'agent::events');
    const emitRaw = makeEmitter(iii.asClient(), () => 'cursor::events');
    const firstClient = new FakeBridgeClient(unaryResponse, (call) => firstProcessStream(call));
    const firstWorker = new CursorWorker(
      iii.asClient(),
      testConfig,
      emit,
      emitRaw,
      new FakeBridgeFactory(firstClient),
    );
    const request = {
      runtime: 'local' as const,
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'recover stream',
      session_id: 'stream-replay',
    };

    const failed = await firstWorker.executeRun(request);
    expect(
      iii.streamItems.filter((item) => {
        const payload = item as { stream_name?: string; data?: { type?: string } };
        return payload.stream_name === 'agent::events' && payload.data?.type === 'message_complete';
      }),
    ).toHaveLength(0);
    const secondClient = new FakeBridgeClient(unaryResponse, (call) => {
      if (call.method !== 'ObserveRun') throw new Error(`unexpected stream ${call.method}`);
      return frames(...durableFrames());
    });
    const recovered = await new CursorWorker(
      iii.asClient(),
      testConfig,
      emit,
      emitRaw,
      new FakeBridgeFactory(secondClient),
    ).executeRun(request);

    const normalizedFinal = iii.streamItems.filter((item) => {
      const payload = item as { stream_name?: string; data?: { llm_event?: { delta?: string } } };
      return payload.stream_name === 'agent::events' && payload.data?.llm_event?.delta === 'Hello';
    });
    const nonDurable = iii.streamItems.filter((item) => {
      const payload = item as {
        stream_name?: string;
        data?: { interactionUpdate?: { update?: { delta?: string } } };
      };
      return payload.data?.interactionUpdate?.update?.delta === 'Hel';
    });
    expect(failed.is_error).toBe(true);
    expect(recovered.status).toBe('FINISHED');
    expect(normalizedFinal).toHaveLength(1);
    expect(nonDurable).toHaveLength(0);

    async function* firstProcessStream(call: { method: string }) {
      if (call.method === 'Send') {
        yield { interactionUpdate: { type: 'text-delta', update: { delta: 'Hel' } } };
        yield {
          sdkMessage: {
            type: 'system',
            message: { agent_id: 'agent-replay', run_id: 'run-replay' },
          },
        };
        return;
      }
      yield durableFrames()[0];
      throw new Error('simulated process exit during ObserveRun');
    }

    function durableFrames(): RunStreamMessageWire[] {
      return [
        {
          offset: 'observe-offset-one',
          sdkMessage: {
            type: 'assistant',
            message: {
              agent_id: 'agent-replay',
              run_id: 'run-replay',
              message: { content: [{ type: 'text', text: 'Hello' }] },
            },
          },
        },
        ...terminalFrames('run-replay', 'Hello').slice(1),
      ];
    }
  });

  it('recreates a cloud Bridge handle after Create succeeds before the first run id', async () => {
    const iii = new MockIII();
    const failedClient = new FakeBridgeClient(unaryResponse, () =>
      throwingStream(new BridgeTransportError('response lost before run id')),
    );
    const request = {
      runtime: 'cloud' as const,
      repositories: [{ url: 'https://github.com/acme/repo' }],
      model: 'composer-2',
      prompt: 'create once',
      session_id: 'lazy-cloud-create',
    };

    const failed = await makeWorker(iii, new FakeBridgeFactory(failedClient)).executeRun(request);
    expect((iii.state.get('lazy-cloud-create') as SessionRecord).agent_created).toBe(false);
    const recoveredClient = successfulClient('run-created');
    const recovered = await makeWorker(iii, new FakeBridgeFactory(recoveredClient)).executeRun(
      request,
    );

    const creates = [...failedClient.calls, ...recoveredClient.calls].filter(
      (call) => call.method === 'CreateAgent',
    );
    expect(failed.is_error).toBe(true);
    expect((iii.state.get('lazy-cloud-create') as SessionRecord).agent_created).toBe(true);
    expect(recovered.status).toBe('FINISHED');
    expect(creates).toHaveLength(2);
    expect(creates[0]?.request.idempotencyKey).toBe(creates[1]?.request.idempotencyKey);
    expect(recoveredClient.calls.some((call) => call.method === 'ResumeAgent')).toBe(false);
  });

  it('registers a persisted cloud run before observing it in a fresh Bridge process', async () => {
    const iii = new MockIII();
    const record = activeRecord('cloud-observe', 'resume cloud', 'run-cloud-active');
    record.runtime = 'cloud';
    record.agent_id = 'bc-existing';
    record.workspace = '';
    record.repositories = [{ url: 'https://github.com/acme/repo' }];
    iii.state.set(record.session_id, record);
    let registered = false;
    const client = new FakeBridgeClient(
      (call) => {
        if (call.method === 'GetRun') {
          expect(call.request).toMatchObject({
            runId: 'run-cloud-active',
            options: {
              runtime: 'RUNTIME_CLOUD',
              agentId: 'bc-existing',
              apiKey: 'key_test_secret',
            },
          });
          registered = true;
        }
        return unaryResponse(call);
      },
      (call) => {
        expect(call.method).toBe('ObserveRun');
        expect(registered).toBe(true);
        return frames(...terminalFrames('run-cloud-active', 'recovered cloud'));
      },
    );

    const response = await makeWorker(iii, new FakeBridgeFactory(client)).executeRun({
      runtime: 'cloud',
      repositories: [{ url: 'https://github.com/acme/repo' }],
      model: 'composer-2',
      prompt: 'resume cloud',
      session_id: 'cloud-observe',
    });

    expect(response.result).toBe('recovered cloud');
    expect(registered).toBe(true);
  });

  it('resumes a completed agent for a follow-up instead of creating it again', async () => {
    const iii = new MockIII();
    const completed = activeRecord('follow', 'old', 'run-old');
    completed.status = 'done';
    completed.turns = 1;
    completed.active_turn = null;
    completed.active_run_id = null;
    completed.last_run_id = 'run-old';
    completed.send_idempotency_key = null;
    completed.pending_prompt_sha256 = null;
    completed.usage = {
      input_tokens: 4,
      output_tokens: 5,
      cache_read_tokens: 0,
      cache_write_tokens: 0,
      total_tokens: 9,
    };
    completed.cost = { raw_cost_cents: 2, charged_cents: 1 };
    iii.state.set('follow', completed);
    const client = successfulClient('run-next', 'next');
    const worker = makeWorker(iii, new FakeBridgeFactory(client));

    await worker.executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'new-model',
      prompt: 'new prompt',
      session_id: 'follow',
    });

    expect(client.calls.some((call) => call.method === 'CreateAgent')).toBe(false);
    expect(client.calls.some((call) => call.method === 'ResumeAgent')).toBe(true);
    const resume = client.calls.find((call) => call.method === 'ResumeAgent');
    expect(resume?.request).toMatchObject({ options: { model: { id: 'new-model' } } });
    const final = iii.state.get('follow') as SessionRecord;
    expect(final.turns).toBe(2);
    expect(final.model).toBe('new-model');
    expect(final.usage).toBeNull();
    expect(final.cost).toBeNull();
  });

  it('claims a concurrent follow-up before applying its model through ResumeAgent', async () => {
    const iii = new MockIII();
    const completed = activeRecord('model-race', 'old', 'run-old');
    completed.status = 'done';
    completed.turns = 1;
    completed.active_turn = null;
    completed.active_run_id = null;
    completed.last_run_id = 'run-old';
    completed.send_idempotency_key = null;
    completed.send_started = false;
    completed.claim_id = null;
    completed.claim_started_at_ms = null;
    completed.pending_prompt_sha256 = null;
    iii.state.set('model-race', completed);
    holdFirstStateReads(iii, 2);
    const firstClient = successfulClient('run-next');
    const secondClient = successfulClient('run-next');

    const responses = await Promise.all([
      makeWorker(iii, new FakeBridgeFactory(firstClient)).executeRun({
        runtime: 'local',
        cwd: '/repo',
        model: 'model-a',
        prompt: 'next A',
        session_id: 'model-race',
      }),
      makeWorker(iii, new FakeBridgeFactory(secondClient)).executeRun({
        runtime: 'local',
        cwd: '/repo',
        model: 'model-b',
        prompt: 'next B',
        session_id: 'model-race',
      }),
    ]);

    const calls = [...firstClient.calls, ...secondClient.calls];
    const resumes = calls.filter((call) => call.method === 'ResumeAgent');
    expect(resumes).toHaveLength(1);
    expect(calls.filter((call) => call.method === 'Send')).toHaveLength(1);
    expect(responses.filter((response) => response.busy)).toHaveLength(1);
    const appliedModel = (resumes[0]?.request.options as { model: { id: string } }).model.id;
    expect((iii.state.get('model-race') as SessionRecord).model).toBe(appliedModel);
  });

  it('does not mutate a completed session when follow-up-only validation fails', async () => {
    const iii = new MockIII();
    const completed = activeRecord('unchanged', 'old', 'run-old');
    completed.status = 'done';
    completed.turns = 1;
    completed.active_turn = null;
    completed.active_run_id = null;
    completed.last_run_id = 'run-old';
    completed.send_idempotency_key = null;
    completed.pending_prompt_sha256 = null;
    iii.state.set('unchanged', clone(completed));
    const before = clone(iii.state.get('unchanged'));
    const client = successfulClient();
    const events: unknown[] = [];
    const worker = makeWorker(iii, new FakeBridgeFactory(client), events);

    const response = await worker.executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'next',
      name: 'cannot-rename',
      session_id: 'unchanged',
    });

    expect(response.is_error).toBe(true);
    expect(response.error).toContain('name can only be set');
    expect(iii.state.get('unchanged')).toEqual(before);
    expect(client.calls).toHaveLength(0);
    expect(events).toHaveLength(0);
  });

  it('does not reuse a pending cloud Send key for a different prompt or model', async () => {
    const iii = new MockIII();
    const pending = activeRecord('pending-cloud', 'prompt A', 'run-unused');
    pending.runtime = 'cloud';
    pending.workspace = '';
    pending.repositories = [{ url: 'https://github.com/acme/repo' }];
    pending.status = 'error';
    pending.active_run_id = null;
    pending.model = 'model-a';
    pending.claim_id = null;
    pending.claim_started_at_ms = null;
    iii.state.set('pending-cloud', clone(pending));
    const before = clone(iii.state.get('pending-cloud'));
    const client = successfulClient();

    const response = await makeWorker(iii, new FakeBridgeFactory(client)).executeRun({
      runtime: 'cloud',
      repositories: [{ url: 'https://github.com/acme/repo' }],
      model: 'model-b',
      prompt: 'prompt B',
      session_id: 'pending-cloud',
    });

    expect(response.error).toContain('model must match the pending Cursor Send request');
    expect(iii.state.get('pending-cloud')).toEqual(before);
    expect(client.calls).toHaveLength(0);
  });

  it('merges fetched usage without overwriting concurrent run state', async () => {
    const iii = new MockIII();
    const record = activeRecord('usage-race', 'active prompt', 'run-active');
    record.runtime = 'cloud';
    record.workspace = '';
    record.repositories = [{ url: 'https://github.com/acme/repo' }];
    iii.state.set('usage-race', record);
    let releaseUsage: () => void = () => undefined;
    const usageReady = new Promise<void>((resolvePromise) => {
      releaseUsage = resolvePromise;
    });
    let usageCalled: () => void = () => undefined;
    const usageStarted = new Promise<void>((resolvePromise) => {
      usageCalled = resolvePromise;
    });
    const client = new FakeBridgeClient(
      async (call) => {
        if (call.method !== 'GetUsage') return unaryResponse(call);
        usageCalled();
        await usageReady;
        return {
          usage: {
            usage: {
              inputTokens: '2',
              outputTokens: '3',
              cacheReadTokens: '0',
              cacheWriteTokens: '0',
              totalTokens: '5',
            },
            runs: [],
          },
        };
      },
      () => frames(),
    );
    const worker = makeWorker(iii, new FakeBridgeFactory(client));
    worker.register();

    const usagePromise = iii.functions.get('cursor::usage')?.handler({
      session_id: 'usage-race',
    });
    await usageStarted;
    const concurrent = clone(iii.state.get('usage-race')) as SessionRecord;
    concurrent.active_run_id = 'run-newer';
    concurrent.send_idempotency_key = 'send-newer';
    concurrent.updated_at_ms += 1;
    iii.state.set('usage-race', concurrent);
    releaseUsage();
    await usagePromise;

    const final = iii.state.get('usage-race') as SessionRecord;
    expect(final.active_run_id).toBe('run-newer');
    expect(final.send_idempotency_key).toBe('send-newer');
    expect(final.usage?.total_tokens).toBe(5);
  });

  it('fails before creating a Bridge client when the API key is blank and emits a terminal error', async () => {
    const iii = new MockIII();
    const client = successfulClient();
    const factory = new FakeBridgeFactory(client);
    const events: unknown[] = [];
    const worker = makeWorker(iii, factory, events, [], { api_key: '   ' });

    const response = await worker.executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'test',
      session_id: 'no-key',
    });

    expect(response.error).toContain('Cursor API key is not configured');
    expect(response.error).not.toContain('key_test_secret');
    expect(factory.options).toHaveLength(0);
    expect(events.map((event) => (event as { type: string }).type).slice(-3)).toEqual([
      'message_complete',
      'turn_end',
      'agent_end',
    ]);
  });

  it('persists terminal state before publishing terminal success events', async () => {
    const iii = new MockIII();
    const order: string[] = [];
    const originalTrigger = iii.trigger.bind(iii);
    iii.trigger = async (request: Record<string, unknown>) => {
      if (request.function_id === 'state::compare-and-set') {
        const value = (request.payload as { value: SessionRecord }).value;
        if (value.status === 'done') order.push('save-done');
      }
      return originalTrigger(request);
    };
    const client = successfulClient();
    const worker = new CursorWorker(
      iii.asClient(),
      testConfig,
      async (_group, event) => {
        if ((event as { type?: string }).type === 'message_complete') order.push('complete');
      },
      async () => undefined,
      new FakeBridgeFactory(client),
    );

    await worker.executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'test',
      session_id: 'ordered',
    });

    expect(order).toEqual(['save-done', 'complete']);
  });

  it('passes the live agent id when cancelling a run', async () => {
    const iii = new MockIII();
    let runSeen: () => void = () => undefined;
    const started = new Promise<void>((resolvePromise) => {
      runSeen = resolvePromise;
    });
    let releaseRun: () => void = () => undefined;
    const released = new Promise<void>((resolvePromise) => {
      releaseRun = resolvePromise;
    });
    const client = new FakeBridgeClient(
      (call) => {
        if (call.method === 'CancelRun') return {};
        return unaryResponse(call);
      },
      (call) => {
        if (call.method === 'Send') {
          return frames({
            sdkMessage: {
              type: 'system',
              message: { agent_id: 'agent-live', run_id: 'run-live' },
            },
          });
        }
        runSeen();
        return liveStream();
      },
    );
    const worker = makeWorker(iii, new FakeBridgeFactory(client));
    worker.register();

    const start = await iii.functions.get('cursor::start')?.handler({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'wait',
      session_id: 'cancel-live',
    });
    await started;
    const stop = await iii.functions.get('cursor::stop')?.handler({
      session_id: 'cancel-live',
    });
    releaseRun();
    await vi.waitFor(() => {
      expect((iii.state.get('cancel-live') as SessionRecord).status).toBe('cancelled');
    });

    const cancel = client.calls.find((call) => call.method === 'CancelRun');
    expect(start).toEqual({ session_id: 'cancel-live', started: true });
    expect(stop).toEqual({ session_id: 'cancel-live', stopped: true, reason: null });
    expect(cancel?.request).toMatchObject({
      agentId: expect.stringMatching(/^agent-/),
      runId: 'run-live',
    });

    async function* liveStream() {
      await released;
      yield* terminalFrames('run-live', '', 'RUN_LIFECYCLE_STATUS_CANCELLED');
    }
  });

  it('persists a detached stop request before Send reports a run id', async () => {
    const iii = new MockIII();
    let markSendStarted: () => void = () => undefined;
    const sendStarted = new Promise<void>((resolvePromise) => {
      markSendStarted = resolvePromise;
    });
    let releaseSend: () => void = () => undefined;
    const sendReleased = new Promise<void>((resolvePromise) => {
      releaseSend = resolvePromise;
    });
    const events: unknown[] = [];
    const ownerClient = new FakeBridgeClient(
      (call) => (call.method === 'CancelRun' ? {} : unaryResponse(call)),
      (call) => (call.method === 'Send' ? delayedSend() : cancelledRun()),
    );
    const ownerRun = makeWorker(iii, new FakeBridgeFactory(ownerClient), events).executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'wait for the run id',
      session_id: 'pending-cancel',
    });
    await sendStarted;
    const detachedClient = successfulClient();
    const detachedFactory = new FakeBridgeFactory(detachedClient);
    const detachedWorker = makeWorker(iii, detachedFactory, events);
    detachedWorker.register();

    const stopped = await iii.functions.get('cursor::stop')?.handler({
      session_id: 'pending-cancel',
    });
    expect((iii.state.get('pending-cancel') as SessionRecord).cancel_requested).toBe(true);
    expect(detachedFactory.options).toHaveLength(0);
    releaseSend();
    const response = await ownerRun;

    expect(stopped).toEqual({ session_id: 'pending-cancel', stopped: true, reason: null });
    expect(response.status).toBe('CANCELLED');
    expect(ownerClient.calls.filter((call) => call.method === 'CancelRun')).toHaveLength(1);
    expect((iii.state.get('pending-cancel') as SessionRecord).cancel_requested).toBe(false);
    expect(
      events.filter((event) => (event as { type?: string }).type === 'message_complete'),
    ).toHaveLength(1);

    async function* delayedSend() {
      markSendStarted();
      await sendReleased;
      yield {
        sdkMessage: {
          type: 'system',
          message: { agent_id: 'agent-pending', run_id: 'run-pending' },
        },
      };
    }

    async function* cancelledRun() {
      yield* terminalFrames('run-pending', '', 'RUN_LIFECYCLE_STATUS_CANCELLED');
    }
  });

  it('reconciles a detached cancellation before accepting a different follow-up', async () => {
    const iii = new MockIII();
    iii.state.set('detached-stop', activeRecord('detached-stop', 'old prompt', 'run-old'));
    const client = new FakeBridgeClient(
      (call) => {
        if (call.method === 'CancelRun') return {};
        if (call.method === 'WaitLiveRun') {
          return {
            result: {
              agentId: 'agent-existing',
              runId: 'run-old',
              status: 'RUN_LIFECYCLE_STATUS_CANCELLED',
              result: '',
            },
          };
        }
        return unaryResponse(call);
      },
      () => frames(...terminalFrames('run-next', 'next result')),
    );
    const worker = makeWorker(iii, new FakeBridgeFactory(client));
    worker.register();

    const stopped = await iii.functions.get('cursor::stop')?.handler({
      session_id: 'detached-stop',
    });
    const followUp = await worker.executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'different prompt',
      session_id: 'detached-stop',
    });

    expect(stopped).toEqual({ session_id: 'detached-stop', stopped: true, reason: null });
    expect(followUp.status).toBe('FINISHED');
    expect((iii.state.get('detached-stop') as SessionRecord).last_run_id).toBe('run-next');
    expect(client.calls.filter((call) => call.method === 'CancelRun')).toHaveLength(1);
  });

  it('finalizes an already-terminal detached run without attempting cancellation', async () => {
    const iii = new MockIII();
    iii.state.set('terminal-stop', activeRecord('terminal-stop', 'old prompt', 'run-old'));
    const client = new FakeBridgeClient(
      (call) => {
        if (call.method === 'GetRun' && call.request.runId === 'run-old') {
          return {
            run: {
              agentId: 'agent-existing',
              runId: 'run-old',
              status: 'RUN_LIFECYCLE_STATUS_FINISHED',
              result: 'already done',
            },
          };
        }
        if (call.method === 'CancelRun' || call.method === 'WaitLiveRun') {
          throw new Error(`${call.method} must not be called for a terminal run`);
        }
        return unaryResponse(call);
      },
      () => frames(...terminalFrames('run-next', 'next result')),
    );
    const worker = makeWorker(iii, new FakeBridgeFactory(client));
    worker.register();

    const stopped = await iii.functions.get('cursor::stop')?.handler({
      session_id: 'terminal-stop',
    });
    const followUp = await worker.executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'different prompt',
      session_id: 'terminal-stop',
    });

    expect(stopped).toEqual({ session_id: 'terminal-stop', stopped: true, reason: null });
    expect(followUp.status).toBe('FINISHED');
    expect(client.calls.some((call) => call.method === 'CancelRun')).toBe(false);
    expect(client.calls.some((call) => call.method === 'WaitLiveRun')).toBe(false);
  });

  it('emits one terminal event when detached stop takes ownership from an observer', async () => {
    const iii = new MockIII();
    iii.state.set('stop-race', activeRecord('stop-race', 'wait', 'run-race'));
    let markObserveStarted: () => void = () => undefined;
    const observeStarted = new Promise<void>((resolvePromise) => {
      markObserveStarted = resolvePromise;
    });
    let releaseObserve: () => void = () => undefined;
    const observeReleased = new Promise<void>((resolvePromise) => {
      releaseObserve = resolvePromise;
    });
    const events: unknown[] = [];
    const observerClient = new FakeBridgeClient(unaryResponse, () => delayedObserve());
    const observerRun = makeWorker(iii, new FakeBridgeFactory(observerClient), events).executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'wait',
      session_id: 'stop-race',
    });
    await observeStarted;
    const stopClient = new FakeBridgeClient(
      (call) => {
        if (call.method === 'CancelRun') return {};
        if (call.method === 'WaitLiveRun') {
          return {
            result: {
              agentId: 'agent-existing',
              runId: 'run-race',
              status: 'RUN_LIFECYCLE_STATUS_CANCELLED',
              result: '',
            },
          };
        }
        return unaryResponse(call);
      },
      () => frames(),
    );
    const stopper = makeWorker(iii, new FakeBridgeFactory(stopClient), events);
    stopper.register();

    const stopped = await iii.functions.get('cursor::stop')?.handler({
      session_id: 'stop-race',
    });
    releaseObserve();
    const observerResponse = await observerRun;

    expect(stopped).toEqual({ session_id: 'stop-race', stopped: true, reason: null });
    expect(observerResponse.status).toBe('CANCELLED');
    expect(
      events.filter((event) => (event as { type?: string }).type === 'message_complete'),
    ).toHaveLength(1);

    async function* delayedObserve() {
      markObserveStarted();
      await observeReleased;
      yield* terminalFrames('run-race', '', 'RUN_LIFECYCLE_STATUS_CANCELLED');
    }
  });

  it('resumes the agent before reading usage through a fresh Bridge process', async () => {
    const iii = new MockIII();
    const record = activeRecord('usage-fresh', 'old', 'run-old');
    record.runtime = 'cloud';
    record.agent_id = 'bc-usage';
    record.workspace = '';
    record.repositories = [{ url: 'https://github.com/acme/repo' }];
    record.status = 'done';
    record.active_run_id = null;
    record.last_run_id = 'run-old';
    record.send_idempotency_key = null;
    record.send_started = false;
    record.claim_id = null;
    record.claim_started_at_ms = null;
    record.pending_prompt_sha256 = null;
    iii.state.set(record.session_id, record);
    let resumed = false;
    const client = new FakeBridgeClient(
      (call) => {
        if (call.method === 'ResumeAgent') resumed = true;
        if (call.method === 'GetUsage') {
          expect(resumed).toBe(true);
          return { usage: { runs: [] } };
        }
        return unaryResponse(call);
      },
      () => frames(),
    );
    const worker = makeWorker(iii, new FakeBridgeFactory(client));
    worker.register();

    await iii.functions.get('cursor::usage')?.handler({ session_id: 'usage-fresh' });

    expect(resumed).toBe(true);
  });

  it('falls back to a complete body when a streamed delta cannot be persisted', async () => {
    const iii = new MockIII();
    const originalTrigger = iii.trigger.bind(iii);
    let rejected = false;
    iii.trigger = async (request: Record<string, unknown>) => {
      const payload = request.payload as { stream_name?: string; data?: { type?: string } };
      if (
        !rejected &&
        request.function_id === 'stream::set' &&
        payload.stream_name === 'agent::events' &&
        payload.data?.type === 'message_update'
      ) {
        rejected = true;
        throw new Error('stream unavailable');
      }
      return originalTrigger(request);
    };
    const emit = makeEmitter(iii.asClient(), () => 'agent::events');
    const worker = new CursorWorker(
      iii.asClient(),
      testConfig,
      emit,
      makeEmitter(iii.asClient(), () => 'cursor::events'),
      new FakeBridgeFactory(successfulClient('run-delivery', 'complete text')),
    );

    const response = await worker.executeRun({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'test delivery',
      session_id: 'delivery-failure',
    });
    const complete = iii.streamItems.find((item) => {
      const payload = item as { stream_name?: string; data?: { type?: string } };
      return payload.stream_name === 'agent::events' && payload.data?.type === 'message_complete';
    }) as { data?: { body_streamed?: boolean } };

    expect(response.status).toBe('FINISHED');
    expect(rejected).toBe(true);
    expect(complete.data).not.toHaveProperty('body_streamed');
  });
});

describe('Cursor worker interface', () => {
  it('registers the standard alias and concrete request and response schemas', () => {
    const iii = new MockIII();
    const worker = makeWorker(iii, new FakeBridgeFactory(successfulClient()));
    worker.register();

    expect([...iii.functions.keys()]).toEqual([
      'cursor::run',
      'run::start_and_wait',
      'cursor::start',
      'cursor::stop',
      'cursor::status',
      'cursor::sessions::list',
      'cursor::models::list',
      'cursor::usage',
    ]);
    for (const registration of iii.functions.values()) {
      const request = registration.options.request_format as Record<string, unknown>;
      expect(
        request.type === 'object' || Array.isArray(request.oneOf) || Array.isArray(request.anyOf),
      ).toBe(true);
      expect(registration.options.response_format).toMatchObject({ type: 'object' });
      expect(registration.options.response_format).not.toEqual({});
    }
  });

  it('accepts the exact ACP external-brain payload as a sandboxed local run', async () => {
    const iii = new MockIII();
    const client = successfulClient('run-acp', 'editor result');
    const worker = makeWorker(iii, new FakeBridgeFactory(client));
    worker.register();

    const response = await iii.functions.get('run::start_and_wait')?.handler({
      session_id: 'acp-session',
      cwd: '/editor/workspace',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'from editor' }] }],
      timeout_ms: 600_000,
      model: 'composer-2',
      provider: 'cursor',
      system_prompt: 'Use the repository context.',
    });

    expect(response).toMatchObject({ status: 'FINISHED', stop_reason: 'end' });
    expect(client.calls.find((call) => call.method === 'CreateAgent')?.request).toMatchObject({
      options: {
        model: { id: 'composer-2' },
        local: { cwd: ['/editor/workspace'], sandboxOptions: { enabled: true } },
      },
    });
    expect(client.calls.find((call) => call.method === 'Send')?.request).toMatchObject({
      message: { text: 'Use the repository context.\n\nfrom editor' },
    });
  });

  it('does not accept a background start that cannot be recovered safely', async () => {
    const iii = new MockIII();
    const record = activeRecord('stuck-local', 'change code', 'run-unused');
    record.status = 'recovery-required';
    record.active_run_id = null;
    iii.state.set('stuck-local', record);
    const client = successfulClient();
    const worker = makeWorker(iii, new FakeBridgeFactory(client));
    worker.register();

    const start = iii.functions.get('cursor::start');
    const response = await start?.handler({
      runtime: 'local',
      cwd: '/repo',
      model: 'composer-2',
      prompt: 'change code',
      session_id: 'stuck-local',
    });

    expect(response).toEqual({ session_id: 'stuck-local', started: false });
    expect(client.calls).toHaveLength(0);
  });

  it('preserves tools presence and extracts the last user message', () => {
    const parsed = RunPayloadSchema.parse({
      runtime: 'local',
      cwd: '/repo',
      model: 'model',
      tools: [],
      messages: [
        { role: 'user', content: 'first' },
        { role: 'assistant', content: 'reply' },
        { role: 'user', content: [{ type: 'text', text: 'last' }] },
      ],
    });
    if (parsed.runtime !== 'local') throw new Error('expected local payload');
    expect(parsed.tools).toEqual([]);
    expect(extractPrompt(parsed)).toBe('last');
  });
});

function makeWorker(
  iii: MockIII,
  factory: FakeBridgeFactory,
  normalized: unknown[] = [],
  raw: unknown[] = [],
  configOverrides: Parameters<typeof testConfig>[0] = {},
): CursorWorker {
  return new CursorWorker(
    iii.asClient(),
    () => testConfig(configOverrides),
    async (_group, event) => {
      normalized.push(clone(event));
    },
    async (_group, event) => {
      raw.push(clone(event));
    },
    factory,
  );
}

function successfulClient(runId = 'run-one', text = 'done'): FakeBridgeClient {
  return new FakeBridgeClient(unaryResponse, () => frames(...terminalFrames(runId, text)));
}

function unaryResponse(call: { method: string; request: Record<string, unknown> }): unknown {
  if (call.method === 'CreateAgent') {
    const options = call.request.options as { agentId: string };
    return { agentId: options.agentId };
  }
  if (call.method === 'ResumeAgent') return { agentId: call.request.agentId };
  if (call.method === 'GetRun') {
    return {
      run: {
        runId: call.request.runId,
        agentId:
          (call.request.options as { agentId?: string } | undefined)?.agentId ?? 'agent-existing',
        status: 'RUN_LIFECYCLE_STATUS_RUNNING',
      },
    };
  }
  if (call.method === 'GetUsage') return { usage: { runs: [] } };
  throw new Error(`unexpected unary ${call.method}`);
}

async function* throwingStream(error: Error): AsyncIterable<unknown> {
  yield* [];
  throw error;
}

function activeRecord(sessionId: string, prompt: string, runId: string): SessionRecord {
  return {
    session_id: sessionId,
    agent_id: 'agent-existing',
    runtime: 'local',
    workspace: '/repo',
    name: null,
    model: 'composer-2',
    tools: ['read', 'grep', 'glob', 'ls'],
    repositories: [],
    work_on_current_branch: false,
    auto_create_pr: false,
    status: 'working',
    agent_created: true,
    turns: 0,
    active_turn: 1,
    active_run_id: runId,
    last_run_id: null,
    create_idempotency_key: 'create-key',
    send_idempotency_key: 'send-key',
    send_started: true,
    cancel_requested: false,
    claim_id: 'claim-existing',
    claim_started_at_ms: 1,
    pending_prompt_sha256: createHash('sha256').update(prompt).digest('hex'),
    usage: null,
    cost: null,
    updated_at_ms: 1,
  };
}

function holdFirstStateReads(iii: MockIII, count: number): void {
  const originalTrigger = iii.trigger.bind(iii);
  let reads = 0;
  let release: () => void = () => undefined;
  const ready = new Promise<void>((resolvePromise) => {
    release = resolvePromise;
  });
  iii.trigger = async (request: Record<string, unknown>) => {
    if (request.function_id === 'state::get' && reads < count) {
      reads += 1;
      if (reads === count) release();
      await ready;
    }
    return originalTrigger(request);
  };
}
