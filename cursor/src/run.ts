import { createHash, randomUUID } from 'node:crypto';
import { resolve } from 'node:path';
import type { IIIClient } from 'iii-sdk';
import { z } from 'zod';
import {
  BridgeRpcError,
  BridgeTransportError,
  type BridgeClient,
  type BridgeClientFactory,
} from './bridge.js';
import {
  type CursorAcpSession,
  type CursorAcpSessionUpdate,
  type CursorCliClient,
  CursorCliError,
  type CursorCliFactory,
  ProductionCursorCliFactory,
  resolveCursorAcpModelId,
} from './cli.js';
import { bridgeLaunchOptions, type Config, cursorCliLaunchOptions } from './config.js';
import { type Emit, releaseEmitterSequence } from './events.js';
import {
  extractRunId,
  mapAgentInfo,
  mapModels,
  mapRunSnapshot,
  mapUsageResponse,
  normalizeEnum,
  RunAccumulator,
} from './map.js';
import { jsonSchema } from './schema.js';
import { compareAndSetSession, listSessions, loadSession, updateSession } from './state.js';
import {
  AgentInfoSchema,
  AgentUsageSchema,
  CreateAgentResponseWireSchema,
  CursorModelSchema,
  GetAgentResponseWireSchema,
  GetRunResponseWireSchema,
  GetUsageResponseWireSchema,
  ListModelsResponseWireSchema,
  RepositorySchema,
  RunResultWireSchema,
  RunSnapshotSchema,
  RunStreamMessageWireSchema,
  SessionRecordSchema,
  TokenUsageSchema,
  UsageCostSchema,
  type Repository,
  type RunStreamMessageWire,
  type SessionRecord,
} from './types.js';

const SAFE_LOCAL_TOOLS = ['read', 'grep', 'glob', 'ls'];
const EmptyResponseWireSchema = z.object({}).passthrough();
const WaitLiveRunResponseWireSchema = z.object({ result: RunResultWireSchema }).passthrough();

const ContentBlockSchema = z.object({ type: z.string() }).passthrough();
const InputMessageSchema = z.object({
  role: z.string(),
  content: z.union([z.string(), z.array(ContentBlockSchema)]),
});
type InputMessage = z.infer<typeof InputMessageSchema>;

const SharedRunFields = {
  session_id: z.string().min(1).optional(),
  prompt: z.string().optional(),
  messages: z.array(InputMessageSchema).optional(),
  name: z.string().min(1).optional(),
};

const LocalRunPayloadSchema = z.object({
  ...SharedRunFields,
  runtime: z.literal('local'),
  cwd: z.string().min(1),
  model: z.string().min(1),
  tools: z.array(z.string()).optional(),
  timeout_ms: z.number().int().positive().optional(),
});

const CloudRunPayloadSchema = z.object({
  ...SharedRunFields,
  runtime: z.literal('cloud'),
  model: z.string().min(1).optional(),
  repositories: z.array(RepositorySchema).optional(),
  work_on_current_branch: z.boolean().optional(),
  auto_create_pr: z.boolean().optional(),
});

export const RunPayloadSchema = z.discriminatedUnion('runtime', [
  LocalRunPayloadSchema,
  CloudRunPayloadSchema,
]);
export type RunPayload = z.infer<typeof RunPayloadSchema>;

const AcpRunPayloadSchema = z.object({
  runtime: z.never().optional(),
  session_id: z.string().min(1).optional(),
  prompt: z.string().optional(),
  messages: z.array(InputMessageSchema).optional(),
  cwd: z.string().min(1),
  model: z.string().min(1),
  provider: z.literal('cursor').optional(),
  system_prompt: z.string().optional(),
  timeout_ms: z.number().int().positive().optional(),
});

export const CursorRunRequestSchema = z.union([RunPayloadSchema, AcpRunPayloadSchema]);

export const RunResponseSchema = z.object({
  session_id: z.string(),
  agent_id: z.string().nullable(),
  run_id: z.string().nullable(),
  result: z.string(),
  status: z.string(),
  stop_reason: z.enum(['end', 'length', 'aborted', 'error']).nullable(),
  is_error: z.boolean(),
  usage: TokenUsageSchema.nullable(),
  cost: UsageCostSchema.nullable(),
  busy: z.boolean(),
  recovery_required: z.boolean(),
  error: z.string().nullable(),
  error_details: z
    .object({
      transport_code: z.string().nullable(),
      sdk_error_code: z.union([z.string(), z.number()]).nullable(),
      request_id: z.string().nullable(),
      retry_after_seconds: z.number().nonnegative().nullable(),
      rate_limit: z
        .object({
          limit: z.string().nullable(),
          remaining: z.string().nullable(),
          reset_epoch_seconds: z.string().nullable(),
        })
        .nullable(),
    })
    .nullable(),
});
export type RunResponse = z.infer<typeof RunResponseSchema>;

const SessionIdSchema = z.object({ session_id: z.string().min(1) });
const StartResponseSchema = z.object({ session_id: z.string(), started: z.boolean() });
const StopResponseSchema = z.object({
  session_id: z.string(),
  stopped: z.boolean(),
  reason: z.string().nullable(),
});
const StatusResponseSchema = z.object({
  session_id: z.string(),
  live: z.boolean(),
  record: SessionRecordSchema.nullable(),
  agent: AgentInfoSchema.nullable(),
  run: RunSnapshotSchema.nullable(),
  remote_error: z.string().nullable(),
});
const SessionsResponseSchema = z.object({ sessions: z.array(SessionRecordSchema) });
const ModelsRequestSchema = z.object({
  backend: z.enum(['cli-acp', 'sdk-bridge']).optional(),
});
const ModelsResponseSchema = z.object({ models: z.array(CursorModelSchema) });
const UsageRequestSchema = z.object({
  session_id: z.string().min(1),
  run_id: z.string().min(1).optional(),
});
const UsageResponseSchema = z.object({
  session_id: z.string(),
  agent_id: z.string(),
  usage: AgentUsageSchema,
});
const AuthStatusResponseSchema = z.object({
  available: z.boolean(),
  authenticated: z.boolean(),
  status: z.enum(['authenticated', 'partial', 'unauthenticated', 'unavailable']),
  version: z.string().nullable(),
  login_command: z.literal('cursor-agent login'),
  error: z.string().nullable(),
});

type SessionBackend = 'cli-acp' | 'sdk-bridge';
type ReservationSettler = (accepted: boolean) => void;

type LiveRun = {
  client: BridgeClient | null;
  cli: CursorCliClient | null;
  agentId: string | null;
  runId: string | null;
  cancelRequested: boolean;
  cancelSent: boolean;
  claimId: string;
  sendStarted: boolean;
  leaseRefreshedAtMs: number;
};

export class CursorWorker {
  private readonly live = new Map<string, LiveRun>();

  constructor(
    private readonly iii: IIIClient,
    private readonly getConfig: () => Config,
    private readonly emit: Emit,
    private readonly emitRaw: Emit,
    private readonly factory: BridgeClientFactory,
    private readonly cliFactory: CursorCliFactory = new ProductionCursorCliFactory(),
  ) {}

  async executeRun(payload: RunPayload, onReservation?: ReservationSettler): Promise<RunResponse> {
    let reservationSettled = false;
    const settleReservation = (accepted: boolean): void => {
      if (reservationSettled) return;
      reservationSettled = true;
      onReservation?.(accepted);
    };
    const sessionId = payload.session_id ?? randomUUID();
    const existingLive = this.live.get(sessionId);
    if (existingLive) {
      settleReservation(false);
      return this.response(sessionId, null, {
        status: 'working',
        busy: true,
        error: 'a Cursor run is already active for this session',
      });
    }

    const handle: LiveRun = {
      client: null,
      cli: null,
      agentId: null,
      runId: null,
      cancelRequested: false,
      cancelSent: false,
      claimId: randomUUID(),
      sendStarted: false,
      leaseRefreshedAtMs: 0,
    };
    this.live.set(sessionId, handle);
    try {
      const config = this.getConfig();
      const prompt = extractPrompt(payload);
      const response = await this.runReserved(
        config,
        payload,
        sessionId,
        prompt,
        handle,
        settleReservation,
      );
      settleReservation(false);
      return response;
    } catch (error) {
      settleReservation(false);
      return await this.failRun(sessionId, error, handle);
    } finally {
      if (handle.cli) await handle.cli.close().catch(() => undefined);
      if (handle.client) await handle.client.close().catch(() => undefined);
      if (this.live.get(sessionId) === handle) this.live.delete(sessionId);
    }
  }

  register(): void {
    const runHandler = async (payload: unknown) => this.executeRun(parseRunRequest(payload));
    const runRegistration = {
      description:
        'Run one Cursor coding-agent turn and wait for its terminal result. Local runs default to the login-backed Cursor CLI ACP; cloud runs use the separately installed sdk.v1 Bridge. The ACP-compatible shape requires cwd and model.',
      request_format: jsonSchema(CursorRunRequestSchema),
      response_format: jsonSchema(RunResponseSchema),
    };
    this.iii.registerFunction('cursor::run', runHandler, runRegistration);
    this.iii.registerFunction('run::start_and_wait', runHandler, {
      ...runRegistration,
      description:
        'Standard agent alias for cursor::run. Configure ACP with --brain-fn cursor::run when more than one agent worker is installed.',
    });

    this.iii.registerFunction(
      'cursor::start',
      async (payload: unknown) => this.start(RunPayloadSchema.parse(payload ?? {})),
      {
        description:
          'Start a Cursor agent turn and return immediately. Watch agent::events with group_id equal to session_id.',
        request_format: jsonSchema(RunPayloadSchema),
        response_format: jsonSchema(StartResponseSchema),
      },
    );

    this.iii.registerFunction(
      'cursor::stop',
      async (payload: unknown) => this.stop(SessionIdSchema.parse(payload ?? {}).session_id),
      {
        description: 'Request cancellation of the active Cursor run for a session.',
        request_format: jsonSchema(SessionIdSchema),
        response_format: jsonSchema(StopResponseSchema),
      },
    );

    this.iii.registerFunction(
      'cursor::status',
      async (payload: unknown) => this.status(SessionIdSchema.parse(payload ?? {}).session_id),
      {
        description: 'Read durable local ACP or Bridge-reported status for a Cursor session.',
        request_format: jsonSchema(SessionIdSchema),
        response_format: jsonSchema(StatusResponseSchema),
      },
    );

    this.iii.registerFunction(
      'cursor::sessions::list',
      async () => ({ sessions: await listSessions(this.iii) }),
      {
        description: 'List durable iii-to-Cursor session mappings owned by this worker.',
        request_format: { type: 'object', properties: {}, additionalProperties: false },
        response_format: jsonSchema(SessionsResponseSchema),
      },
    );

    this.iii.registerFunction(
      'cursor::models::list',
      async (payload: unknown) => this.listModels(ModelsRequestSchema.parse(payload ?? {})),
      {
        description:
          'List the current account-scoped Cursor model catalog. Select sdk-bridge for the cloud/API-key catalog.',
        request_format: jsonSchema(ModelsRequestSchema),
        response_format: jsonSchema(ModelsResponseSchema),
      },
    );

    this.iii.registerFunction('cursor::auth::status', async () => this.authStatus(), {
      description:
        'Check whether the official Cursor Agent CLI can reuse an existing cursor-agent login. Account details and credentials are never returned.',
      request_format: { type: 'object', properties: {}, additionalProperties: false },
      response_format: jsonSchema(AuthStatusResponseSchema),
    });

    this.iii.registerFunction(
      'cursor::usage',
      async (payload: unknown) => this.usage(UsageRequestSchema.parse(payload ?? {})),
      {
        description:
          'Read Bridge-reported token usage and optional billed cost for a cloud Cursor session.',
        request_format: jsonSchema(UsageRequestSchema),
        response_format: jsonSchema(UsageResponseSchema),
      },
    );
  }

  async close(): Promise<void> {
    const timeoutMs = Math.max(1_000, this.getConfig().shutdown_timeout_ms * 2);
    const shutdown = (async () => {
      await this.markLiveCliSessionsForShutdown();
      await Promise.allSettled([this.factory.closeAll(), this.cliFactory.closeAll()]);
    })();
    if (!(await completesWithin(shutdown, timeoutMs))) this.forceClose();
  }

  forceClose(): void {
    this.factory.forceCloseAll();
    this.cliFactory.forceCloseAll();
  }

  private async markLiveCliSessionsForShutdown(): Promise<void> {
    await Promise.all(
      [...this.live.entries()].map(async ([sessionId, handle]) => {
        await updateSession(this.iii, sessionId, (current) => {
          if (
            sessionBackend(current) !== 'cli-acp' ||
            current.status !== 'working' ||
            current.claim_id !== handle.claimId
          ) {
            return null;
          }
          const now = Date.now();
          if (current.active_run_id || current.send_started) {
            return {
              ...current,
              status: 'recovery-required',
              cancel_requested: false,
              claim_id: null,
              claim_started_at_ms: null,
              updated_at_ms: now,
            };
          }
          return {
            ...current,
            status: 'error',
            agent_created:
              current.turns === 0 && !current.last_run_id ? false : current.agent_created,
            active_turn: null,
            send_idempotency_key: null,
            send_started: false,
            cancel_requested: false,
            claim_id: null,
            claim_started_at_ms: null,
            pending_prompt_sha256: null,
            updated_at_ms: now,
          };
        }).catch(() => null);
      }),
    );
  }

  private async start(payload: RunPayload): Promise<z.infer<typeof StartResponseSchema>> {
    const sessionId = payload.session_id ?? randomUUID();
    const request = { ...payload, session_id: sessionId };
    const config = this.getConfig();
    const prior = await loadSession(this.iii, sessionId);
    const backend = backendForRun(request, config, prior);
    validateCliTools(request, backend);
    validateBackendTimeout(request, backend);
    if (!prior) validateNewSessionPayload(request);
    if (backend === 'sdk-bridge') {
      bridgeLaunchOptions(config, request.runtime === 'local' ? request.cwd : undefined);
    } else {
      cursorCliLaunchOptions(config, request.runtime === 'local' ? request.cwd : undefined);
    }
    const promptHash = hash(extractPrompt(request));
    if (prior) {
      validateSessionBackend(prior, backend);
      validateExistingPayload(prior, request);
    }
    if (
      this.live.has(sessionId) ||
      prior?.status === 'recovery-required' ||
      Boolean(
        prior?.send_idempotency_key &&
          !prior.active_run_id &&
          prior.claim_id &&
          prior.send_started &&
          !claimIsStale(prior, config),
      ) ||
      Boolean(prior?.active_run_id && prior.claim_id && !claimIsStale(prior, config)) ||
      (prior?.status === 'working' && prior.pending_prompt_sha256 !== promptHash) ||
      (prior?.status === 'working' && request.model && request.model !== prior.model)
    ) {
      return { session_id: sessionId, started: false };
    }
    let resolveReservation: ReservationSettler = () => undefined;
    const reservation = new Promise<boolean>((resolvePromise) => {
      resolveReservation = resolvePromise;
    });
    void this.executeRun(request, resolveReservation).catch((error) => {
      console.error(`cursor background run failed for ${sessionId}: ${safeError(error)}`);
    });
    return { session_id: sessionId, started: await reservation };
  }

  private async runReserved(
    config: Config,
    payload: RunPayload,
    sessionId: string,
    prompt: string,
    handle: LiveRun,
    settleReservation: ReservationSettler,
  ): Promise<RunResponse> {
    const promptHash = hash(prompt);
    const prior = await loadSession(this.iii, sessionId);
    const requestedBackend = backendForRun(payload, config, prior);
    validateCliTools(payload, requestedBackend);
    validateBackendTimeout(payload, requestedBackend);
    if (!prior) validateNewSessionPayload(payload);
    if (requestedBackend === 'sdk-bridge') {
      bridgeLaunchOptions(config, payload.runtime === 'local' ? payload.cwd : undefined);
    } else {
      cursorCliLaunchOptions(config, payload.runtime === 'local' ? payload.cwd : undefined);
    }
    if (prior) {
      validateSessionBackend(prior, requestedBackend);
      validateExistingPayload(prior, payload);
    }
    if (prior?.status === 'recovery-required') {
      return this.response(sessionId, prior, {
        status: prior.status,
        recoveryRequired: true,
        error: recoveryGuidance(prior),
      });
    }
    if (prior?.status === 'working' && prior.pending_prompt_sha256 !== promptHash) {
      return this.response(sessionId, prior, {
        status: 'working',
        busy: true,
        error: 'a different prompt is already active for this session',
      });
    }
    if (prior?.status === 'working' && payload.model && payload.model !== prior.model) {
      return this.response(sessionId, prior, {
        status: 'working',
        busy: true,
        error: 'the active Cursor turn must resume with its original model',
      });
    }

    let record = prior
      ? { ...prior }
      : newSessionRecord(sessionId, payload, promptHash, handle.claimId, requestedBackend);
    if (!prior) {
      const created = await compareAndSetSession(this.iii, null, record);
      if (!created.swapped) {
        if (!created.current) throw new SessionConflictError('Cursor session mapping disappeared');
        validateExistingPayload(created.current, payload);
        return this.response(sessionId, created.current, {
          status: created.current.status,
          busy: true,
          error: 'another Cursor worker created this session concurrently',
        });
      }
    }
    const claimed = await this.claimTurn(record, payload, promptHash, handle, config);
    if ('busy' in claimed) return claimed;
    record = claimed;
    handle.leaseRefreshedAtMs = record.claim_started_at_ms ?? Date.now();
    settleReservation(true);

    if (sessionBackend(record) === 'cli-acp') {
      return this.runLocalAcp(
        config,
        record,
        handle,
        prompt,
        payload.runtime === 'local' ? payload.timeout_ms : undefined,
      );
    }

    handle.agentId = record.agent_id;
    const workspace = record.runtime === 'local' ? record.workspace : config.workspace;
    const client = this.factory.create(bridgeLaunchOptions(config, workspace));
    handle.client = client;
    if (!record.agent_created) {
      await this.createOrResumeAgent(client, record);
      if (record.runtime === 'local') {
        const updated = await updateSession(this.iii, sessionId, (current) => {
          assertSameAgentCreation(record, current);
          if (current.agent_created) return null;
          return { ...current, agent_created: true, updated_at_ms: Date.now() };
        });
        if (!updated) throw new SessionConflictError('Cursor session mapping disappeared');
        record = updated;
      }
    } else {
      await this.resumeAgent(client, record);
    }

    const accumulator = new RunAccumulator(sessionId, record.model, this.emit);
    if (record.active_run_id) {
      handle.runId = record.active_run_id;
      if (record.cancel_requested) {
        await requestCancellation(client, record.agent_id, record.active_run_id);
      }
      await this.observeExisting(client, record, handle, accumulator);
    } else {
      const dispatched = await this.sendAndRecover(client, record, handle, accumulator, prompt);
      if (!dispatched) {
        return this.finalizeUnpromptedCancellation(record, handle, record.agent_created);
      }
    }

    if (!accumulator.runId) {
      throw new Error('Cursor sdk.v1 run reached terminal handling without a run id');
    }
    if (record.runtime === 'cloud') {
      const usage = await this.fetchUsage(client, record, accumulator.runId).catch(() => null);
      if (usage) {
        const currentRun = usage.runs.find((run) => run.run_id === accumulator.runId);
        accumulator.usage = currentRun?.usage ?? usage.usage ?? accumulator.usage;
        record.cost = currentRun?.cost ?? usage.cost;
      }
    }
    let alreadyFinalized = false;
    const terminalRecord = await updateSession(this.iii, sessionId, (current) => {
      if (
        current.agent_id !== record.agent_id ||
        (current.active_run_id === accumulator.runId && current.claim_id !== handle.claimId)
      ) {
        throw new SessionConflictError('Cursor terminal state no longer owns the recorded run');
      }
      if (current.last_run_id === accumulator.runId && !current.active_run_id) {
        alreadyFinalized = true;
        return null;
      }
      if (current.active_run_id !== accumulator.runId) {
        throw new SessionConflictError('Cursor terminal state no longer owns the recorded run');
      }
      return {
        ...current,
        status: terminalRecordStatus(accumulator.status),
        turns: Math.max(current.turns, current.active_turn ?? current.turns + 1),
        active_turn: null,
        active_run_id: null,
        last_run_id: accumulator.runId,
        send_idempotency_key: null,
        send_started: false,
        cancel_requested: false,
        claim_id: null,
        claim_started_at_ms: null,
        pending_prompt_sha256: null,
        model: accumulator.resolvedModel || current.model,
        usage: accumulator.usage ?? current.usage,
        cost: record.cost ?? current.cost,
        updated_at_ms: Date.now(),
      };
    });
    if (!terminalRecord) throw new SessionConflictError('Cursor session mapping disappeared');
    record = terminalRecord;
    if (alreadyFinalized) {
      releaseEmitterSequence(sessionId);
      const stopReason =
        record.status === 'done' ? 'end' : record.status === 'cancelled' ? 'aborted' : 'error';
      return {
        session_id: sessionId,
        agent_id: record.agent_id,
        run_id: accumulator.runId,
        result: accumulator.resultText,
        status: normalizeEnum(accumulator.status),
        stop_reason: stopReason,
        is_error: stopReason !== 'end',
        usage: record.usage,
        cost: record.cost,
        busy: false,
        recovery_required: false,
        error: stopReason === 'end' ? null : (accumulator.errorMessage ?? accumulator.errorCode),
        error_details: null,
      };
    }
    let terminal: Awaited<ReturnType<RunAccumulator['finalize']>>;
    try {
      terminal = await accumulator.finalize();
    } finally {
      releaseEmitterSequence(sessionId);
    }
    return {
      session_id: sessionId,
      agent_id: record.agent_id,
      run_id: accumulator.runId,
      result: accumulator.resultText,
      status: normalizeEnum(accumulator.status),
      stop_reason: terminal.message.stop_reason,
      is_error: terminal.message.stop_reason !== 'end',
      usage: record.usage,
      cost: record.cost,
      busy: false,
      recovery_required: false,
      error: terminal.message.error_message ?? null,
      error_details: null,
    };
  }

  private async claimTurn(
    initial: SessionRecord,
    payload: RunPayload,
    promptHash: string,
    handle: LiveRun,
    config: Config,
  ): Promise<SessionRecord | RunResponse> {
    const record = initial;
    validateExistingPayload(record, payload);
    if (record.active_run_id) {
      if (record.claim_id === handle.claimId) return record;
      if (record.claim_id && !claimIsStale(record, config)) {
        return this.response(record.session_id, record, {
          status: 'working',
          busy: true,
          error: 'another Cursor worker owns this active run',
        });
      }
      if (sessionBackend(record) === 'cli-acp') {
        const recovery = {
          ...record,
          status: 'recovery-required' as const,
          claim_id: null,
          claim_started_at_ms: null,
          updated_at_ms: Date.now(),
        };
        const swapped = await compareAndSetSession(this.iii, record, recovery);
        if (!swapped.swapped) {
          if (!swapped.current)
            throw new SessionConflictError('Cursor session mapping disappeared');
          return this.response(record.session_id, swapped.current, {
            status: swapped.current.status,
            busy: true,
            error: 'Cursor session changed while resolving a stale CLI run',
          });
        }
        throw new LocalRecoveryRequiredError(
          'the Cursor CLI process ended while a local ACP prompt may still have been active',
        );
      }
      const takeover = {
        ...record,
        claim_id: handle.claimId,
        claim_started_at_ms: Date.now(),
        updated_at_ms: Date.now(),
      };
      const swapped = await compareAndSetSession(this.iii, record, takeover);
      if (swapped.swapped) return takeover;
      if (!swapped.current) throw new SessionConflictError('Cursor session mapping disappeared');
      return this.response(record.session_id, swapped.current, {
        status: swapped.current.status,
        busy: true,
        error: 'another Cursor worker reclaimed this active run',
      });
    }
    if (record.status === 'recovery-required') {
      return this.response(record.session_id, record, {
        status: record.status,
        recoveryRequired: true,
        error: recoveryGuidance(record),
      });
    }
    if (record.send_idempotency_key) {
      if (record.claim_id === handle.claimId) return record;
      if (record.send_started && !claimIsStale(record, config)) {
        return this.response(record.session_id, record, {
          status: 'working',
          busy: true,
          error: 'another Cursor worker owns this pending turn',
        });
      }
      if (record.runtime === 'local' && record.send_started) {
        const recovery = {
          ...record,
          status: 'recovery-required' as const,
          claim_id: null,
          claim_started_at_ms: null,
          updated_at_ms: Date.now(),
        };
        const swapped = await compareAndSetSession(this.iii, record, recovery);
        if (!swapped.swapped) {
          if (!swapped.current)
            throw new SessionConflictError('Cursor session mapping disappeared');
          return this.response(record.session_id, swapped.current, {
            status: swapped.current.status,
            busy: true,
            error: 'Cursor session changed while resolving a stale local claim',
          });
        }
        throw new LocalRecoveryRequiredError(
          'local Send acceptance is ambiguous and sdk.v1 does not guarantee local Send idempotency',
        );
      }
      const takeover = {
        ...record,
        status: 'working' as const,
        claim_id: handle.claimId,
        claim_started_at_ms: Date.now(),
        updated_at_ms: Date.now(),
      };
      const swapped = await compareAndSetSession(this.iii, record, takeover);
      if (swapped.swapped) return takeover;
      if (!swapped.current) throw new SessionConflictError('Cursor session mapping disappeared');
      return this.response(record.session_id, swapped.current, {
        status: swapped.current.status,
        busy: true,
        error: 'another Cursor worker reclaimed this pending turn',
      });
    }
    const activeTurn = record.turns + 1;
    const claimed = {
      ...record,
      model: payload.model ?? record.model,
      status: 'working' as const,
      active_turn: activeTurn,
      send_idempotency_key: randomIdempotencyKey('send'),
      send_started: false,
      cancel_requested: false,
      claim_id: handle.claimId,
      claim_started_at_ms: Date.now(),
      pending_prompt_sha256: promptHash,
      usage: null,
      cost: null,
      updated_at_ms: Date.now(),
    };
    const swapped = await compareAndSetSession(this.iii, record, claimed);
    if (swapped.swapped) return claimed;
    if (!swapped.current) throw new SessionConflictError('Cursor session mapping disappeared');
    return this.response(record.session_id, swapped.current, {
      status: swapped.current.status,
      busy: true,
      error: 'another Cursor worker claimed this turn',
    });
  }

  private async createOrResumeAgent(client: BridgeClient, record: SessionRecord): Promise<void> {
    try {
      const response = await client.unary(
        'SdkAgentService',
        'CreateAgent',
        {
          options: agentOptions(record, this.getConfig(), record.name ?? undefined),
          ...(record.runtime === 'cloud' ? { idempotencyKey: record.create_idempotency_key } : {}),
        },
        CreateAgentResponseWireSchema,
      );
      if (response.agentId !== record.agent_id) {
        throw new Error('Cursor Bridge returned a different agent id than requested');
      }
    } catch (error) {
      if (!(error instanceof BridgeRpcError) || error.code !== 'already_exists') throw error;
      await this.resumeAgent(client, record);
    }
  }

  private async resumeAgent(client: BridgeClient, record: SessionRecord): Promise<void> {
    const response = await client.unary(
      'SdkAgentService',
      'ResumeAgent',
      { agentId: record.agent_id, options: agentOptions(record, this.getConfig()) },
      CreateAgentResponseWireSchema,
    );
    if (response.agentId !== record.agent_id) {
      throw new Error('Cursor Bridge resumed a different agent id than requested');
    }
  }

  private async runLocalAcp(
    config: Config,
    initial: SessionRecord,
    handle: LiveRun,
    prompt: string,
    timeoutMs: number | undefined,
  ): Promise<RunResponse> {
    if (initial.runtime !== 'local' || sessionBackend(initial) !== 'cli-acp') {
      throw new SessionConflictError('Cursor CLI ACP can only run CLI-backed local sessions');
    }
    let record = initial;
    const client = await this.cliFactory.create(cursorCliLaunchOptions(config, record.workspace));
    handle.cli = client;
    let acpModelId: string;
    let acpSessionId: string;
    if (!record.agent_created) {
      const session = await client.newSession(record.workspace);
      acpModelId = requireCursorAcpModelId(record.model, session.models);
      acpSessionId = session.sessionId;
    } else {
      const session = await client.loadSession(record.agent_id, record.workspace);
      acpModelId = requireCursorAcpModelId(record.model, session.models);
      acpSessionId = record.agent_id;
    }
    handle.agentId = acpSessionId;
    await client.setModel(acpSessionId, acpModelId);
    await client.setMode(acpSessionId, 'ask');

    const runId = `acp-${randomUUID()}`;
    const started = await updateSession(this.iii, record.session_id, (current) => {
      if (
        current.claim_id !== handle.claimId ||
        current.send_idempotency_key !== record.send_idempotency_key ||
        current.active_run_id
      ) {
        throw new SessionConflictError('Cursor ACP prompt claim is no longer owned');
      }
      if (record.agent_created && current.agent_id !== record.agent_id) {
        throw new SessionConflictError('Cursor ACP session identity changed before prompting');
      }
      if (!record.agent_created && current.agent_created) {
        throw new SessionConflictError('Cursor ACP session was created by another worker');
      }
      if (handle.cancelRequested || current.cancel_requested) {
        return cancelUnpromptedRecord(current, record.agent_created);
      }
      return {
        ...current,
        agent_id: acpSessionId,
        agent_created: true,
        backend: 'cli-acp',
        active_run_id: runId,
        send_started: true,
        updated_at_ms: Date.now(),
      };
    });
    if (!started) throw new SessionConflictError('Cursor session mapping disappeared');
    record = started;
    if (record.active_run_id !== runId) {
      return this.finalizeUnpromptedCancellation(record, handle, initial.agent_created);
    }
    handle.runId = runId;
    handle.sendStarted = true;

    if (handle.cancelRequested) {
      const cancelled = await updateSession(this.iii, record.session_id, (current) => {
        if (current.active_run_id !== runId || current.claim_id !== handle.claimId) {
          throw new SessionConflictError('Cursor ACP prompt claim is no longer owned');
        }
        return cancelUnpromptedRecord(current, initial.agent_created);
      });
      if (!cancelled) throw new SessionConflictError('Cursor session mapping disappeared');
      return this.finalizeUnpromptedCancellation(cancelled, handle, initial.agent_created);
    }

    const accumulator = new RunAccumulator(record.session_id, record.model, this.emit);
    let resultText = '';
    const occurrences = new Map<string, number>();
    const onUpdate = async (notification: CursorAcpSessionUpdate): Promise<void> => {
      if (notification.sessionId !== record.agent_id) {
        throw new SessionConflictError('Cursor ACP update belongs to an unexpected session');
      }
      await this.refreshClaim(record, handle);
      const deliveryKey = stableAcpItemId(record, notification, occurrences);
      await this.emitRaw(
        record.session_id,
        { method: 'session/update', params: notification },
        `${deliveryKey}-raw`,
      );
      const frame = acpUpdateFrame(notification.update);
      if (!frame) return;
      const delta = frame.interactionUpdate?.update.delta;
      if (frame.interactionUpdate?.type === 'text-delta' && typeof delta === 'string') {
        resultText += delta;
      }
      await accumulator.ingest(frame, true, deliveryKey);
    };

    let stop: Awaited<ReturnType<CursorCliClient['prompt']>>;
    let timedOut = false;
    const promptOperation = client.prompt(record.agent_id, prompt, onUpdate);
    try {
      stop = await this.waitWithClaimHeartbeat(record, handle, promptOperation, timeoutMs);
    } catch (error) {
      if (!(error instanceof CursorTurnTimeoutError)) throw error;
      timedOut = true;
      handle.cancelRequested = true;
      await this.cancelLocalPromptIfRequested(true, handle);
      try {
        stop = await this.waitWithClaimHeartbeat(
          record,
          handle,
          promptOperation,
          config.shutdown_timeout_ms,
        );
      } catch (settleError) {
        if (!(settleError instanceof CursorTurnTimeoutError)) throw settleError;
        throw new LocalRecoveryRequiredError(
          'Cursor ACP prompt timed out and did not confirm cancellation before shutdown',
        );
      }
    }
    const status = acpLifecycleStatus(stop);
    if (timedOut) {
      accumulator.errorMessage = 'Cursor ACP turn timed out and cancellation was confirmed';
    } else if (stop === 'max_tokens') {
      accumulator.errorMessage = 'Cursor stopped after reaching the model output limit';
    } else if (stop === 'max_turn_requests') {
      accumulator.errorMessage = 'Cursor stopped after reaching the ACP turn-request limit';
    } else if (stop === 'refusal') {
      accumulator.errorMessage = 'Cursor refused the prompt';
    }
    const terminalFrame: RunStreamMessageWire = {
      result: {
        agentId: record.agent_id,
        runId,
        status,
        result: {
          agentId: record.agent_id,
          runId,
          status,
          result: resultText,
          model: { id: record.model },
        },
      },
      done: { agentId: record.agent_id, runId },
    };
    const terminalKey = stableAcpItemId(
      record,
      { sessionId: record.agent_id, update: { sessionUpdate: 'terminal', stopReason: stop } },
      occurrences,
    );
    await this.emitRaw(record.session_id, terminalFrame, `${terminalKey}-raw`);
    await accumulator.ingest(terminalFrame, false, terminalKey);

    const terminalRecord = await updateSession(this.iii, record.session_id, (current) => {
      if (current.active_run_id !== runId || current.claim_id !== handle.claimId) {
        throw new SessionConflictError('Cursor ACP terminal state no longer owns the prompt');
      }
      return {
        ...current,
        status: terminalRecordStatus(status),
        turns: Math.max(current.turns, current.active_turn ?? current.turns + 1),
        active_turn: null,
        active_run_id: null,
        last_run_id: runId,
        send_idempotency_key: null,
        send_started: false,
        cancel_requested: false,
        claim_id: null,
        claim_started_at_ms: null,
        pending_prompt_sha256: null,
        backend: 'cli-acp',
        usage: null,
        cost: null,
        updated_at_ms: Date.now(),
      };
    });
    if (!terminalRecord) throw new SessionConflictError('Cursor session mapping disappeared');
    let terminal: Awaited<ReturnType<RunAccumulator['finalize']>>;
    try {
      terminal = await accumulator.finalize();
    } finally {
      releaseEmitterSequence(record.session_id);
    }
    return {
      session_id: terminalRecord.session_id,
      agent_id: terminalRecord.agent_id,
      run_id: runId,
      result: resultText,
      status: normalizeEnum(status),
      stop_reason: terminal.message.stop_reason,
      is_error: terminal.message.stop_reason !== 'end',
      usage: null,
      cost: null,
      busy: false,
      recovery_required: false,
      error: terminal.message.error_message ?? null,
      error_details: null,
    };
  }

  private async finalizeUnpromptedCancellation(
    record: SessionRecord,
    handle: LiveRun,
    preserveAgent: boolean,
  ): Promise<RunResponse> {
    const accumulator = new RunAccumulator(record.session_id, record.model, this.emit);
    accumulator.status = 'RUN_LIFECYCLE_STATUS_CANCELLED';
    accumulator.runId = `cancelled-${hash(`${record.session_id}:${handle.claimId}`).slice(0, 32)}`;
    try {
      await accumulator.finalize();
    } finally {
      releaseEmitterSequence(record.session_id);
    }
    return unpromptedCancellationResponse(record, preserveAgent);
  }

  private async sendAndRecover(
    client: BridgeClient,
    record: SessionRecord,
    handle: LiveRun,
    accumulator: RunAccumulator,
    prompt: string,
  ): Promise<boolean> {
    const started = await updateSession(this.iii, record.session_id, (current) => {
      if (
        current.claim_id !== handle.claimId ||
        current.send_idempotency_key !== record.send_idempotency_key
      ) {
        throw new SessionConflictError('Cursor Send claim is no longer owned by this worker');
      }
      if (current.send_started) return null;
      if (handle.cancelRequested || current.cancel_requested) {
        return cancelUnpromptedRecord(current, current.agent_created);
      }
      return { ...current, send_started: true, updated_at_ms: Date.now() };
    });
    if (!started) throw new SessionConflictError('Cursor session mapping disappeared');
    Object.assign(record, started);
    if (!record.send_started) return false;
    handle.sendStarted = true;
    const request = {
      agentId: record.agent_id,
      message: { text: prompt },
      options: { enableDeltas: true, enableSteps: false },
      ...(record.runtime === 'cloud' && record.send_idempotency_key
        ? { idempotencyKey: record.send_idempotency_key }
        : {}),
    };
    let attempt = 0;
    while (attempt < 2) {
      attempt += 1;
      try {
        const stream = client.stream(
          'SdkAgentService',
          'Send',
          request,
          RunStreamMessageWireSchema,
        );
        await this.consumeSendUntilRunId(stream, record, handle);
      } catch (error) {
        if (record.active_run_id) {
          await this.observeExisting(client, record, handle, accumulator);
          return true;
        }
        if (record.runtime === 'local') {
          const recovery = await updateSession(this.iii, record.session_id, (current) => {
            if (current.active_run_id) return null;
            if (current.claim_id !== handle.claimId) {
              throw new SessionConflictError('Cursor local Send claim changed during recovery');
            }
            return {
              ...current,
              status: 'recovery-required',
              claim_id: null,
              claim_started_at_ms: null,
              updated_at_ms: Date.now(),
            };
          });
          if (recovery) Object.assign(record, recovery);
          throw new LocalRecoveryRequiredError(
            `local Send ended before a run id was observed: ${safeError(error)}`,
          );
        }
        if (attempt >= 2 || !isRetryableSendError(error)) throw error;
        await retryDelay(error, () => this.refreshClaim(record, handle));
        continue;
      }
      await this.observeExisting(client, record, handle, accumulator);
      return true;
    }
    return true;
  }

  private async observeExisting(
    client: BridgeClient,
    record: SessionRecord,
    handle: LiveRun,
    accumulator: RunAccumulator,
  ): Promise<void> {
    const runId = record.active_run_id;
    if (!runId) throw new Error('Cursor run recovery needs an active run id');
    await client.unary(
      'SdkAgentService',
      'GetRun',
      { runId, options: runOperationOptions(record, this.getConfig()) },
      GetRunResponseWireSchema,
    );
    try {
      const stream = client.stream(
        'SdkAgentService',
        'ObserveRun',
        { runId },
        RunStreamMessageWireSchema,
      );
      await this.consume(stream, record, handle, accumulator);
    } catch (error) {
      if (!(error instanceof BridgeTransportError || error instanceof BridgeRpcError)) throw error;
      const waited = await this.waitLiveRun(client, record, handle, runId);
      const frame: RunStreamMessageWire = {
        result: {
          agentId: waited.result.agentId,
          runId: waited.result.runId,
          status: waited.result.status,
          result: waited.result,
        },
      };
      const deliveryKey = stableFrameItemId(record, frame, new Map());
      await this.emitRaw(record.session_id, frame, `${deliveryKey}-raw`);
      await accumulator.ingest(frame, false, deliveryKey);
    }
  }

  private async consumeSendUntilRunId(
    stream: AsyncIterable<RunStreamMessageWire>,
    record: SessionRecord,
    handle: LiveRun,
  ): Promise<void> {
    for await (const frame of stream) {
      await this.refreshClaim(record, handle);
      const runId = extractRunId(frame);
      if (!runId) continue;
      const updated = await updateSession(this.iii, record.session_id, (current) => {
        if (
          current.claim_id !== handle.claimId ||
          current.send_idempotency_key !== record.send_idempotency_key
        ) {
          throw new SessionConflictError('Cursor Send claim is no longer owned by this worker');
        }
        if (current.active_run_id === runId) return null;
        if (current.active_run_id) {
          throw new SessionConflictError('Cursor run id conflicts with durable session state');
        }
        return {
          ...current,
          active_run_id: runId,
          agent_created: current.runtime === 'cloud' ? true : current.agent_created,
          updated_at_ms: Date.now(),
        };
      });
      if (!updated) throw new SessionConflictError('Cursor session mapping disappeared');
      Object.assign(record, updated);
      handle.runId = runId;
      if ((handle.cancelRequested || updated.cancel_requested) && handle.client) {
        await requestCancellation(handle.client, record.agent_id, runId);
      }
      return;
    }
    throw new BridgeTransportError('Cursor Send stream ended before reporting a run id');
  }

  private async consume(
    stream: AsyncIterable<RunStreamMessageWire>,
    record: SessionRecord,
    handle: LiveRun,
    accumulator: RunAccumulator,
  ): Promise<void> {
    let resultSeen = false;
    let doneSeen = false;
    const occurrences = new Map<string, number>();
    for await (const frame of stream) {
      await this.refreshClaim(record, handle);
      const frameRunId = extractRunId(frame);
      if (frameRunId && frameRunId !== record.active_run_id) {
        throw new SessionConflictError('Cursor ObserveRun returned an unexpected run id');
      }
      const deliveryKey = stableFrameItemId(record, frame, occurrences);
      await this.emitRaw(record.session_id, frame, `${deliveryKey}-raw`);
      await accumulator.ingest(frame, true, deliveryKey);
      if (frame.result) resultSeen = true;
      if (frame.done) doneSeen = true;
    }
    if (!resultSeen || !doneSeen) {
      throw new BridgeTransportError('Cursor run stream ended before result and done');
    }
  }

  private async refreshClaim(record: SessionRecord, handle: LiveRun): Promise<void> {
    const now = Date.now();
    if (now - handle.leaseRefreshedAtMs < 30_000) return;
    const updated = await updateSession(this.iii, record.session_id, (current) => {
      if (current.claim_id !== handle.claimId) {
        throw new SessionConflictError('Cursor run lease is no longer owned by this worker');
      }
      if (record.active_run_id && current.active_run_id !== record.active_run_id) {
        throw new SessionConflictError('Cursor active run changed while refreshing its lease');
      }
      return { ...current, claim_started_at_ms: now, updated_at_ms: now };
    });
    if (!updated) throw new SessionConflictError('Cursor session mapping disappeared');
    Object.assign(record, updated);
    handle.leaseRefreshedAtMs = now;
    await this.cancelLocalPromptIfRequested(updated.cancel_requested, handle);
  }

  private async cancelLocalPromptIfRequested(
    durableCancellationRequested: boolean,
    handle: LiveRun,
  ): Promise<void> {
    if (
      !handle.cli ||
      !handle.agentId ||
      handle.cancelSent ||
      (!handle.cancelRequested && !durableCancellationRequested)
    ) {
      return;
    }
    handle.cancelSent = true;
    await handle.cli.cancel(handle.agentId);
  }

  private async waitLiveRun(
    client: BridgeClient,
    record: SessionRecord,
    handle: LiveRun,
    runId: string,
  ): Promise<z.infer<typeof WaitLiveRunResponseWireSchema>> {
    return this.waitWithClaimHeartbeat(
      record,
      handle,
      client.unary('SdkAgentService', 'WaitLiveRun', { runId }, WaitLiveRunResponseWireSchema, {
        timeoutMs: 24 * 60 * 60 * 1_000,
      }),
    );
  }

  private async waitWithClaimHeartbeat<T>(
    record: SessionRecord,
    handle: LiveRun,
    operation: Promise<T>,
    timeoutMs?: number,
  ): Promise<T> {
    const deadlineMs = timeoutMs === undefined ? null : Date.now() + timeoutMs;
    const completion = operation.then(
      (value) => ({ kind: 'value' as const, value }),
      (error: unknown) => ({ kind: 'error' as const, error }),
    );
    while (true) {
      const remainingMs = deadlineMs === null ? 30_000 : deadlineMs - Date.now();
      if (remainingMs <= 0) throw new CursorTurnTimeoutError();
      let timer: ReturnType<typeof setTimeout> | undefined;
      const tick = new Promise<{ kind: 'tick' }>((resolvePromise) => {
        timer = setTimeout(() => resolvePromise({ kind: 'tick' }), Math.min(30_000, remainingMs));
      });
      const outcome = await Promise.race([completion, tick]);
      if (timer) clearTimeout(timer);
      if (outcome.kind === 'value') return outcome.value;
      if (outcome.kind === 'error') throw outcome.error;
      await this.refreshClaim(record, handle);
    }
  }

  private async stop(sessionId: string): Promise<z.infer<typeof StopResponseSchema>> {
    const live = this.live.get(sessionId);
    if (live) {
      let recordFound = false;
      let locallyOwned = false;
      await updateSession(this.iii, sessionId, (current) => {
        recordFound = true;
        locallyOwned = false;
        if (current.claim_id !== live.claimId) return null;
        if (live.runId && current.active_run_id !== live.runId) return null;
        locallyOwned = true;
        if (!current.active_run_id && !current.send_idempotency_key && !current.send_started) {
          return null;
        }
        if (current.cancel_requested) return null;
        return { ...current, cancel_requested: true, updated_at_ms: Date.now() };
      });
      if (!recordFound || locallyOwned) {
        live.cancelRequested = true;
        if (live.cli && live.agentId) {
          await this.cancelLocalPromptIfRequested(true, live);
        } else if (live.client && live.agentId && live.runId) {
          await requestCancellation(live.client, live.agentId, live.runId);
        }
        return { session_id: sessionId, stopped: true, reason: null };
      }
    }
    let record = await loadSession(this.iii, sessionId);
    if (!record) {
      return { session_id: sessionId, stopped: false, reason: 'no cancellable run id' };
    }
    if (sessionBackend(record) === 'cli-acp') {
      if (!record.active_run_id && !record.send_started && !record.send_idempotency_key) {
        return { session_id: sessionId, stopped: false, reason: 'no cancellable run id' };
      }
      const config = this.getConfig();
      if (record.claim_id && !claimIsStale(record, config)) {
        const expectedClaimId = record.claim_id;
        const expectedSendId = record.send_idempotency_key;
        const expectedRunId = record.active_run_id;
        const expectedPromptHash = record.pending_prompt_sha256;
        let cancellationRequested = false;
        await updateSession(this.iii, sessionId, (current) => {
          cancellationRequested = false;
          if (
            sessionBackend(current) !== 'cli-acp' ||
            current.status !== 'working' ||
            current.claim_id !== expectedClaimId ||
            current.send_idempotency_key !== expectedSendId ||
            current.active_run_id !== expectedRunId ||
            current.pending_prompt_sha256 !== expectedPromptHash
          ) {
            return null;
          }
          cancellationRequested = true;
          if (current.cancel_requested) return null;
          return { ...current, cancel_requested: true, updated_at_ms: Date.now() };
        });
        return {
          session_id: sessionId,
          stopped: cancellationRequested,
          reason: cancellationRequested
            ? null
            : 'Cursor session changed before its active prompt was asked to stop',
        };
      }
      if (!record.active_run_id && !record.send_started) {
        const pendingSendId = record.send_idempotency_key;
        let clearedPending = false;
        const cleared = await updateSession(this.iii, sessionId, (current) => {
          clearedPending = false;
          if (
            sessionBackend(current) !== 'cli-acp' ||
            current.status !== 'working' ||
            current.active_run_id ||
            current.send_started ||
            !current.send_idempotency_key ||
            current.send_idempotency_key !== pendingSendId ||
            (current.claim_id && !claimIsStale(current, config))
          ) {
            return null;
          }
          clearedPending = true;
          return {
            ...current,
            status: 'cancelled',
            active_turn: null,
            send_idempotency_key: null,
            send_started: false,
            cancel_requested: false,
            claim_id: null,
            claim_started_at_ms: null,
            pending_prompt_sha256: null,
            updated_at_ms: Date.now(),
          };
        });
        const stopped = Boolean(
          clearedPending &&
            cleared &&
            cleared.status === 'cancelled' &&
            !cleared.active_run_id &&
            !cleared.send_idempotency_key,
        );
        return {
          session_id: sessionId,
          stopped,
          reason: stopped ? null : 'Cursor session changed before its pending prompt was stopped',
        };
      }
      let markedRecovery = false;
      const recovery = await updateSession(this.iii, sessionId, (current) => {
        markedRecovery = false;
        if (
          sessionBackend(current) !== 'cli-acp' ||
          current.status !== 'working' ||
          (!current.active_run_id && !current.send_started) ||
          (current.claim_id && !claimIsStale(current, config))
        ) {
          return null;
        }
        markedRecovery = true;
        return {
          ...current,
          status: 'recovery-required',
          cancel_requested: false,
          claim_id: null,
          claim_started_at_ms: null,
          updated_at_ms: Date.now(),
        };
      });
      return {
        session_id: sessionId,
        stopped: false,
        reason:
          markedRecovery && recovery
            ? 'the Cursor ACP process is unavailable; start a new session to avoid replaying work'
            : 'Cursor session changed before its active prompt was marked for recovery',
      };
    }
    if (
      record.runtime === 'local' &&
      sessionBackend(record) === 'sdk-bridge' &&
      record.status === 'recovery-required' &&
      !record.active_run_id
    ) {
      return {
        session_id: sessionId,
        stopped: false,
        reason: recoveryGuidance(record),
      };
    }
    if (!record.active_run_id) {
      if (!record.send_idempotency_key && !record.send_started) {
        return { session_id: sessionId, stopped: false, reason: 'no cancellable run id' };
      }
      const marked = await updateSession(this.iii, sessionId, (current) => {
        if (current.active_run_id || current.cancel_requested) return null;
        if (!current.send_idempotency_key && !current.send_started) return null;
        return { ...current, cancel_requested: true, updated_at_ms: Date.now() };
      });
      if (!marked?.active_run_id) {
        return { session_id: sessionId, stopped: true, reason: null };
      }
      record = marked;
    }
    const runId = record.active_run_id;
    if (!runId) {
      return { session_id: sessionId, stopped: true, reason: null };
    }
    const config = this.getConfig();
    if (record.claim_id && !claimIsStale(record, config)) {
      const expectedClaimId = record.claim_id;
      let sameRun = false;
      const marked = await updateSession(this.iii, sessionId, (current) => {
        sameRun = false;
        if (current.active_run_id !== runId || current.claim_id !== expectedClaimId) return null;
        sameRun = true;
        if (current.cancel_requested) return null;
        return { ...current, cancel_requested: true, updated_at_ms: Date.now() };
      });
      if (!sameRun || !marked || marked.active_run_id !== runId) {
        return { session_id: sessionId, stopped: false, reason: 'active run changed' };
      }
      const client = this.factory.create(
        bridgeLaunchOptions(config, marked.runtime === 'local' ? marked.workspace : undefined),
      );
      try {
        await requestCancellation(client, marked.agent_id, runId);
        return { session_id: sessionId, stopped: true, reason: null };
      } finally {
        await client.close();
      }
    }
    const claimId = randomUUID();
    const claimed = await updateSession(this.iii, sessionId, (current) => {
      if (current.active_run_id !== runId) return null;
      if (current.claim_id && !claimIsStale(current, config)) return null;
      const now = Date.now();
      return {
        ...current,
        cancel_requested: true,
        claim_id: claimId,
        claim_started_at_ms: now,
        updated_at_ms: now,
      };
    });
    if (!claimed || claimed.active_run_id !== runId || claimed.claim_id !== claimId) {
      return { session_id: sessionId, stopped: false, reason: 'active run changed' };
    }
    const client = this.factory.create(
      bridgeLaunchOptions(
        this.getConfig(),
        claimed.runtime === 'local' ? claimed.workspace : undefined,
      ),
    );
    const handle: LiveRun = {
      client,
      cli: null,
      agentId: claimed.agent_id,
      runId,
      cancelRequested: true,
      cancelSent: false,
      claimId,
      sendStarted: claimed.send_started,
      leaseRefreshedAtMs: claimed.claim_started_at_ms ?? Date.now(),
    };
    try {
      const snapshot = await client.unary(
        'SdkAgentService',
        'GetRun',
        { runId, options: runOperationOptions(claimed, this.getConfig()) },
        GetRunResponseWireSchema,
      );
      const terminalResult = isTerminalStatus(snapshot.run.status)
        ? snapshot.run
        : await (async () => {
            await requestCancellation(client, claimed.agent_id, runId);
            return (await this.waitLiveRun(client, claimed, handle, runId)).result;
          })();
      const frame: RunStreamMessageWire = {
        result: {
          agentId: terminalResult.agentId,
          runId: terminalResult.runId,
          status: terminalResult.status,
          result: terminalResult,
        },
      };
      const accumulator = new RunAccumulator(sessionId, claimed.model, this.emit);
      const deliveryKey = stableFrameItemId(claimed, frame, new Map());
      await this.emitRaw(sessionId, frame, `${deliveryKey}-raw`);
      await accumulator.ingest(frame, false, deliveryKey);
      const normalized = normalizeEnum(accumulator.status);
      if (!['FINISHED', 'CANCELLED', 'ERROR', 'EXPIRED'].includes(normalized)) {
        throw new Error(`Cursor cancelled run remained non-terminal: ${normalized}`);
      }
      let alreadyFinalized = false;
      const terminal = await updateSession(this.iii, sessionId, (current) => {
        if (current.last_run_id === runId && !current.active_run_id) {
          alreadyFinalized = true;
          return null;
        }
        if (current.active_run_id !== runId || current.claim_id !== claimId) {
          throw new SessionConflictError('Cursor cancellation no longer owns the active run');
        }
        return {
          ...current,
          status: terminalRecordStatus(accumulator.status),
          turns: Math.max(current.turns, current.active_turn ?? current.turns + 1),
          active_turn: null,
          active_run_id: null,
          last_run_id: runId,
          send_idempotency_key: null,
          send_started: false,
          cancel_requested: false,
          claim_id: null,
          claim_started_at_ms: null,
          pending_prompt_sha256: null,
          model: accumulator.resolvedModel || current.model,
          usage: accumulator.usage ?? current.usage,
          updated_at_ms: Date.now(),
        };
      });
      if (!terminal) throw new SessionConflictError('Cursor session mapping disappeared');
      if (!alreadyFinalized) {
        try {
          await accumulator.finalize();
        } finally {
          releaseEmitterSequence(sessionId);
        }
      } else {
        releaseEmitterSequence(sessionId);
      }
      return { session_id: sessionId, stopped: true, reason: null };
    } catch (error) {
      await updateSession(this.iii, sessionId, (current) => {
        if (current.claim_id !== claimId) return null;
        return {
          ...current,
          claim_id: null,
          claim_started_at_ms: null,
          updated_at_ms: Date.now(),
        };
      }).catch(() => undefined);
      throw error;
    } finally {
      await client.close();
    }
  }

  private async status(sessionId: string): Promise<z.infer<typeof StatusResponseSchema>> {
    const record = await loadSession(this.iii, sessionId);
    if (!record) {
      return {
        session_id: sessionId,
        live: this.live.has(sessionId),
        record: null,
        agent: null,
        run: null,
        remote_error: null,
      };
    }
    if (sessionBackend(record) === 'cli-acp') {
      return {
        session_id: sessionId,
        live: this.live.has(sessionId),
        record,
        agent: {
          agent_id: record.agent_id,
          name: record.name ?? '',
          summary: '',
          status: record.status,
          archived: false,
          created_at: null,
          last_modified: null,
          runtime: 'local',
          cwd: record.workspace,
          repositories: [],
          metadata: { backend: 'cli-acp' },
        },
        run: null,
        remote_error: null,
      };
    }
    const client = this.factory.create(
      bridgeLaunchOptions(
        this.getConfig(),
        record.runtime === 'local' ? record.workspace : undefined,
      ),
    );
    try {
      const agentResponse = await client.unary(
        'SdkAgentService',
        'GetAgent',
        { agentId: record.agent_id, options: operationOptions(record, this.getConfig()) },
        GetAgentResponseWireSchema,
      );
      const runId = record.active_run_id ?? record.last_run_id;
      const runResponse = runId
        ? await client.unary(
            'SdkAgentService',
            'GetRun',
            {
              runId,
              options: runOperationOptions(record, this.getConfig()),
            },
            GetRunResponseWireSchema,
          )
        : null;
      return {
        session_id: sessionId,
        live: this.live.has(sessionId),
        record,
        agent: mapAgentInfo(agentResponse.agent),
        run: runResponse ? mapRunSnapshot(runResponse.run) : null,
        remote_error: null,
      };
    } catch (error) {
      return {
        session_id: sessionId,
        live: this.live.has(sessionId),
        record,
        agent: null,
        run: null,
        remote_error: safeError(error),
      };
    } finally {
      await client.close();
    }
  }

  private async listModels(
    request: z.infer<typeof ModelsRequestSchema>,
  ): Promise<z.infer<typeof ModelsResponseSchema>> {
    const config = this.getConfig();
    const backend = request.backend ?? config.local_backend;
    if (backend === 'cli-acp') {
      return { models: await this.cliFactory.listModels(cursorCliLaunchOptions(config)) };
    }
    const options = bridgeLaunchOptions(config);
    const client = this.factory.create(options);
    try {
      const response = await client.unary(
        'SdkCursorService',
        'ListModels',
        { options: { apiKey: options.apiKey } },
        ListModelsResponseWireSchema,
      );
      return { models: mapModels(response) };
    } finally {
      await client.close();
    }
  }

  private async authStatus(): Promise<z.infer<typeof AuthStatusResponseSchema>> {
    try {
      const status = await this.cliFactory.authStatus(cursorCliLaunchOptions(this.getConfig()));
      return { available: true, ...status, error: null };
    } catch (error) {
      return {
        available: false,
        authenticated: false,
        status: 'unavailable',
        version: null,
        login_command: 'cursor-agent login',
        error: safeError(error),
      };
    }
  }

  private async usage(
    request: z.infer<typeof UsageRequestSchema>,
  ): Promise<z.infer<typeof UsageResponseSchema>> {
    const record = await loadSession(this.iii, request.session_id);
    if (!record) throw new Error(`Cursor session ${request.session_id} was not found`);
    if (record.runtime !== 'cloud')
      throw new Error('Cursor usage is available for cloud agents only');
    const client = this.factory.create(bridgeLaunchOptions(this.getConfig()));
    try {
      await this.resumeAgent(client, record);
      const usage = await this.fetchUsage(client, record, request.run_id);
      if (request.run_id) {
        return {
          session_id: record.session_id,
          agent_id: record.agent_id,
          usage,
        };
      }
      const updated = await updateSession(this.iii, record.session_id, (current) => ({
        ...current,
        usage: usage.usage ?? current.usage,
        cost: usage.cost ?? current.cost,
        updated_at_ms: Date.now(),
      }));
      if (!updated) throw new SessionConflictError('Cursor session mapping disappeared');
      return {
        session_id: updated.session_id,
        agent_id: updated.agent_id,
        usage: { ...usage, usage: updated.usage, cost: updated.cost },
      };
    } finally {
      await client.close();
    }
  }

  private async fetchUsage(client: BridgeClient, record: SessionRecord, runId?: string) {
    const response = await client.unary(
      'SdkAgentService',
      'GetUsage',
      { agentId: record.agent_id, ...(runId ? { runId } : {}) },
      GetUsageResponseWireSchema,
    );
    return mapUsageResponse(response);
  }

  private async failRun(sessionId: string, error: unknown, handle?: LiveRun): Promise<RunResponse> {
    let record = await loadSession(this.iii, sessionId).catch(() => null);
    const localAmbiguous = Boolean(
      record?.runtime === 'local' &&
        (sessionBackend(record) === 'cli-acp' || !record.active_run_id) &&
        (record.send_started || error instanceof LocalRecoveryRequiredError),
    );
    const recoveryRequired = error instanceof LocalRecoveryRequiredError || localAmbiguous;
    if (recoveryRequired && record?.runtime === 'local' && sessionBackend(record) === 'cli-acp') {
      const updated = await updateSession(this.iii, sessionId, (current) => {
        if (handle && current.claim_id && current.claim_id !== handle.claimId) return null;
        return {
          ...current,
          status: 'recovery-required',
          cancel_requested: false,
          claim_id: null,
          claim_started_at_ms: null,
          updated_at_ms: Date.now(),
        };
      }).catch(() => record);
      if (updated) record = updated;
      return this.response(sessionId, record, {
        status: 'recovery-required',
        recoveryRequired: true,
        error,
      });
    }
    if (
      record?.active_run_id &&
      handle &&
      !(error instanceof CursorPayloadError || error instanceof SessionConflictError)
    ) {
      const updated = await updateSession(this.iii, sessionId, (current) => {
        if (
          current.active_run_id !== record?.active_run_id ||
          current.claim_id !== handle.claimId
        ) {
          return null;
        }
        return {
          ...current,
          claim_id: null,
          claim_started_at_ms: null,
          updated_at_ms: Date.now(),
        };
      }).catch(() => record);
      if (updated) record = updated;
    }
    if (record?.active_run_id) {
      return this.response(sessionId, record, {
        status: 'error',
        error,
      });
    }
    if (
      record &&
      !record.active_run_id &&
      !(error instanceof CursorPayloadError || error instanceof SessionConflictError)
    ) {
      const updated = await updateSession(this.iii, sessionId, (current) => {
        if (current.active_run_id) return null;
        if (handle && current.claim_id && current.claim_id !== handle.claimId) return null;
        if (!current.send_started) {
          const resetUnpromptedCliSession =
            sessionBackend(current) === 'cli-acp' && current.turns === 0 && !current.last_run_id;
          return {
            ...current,
            status: 'error',
            agent_created: resetUnpromptedCliSession ? false : current.agent_created,
            active_turn: null,
            send_idempotency_key: null,
            send_started: false,
            cancel_requested: false,
            claim_id: null,
            claim_started_at_ms: null,
            pending_prompt_sha256: null,
            updated_at_ms: Date.now(),
          };
        }
        return {
          ...current,
          status: current.runtime === 'local' ? 'recovery-required' : 'error',
          claim_id: null,
          claim_started_at_ms: null,
          updated_at_ms: Date.now(),
        };
      }).catch(() => record);
      if (updated) record = updated;
    }
    if (error instanceof CursorPayloadError || error instanceof SessionConflictError) {
      return this.response(sessionId, record, {
        status: record?.status ?? 'error',
        busy: record?.status === 'working',
        error,
      });
    }
    const accumulator = new RunAccumulator(sessionId, record?.model ?? '', this.emit);
    accumulator.status = 'RUN_LIFECYCLE_STATUS_ERROR';
    accumulator.errorMessage = safeError(error);
    accumulator.resultText = safeError(error);
    try {
      await accumulator.finalize();
    } finally {
      releaseEmitterSequence(sessionId);
    }
    return this.response(sessionId, record, {
      status: recoveryRequired ? 'recovery-required' : 'error',
      recoveryRequired,
      error,
    });
  }

  private response(
    sessionId: string,
    record: SessionRecord | null,
    overrides: {
      status: string;
      busy?: boolean;
      recoveryRequired?: boolean;
      error?: unknown;
    },
  ): RunResponse {
    return {
      session_id: sessionId,
      agent_id: record?.agent_id ?? null,
      run_id: record?.active_run_id ?? record?.last_run_id ?? null,
      result: '',
      status: overrides.status,
      stop_reason: overrides.busy ? null : 'error',
      is_error: !overrides.busy,
      usage: record?.usage ?? null,
      cost: record?.cost ?? null,
      busy: overrides.busy ?? false,
      recovery_required: overrides.recoveryRequired ?? false,
      error: overrides.error == null ? null : safeError(overrides.error),
      error_details: errorDetails(overrides.error),
    };
  }
}

export function extractPrompt(payload: { prompt?: string; messages?: InputMessage[] }): string {
  let resolved: string;
  if (typeof payload.prompt === 'string') {
    resolved = payload.prompt;
  } else {
    const users = (payload.messages ?? []).filter((message) => message.role === 'user');
    const last = users.at(-1);
    if (!last) throw new CursorPayloadError('cursor::run requires prompt or a user message');
    resolved =
      typeof last.content === 'string'
        ? last.content
        : last.content
            .map((block) => (typeof block.text === 'string' ? block.text : ''))
            .filter(Boolean)
            .join('\n');
  }
  if (!resolved.trim()) {
    throw new CursorPayloadError('cursor::run requires a non-empty prompt or user message');
  }
  return resolved;
}

function parseRunRequest(payload: unknown): RunPayload {
  const parsed = CursorRunRequestSchema.parse(payload ?? {});
  if (parsed.runtime === 'local' || parsed.runtime === 'cloud') return parsed;
  const prompt = extractPrompt(parsed);
  return {
    runtime: 'local',
    session_id: parsed.session_id,
    cwd: parsed.cwd,
    model: parsed.model,
    prompt: parsed.system_prompt ? `${parsed.system_prompt}\n\n${prompt}` : prompt,
    timeout_ms: parsed.timeout_ms,
  };
}

function newSessionRecord(
  sessionId: string,
  payload: RunPayload,
  promptHash: string,
  claimId: string,
  backend: SessionBackend,
): SessionRecord {
  return {
    session_id: sessionId,
    agent_id: `${payload.runtime === 'local' ? 'agent' : 'bc'}-${randomUUID()}`,
    runtime: payload.runtime,
    backend,
    workspace: payload.runtime === 'local' ? resolve(payload.cwd) : '',
    name: payload.name ?? null,
    model: payload.model ?? '',
    tools: payload.runtime === 'local' ? (payload.tools ?? SAFE_LOCAL_TOOLS) : [],
    repositories: payload.runtime === 'cloud' ? (payload.repositories ?? []) : [],
    work_on_current_branch:
      payload.runtime === 'cloud' ? (payload.work_on_current_branch ?? false) : false,
    auto_create_pr: payload.runtime === 'cloud' ? (payload.auto_create_pr ?? false) : false,
    status: 'working',
    agent_created: false,
    turns: 0,
    active_turn: 1,
    active_run_id: null,
    last_run_id: null,
    create_idempotency_key: randomIdempotencyKey('create'),
    send_idempotency_key: randomIdempotencyKey('send'),
    send_started: false,
    cancel_requested: false,
    claim_id: claimId,
    claim_started_at_ms: Date.now(),
    pending_prompt_sha256: promptHash,
    usage: null,
    cost: null,
    updated_at_ms: Date.now(),
  };
}

function backendForPayload(payload: RunPayload, config: Config): SessionBackend {
  if (payload.runtime === 'cloud') return 'sdk-bridge';
  return config.local_backend;
}

function backendForRun(
  payload: RunPayload,
  config: Config,
  prior: SessionRecord | null,
): SessionBackend {
  return prior ? sessionBackend(prior) : backendForPayload(payload, config);
}

function cancelUnpromptedRecord(record: SessionRecord, preserveAgent: boolean): SessionRecord {
  return {
    ...record,
    status: 'cancelled',
    agent_created: preserveAgent && record.agent_created,
    active_turn: null,
    active_run_id: null,
    send_idempotency_key: null,
    send_started: false,
    cancel_requested: false,
    claim_id: null,
    claim_started_at_ms: null,
    pending_prompt_sha256: null,
    updated_at_ms: Date.now(),
  };
}

function unpromptedCancellationResponse(
  record: SessionRecord,
  preserveAgent: boolean,
): RunResponse {
  return {
    session_id: record.session_id,
    agent_id: preserveAgent ? record.agent_id : null,
    run_id: null,
    result: '',
    status: 'CANCELLED',
    stop_reason: 'aborted',
    is_error: true,
    usage: record.usage,
    cost: record.cost,
    busy: false,
    recovery_required: false,
    error: null,
    error_details: null,
  };
}

function sessionBackend(record: SessionRecord): SessionBackend {
  return record.backend ?? 'sdk-bridge';
}

function validateSessionBackend(record: SessionRecord, requested: SessionBackend): void {
  const existing = sessionBackend(record);
  if (existing === requested) return;
  if (record.runtime === 'local' && existing === 'sdk-bridge' && requested === 'cli-acp') {
    throw new CursorPayloadError(
      'this session was created by the legacy SDK Bridge local backend; start a new session for Cursor CLI login mode',
    );
  }
  throw new CursorPayloadError(`Cursor session backend is ${existing}, not ${requested}`);
}

function validateCliTools(payload: RunPayload, backend: SessionBackend): void {
  if (backend !== 'cli-acp' || payload.runtime !== 'local' || payload.tools === undefined) return;
  throw new CursorPayloadError(
    'Cursor CLI ACP does not support explicit tool lists; omit tools to use ask mode with permission requests denied',
  );
}

function validateBackendTimeout(payload: RunPayload, backend: SessionBackend): void {
  if (backend !== 'sdk-bridge' || payload.runtime !== 'local' || payload.timeout_ms === undefined) {
    return;
  }
  throw new CursorPayloadError(
    'timeout_ms is supported only by the login-backed cli-acp backend; omit it for sdk-bridge sessions',
  );
}

function validateNewSessionPayload(payload: RunPayload): void {
  if (payload.runtime === 'cloud' && (!payload.repositories || payload.repositories.length === 0)) {
    throw new CursorPayloadError('a new cloud Cursor session requires at least one repository');
  }
}

function requireCursorAcpModelId(
  requestedModel: string,
  availableModels: CursorAcpSession['models'],
): string {
  const modelId = resolveCursorAcpModelId(requestedModel, availableModels);
  if (modelId) return modelId;
  throw new CursorCliError(
    `Cursor model "${requestedModel}" is not available through login-backed ACP. Call cursor::models::list with {"backend":"cli-acp"} and use one of those IDs; cursor-agent --list-models can include CLI-only parameterized IDs that ACP rejects.`,
  );
}

function recoveryGuidance(record: SessionRecord): string {
  if (sessionBackend(record) === 'cli-acp') {
    return 'this local Cursor ACP session lost its process after prompt dispatch; start a new session to avoid duplicating work';
  }
  return 'this local session lost its Send stream before receiving a run id; start a new session to avoid duplicating work';
}

function validateExistingPayload(record: SessionRecord, payload: RunPayload): void {
  if (record.runtime !== payload.runtime) {
    throw new CursorPayloadError(
      `Cursor session runtime is ${record.runtime}, not ${payload.runtime}`,
    );
  }
  if (payload.runtime === 'local' && resolve(payload.cwd) !== record.workspace) {
    throw new CursorPayloadError('a local Cursor session must resume with its original cwd');
  }
  if (!record.agent_created) {
    if ((payload.name ?? null) !== record.name) {
      throw new CursorPayloadError('name must match the pending Cursor CreateAgent request');
    }
    if (payload.model !== undefined && payload.model !== record.model) {
      throw new CursorPayloadError('model must match the pending Cursor CreateAgent request');
    }
  } else if (
    payload.name &&
    !(record.turns === 0 && !record.last_run_id && payload.name === record.name)
  ) {
    throw new CursorPayloadError('name can only be set when a Cursor session is created');
  }
  if (
    record.send_idempotency_key &&
    payload.model !== undefined &&
    payload.model !== record.model
  ) {
    throw new CursorPayloadError('model must match the pending Cursor Send request');
  }
  if (
    record.send_idempotency_key &&
    record.pending_prompt_sha256 !== hash(extractPrompt(payload))
  ) {
    throw new CursorPayloadError('prompt must match the pending Cursor Send request');
  }
  if (
    payload.runtime === 'local' &&
    payload.tools &&
    JSON.stringify(payload.tools) !== JSON.stringify(record.tools)
  ) {
    throw new CursorPayloadError('local tools are fixed when the Cursor session is created');
  }
  if (payload.runtime === 'cloud') {
    if (payload.repositories && !sameRepositories(payload.repositories, record.repositories)) {
      throw new CursorPayloadError(
        'cloud repositories are fixed when the Cursor session is created',
      );
    }
    if (
      payload.work_on_current_branch !== undefined &&
      payload.work_on_current_branch !== record.work_on_current_branch
    ) {
      throw new CursorPayloadError(
        'work_on_current_branch is fixed when the Cursor session is created',
      );
    }
    if (payload.auto_create_pr !== undefined && payload.auto_create_pr !== record.auto_create_pr) {
      throw new CursorPayloadError('auto_create_pr is fixed when the Cursor session is created');
    }
  }
}

function agentOptions(
  record: SessionRecord,
  config: Config,
  name?: string,
): Record<string, unknown> {
  const apiKey = bridgeLaunchOptions(
    config,
    record.runtime === 'local' ? record.workspace : undefined,
  ).apiKey;
  return {
    ...(record.model ? { model: { id: record.model } } : {}),
    apiKey,
    agentId: record.agent_id,
    mode: 'AGENT_MODE_OPTION_AGENT',
    ...(name ? { name } : {}),
    ...(record.runtime === 'local'
      ? {
          tools: { names: record.tools },
          local: {
            cwd: [record.workspace],
            sandboxOptions: { enabled: true },
          },
        }
      : {
          cloud: {
            env: { type: 'CLOUD_ENVIRONMENT_TYPE_CLOUD' },
            repos: record.repositories.map((repository) => ({
              url: repository.url,
              ...(repository.starting_ref ? { startingRef: repository.starting_ref } : {}),
              ...(repository.pr_url ? { prUrl: repository.pr_url } : {}),
            })),
            workOnCurrentBranch: record.work_on_current_branch,
            autoCreatePr: record.auto_create_pr,
          },
        }),
  };
}

function operationOptions(record: SessionRecord, config: Config): Record<string, unknown> {
  const apiKey = bridgeLaunchOptions(
    config,
    record.runtime === 'local' ? record.workspace : undefined,
  ).apiKey;
  return {
    apiKey,
    ...(record.runtime === 'local' ? { cwd: record.workspace } : {}),
  };
}

function runOperationOptions(record: SessionRecord, config: Config): Record<string, unknown> {
  return {
    ...operationOptions(record, config),
    runtime: record.runtime === 'local' ? 'RUNTIME_LOCAL' : 'RUNTIME_CLOUD',
    agentId: record.agent_id,
  };
}

async function cancelRun(
  client: BridgeClient,
  agentId: string | null,
  runId: string,
): Promise<void> {
  await client.unary(
    'SdkAgentService',
    'CancelRun',
    { runId, ...(agentId ? { agentId } : {}) },
    EmptyResponseWireSchema,
  );
}

async function requestCancellation(
  client: BridgeClient,
  agentId: string | null,
  runId: string,
): Promise<void> {
  try {
    await cancelRun(client, agentId, runId);
  } catch (error) {
    if (
      !(error instanceof BridgeRpcError) ||
      error.detail?.sdk_error_code !== 'RUN_NOT_CANCELLABLE'
    ) {
      throw error;
    }
  }
}

function isTerminalStatus(status: string | number | undefined): boolean {
  return ['FINISHED', 'CANCELLED', 'ERROR', 'EXPIRED'].includes(normalizeEnum(status));
}

function terminalRecordStatus(status: string): SessionRecord['status'] {
  const normalized = normalizeEnum(status);
  if (normalized === 'FINISHED') return 'done';
  if (normalized === 'CANCELLED') return 'cancelled';
  return 'error';
}

function acpLifecycleStatus(
  stop: 'end_turn' | 'max_tokens' | 'max_turn_requests' | 'refusal' | 'cancelled',
): string {
  if (stop === 'end_turn') return 'RUN_LIFECYCLE_STATUS_FINISHED';
  if (stop === 'cancelled') return 'RUN_LIFECYCLE_STATUS_CANCELLED';
  if (stop === 'max_tokens') return 'RUN_LIFECYCLE_STATUS_MAX_TOKENS';
  if (stop === 'max_turn_requests') return 'RUN_LIFECYCLE_STATUS_MAX_TURN_REQUESTS';
  return 'RUN_LIFECYCLE_STATUS_REFUSAL';
}

function sameRepositories(left: Repository[], right: Repository[]): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

function assertSameAgentCreation(expected: SessionRecord, current: SessionRecord): void {
  if (
    expected.agent_id !== current.agent_id ||
    expected.runtime !== current.runtime ||
    expected.workspace !== current.workspace ||
    expected.name !== current.name ||
    expected.model !== current.model ||
    JSON.stringify(expected.tools) !== JSON.stringify(current.tools) ||
    !sameRepositories(expected.repositories, current.repositories) ||
    expected.work_on_current_branch !== current.work_on_current_branch ||
    expected.auto_create_pr !== current.auto_create_pr ||
    expected.create_idempotency_key !== current.create_idempotency_key
  ) {
    throw new SessionConflictError('Cursor CreateAgent identity changed concurrently');
  }
}

function claimIsStale(record: SessionRecord, config: Config): boolean {
  if (!record.claim_id || record.claim_started_at_ms == null) return true;
  const staleAfterMs = Math.max(
    300_000,
    config.startup_timeout_ms + config.rpc_timeout_ms + 10_000,
  );
  return Date.now() - record.claim_started_at_ms >= staleAfterMs;
}

function randomIdempotencyKey(kind: string): string {
  return `iii-cursor-${kind}-${hash(randomUUID()).slice(0, 32)}`;
}

function hash(value: string): string {
  return createHash('sha256').update(value).digest('hex');
}

function stableFrameItemId(
  record: SessionRecord,
  frame: RunStreamMessageWire,
  occurrences: Map<string, number>,
): string {
  const turn =
    record.send_idempotency_key ?? record.active_run_id ?? record.last_run_id ?? 'pending';
  if (frame.offset) {
    return `cursor-${hash(`${record.agent_id}:${turn}:observe:${frame.offset}`).slice(0, 40)}`;
  }
  const envelope = {
    sdkMessage: frame.sdkMessage,
    result: frame.result,
    done: frame.done,
    interactionUpdate: frame.interactionUpdate,
    step: frame.step,
  };
  const fingerprint = hash(JSON.stringify(canonicalValue(envelope)));
  const occurrence = occurrences.get(fingerprint) ?? 0;
  occurrences.set(fingerprint, occurrence + 1);
  return `cursor-${hash(`${record.agent_id}:${turn}:${fingerprint}:${occurrence}`).slice(0, 40)}`;
}

function stableAcpItemId(
  record: SessionRecord,
  notification: CursorAcpSessionUpdate,
  occurrences: Map<string, number>,
): string {
  const turn = record.active_run_id ?? record.send_idempotency_key ?? 'pending';
  const fingerprint = hash(JSON.stringify(canonicalValue(notification)));
  const occurrence = occurrences.get(fingerprint) ?? 0;
  occurrences.set(fingerprint, occurrence + 1);
  return `cursor-${hash(`${record.agent_id}:${turn}:acp:${fingerprint}:${occurrence}`).slice(0, 40)}`;
}

function acpUpdateFrame(update: Record<string, unknown>): RunStreamMessageWire | null {
  const type = typeof update.sessionUpdate === 'string' ? update.sessionUpdate : '';
  const content = objectValue(update.content);
  const text = typeof content?.text === 'string' ? content.text : '';
  if (type === 'agent_message_chunk' && text) {
    return { interactionUpdate: { type: 'text-delta', update: { delta: text } } };
  }
  if (type === 'agent_thought_chunk' && text) {
    return { interactionUpdate: { type: 'thinking-delta', update: { delta: text } } };
  }
  const callId = typeof update.toolCallId === 'string' ? update.toolCallId : '';
  if (type === 'tool_call' && callId) {
    return {
      interactionUpdate: {
        type: 'tool-call-started',
        update: {
          callId,
          toolCall: {
            type:
              typeof update.kind === 'string'
                ? update.kind
                : typeof update.title === 'string'
                  ? update.title
                  : 'tool',
            args: update.rawInput ?? {},
          },
        },
      },
    };
  }
  if (type === 'tool_call_update' && callId) {
    const status = typeof update.status === 'string' ? update.status : '';
    if (!['completed', 'failed', 'cancelled'].includes(status)) return null;
    return {
      interactionUpdate: {
        type: 'tool-call-completed',
        update: {
          callId,
          toolCall: {
            type: typeof update.kind === 'string' ? update.kind : 'tool',
            args: {},
            result: {
              status: status === 'completed' ? 'success' : 'error',
              content: update.content ?? update.rawOutput ?? null,
            },
          },
        },
      },
    };
  }
  return null;
}

function objectValue(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function canonicalValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonicalValue);
  if (!value || typeof value !== 'object') return value;
  return Object.fromEntries(
    Object.entries(value)
      .filter(([, entry]) => entry !== undefined)
      .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
      .map(([key, entry]) => [key, canonicalValue(entry)]),
  );
}

function isRetryableSendError(error: unknown): boolean {
  if (error instanceof BridgeTransportError) return true;
  if (!(error instanceof BridgeRpcError)) return false;
  const sdkCode = String(error.detail?.sdk_error_code ?? '');
  return (
    ['AGENT_BUSY', 'RATE_LIMIT_EXCEEDED', 'UPSTREAM_ERROR', 'INTERNAL_ERROR'].includes(sdkCode) ||
    ['aborted', 'resource_exhausted', 'unavailable'].includes(error.code)
  );
}

async function retryDelay(error: unknown, heartbeat: () => Promise<void>): Promise<void> {
  let delayMs = 250;
  if (error instanceof BridgeRpcError) {
    const retryAfter = error.detail?.retry_after_seconds;
    const reset = Number(error.detail?.rate_limit?.reset_epoch_seconds);
    if (retryAfter !== undefined && Number.isFinite(retryAfter)) {
      delayMs = Math.max(0, retryAfter * 1_000);
    } else if (Number.isFinite(reset)) {
      delayMs = Math.max(0, reset * 1_000 - Date.now());
    }
  }
  let remainingMs = delayMs;
  while (remainingMs > 0) {
    const sliceMs = Math.min(30_000, remainingMs);
    await new Promise((resolvePromise) => setTimeout(resolvePromise, sliceMs));
    remainingMs -= sliceMs;
    await heartbeat();
  }
}

async function completesWithin(operation: Promise<unknown>, timeoutMs: number): Promise<boolean> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const result = await Promise.race([
    operation.then(() => true),
    new Promise<false>((resolvePromise) => {
      timer = setTimeout(() => resolvePromise(false), timeoutMs);
    }),
  ]);
  if (timer) clearTimeout(timer);
  return result;
}

function errorDetails(error: unknown): z.infer<typeof RunResponseSchema>['error_details'] {
  if (!(error instanceof BridgeRpcError)) return null;
  const rate = error.detail?.rate_limit;
  return {
    transport_code: error.code,
    sdk_error_code: error.detail?.sdk_error_code ?? null,
    request_id: error.detail?.request_id ?? null,
    retry_after_seconds: error.detail?.retry_after_seconds ?? null,
    rate_limit: rate
      ? {
          limit: rate.limit ?? null,
          remaining: rate.remaining ?? null,
          reset_epoch_seconds: rate.reset_epoch_seconds ?? null,
        }
      : null,
  };
}

function safeError(error: unknown): string {
  if (error instanceof BridgeRpcError && error.detail?.request_id) {
    return `${error.message} (request_id: ${error.detail.request_id})`;
  }
  return error instanceof Error ? error.message : String(error);
}

class LocalRecoveryRequiredError extends Error {}
class CursorTurnTimeoutError extends Error {}
class CursorPayloadError extends Error {}
class SessionConflictError extends Error {}
