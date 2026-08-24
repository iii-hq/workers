import { randomUUID } from 'node:crypto';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { IIIClient } from 'iii-sdk';
import { z } from 'zod';
import {
  type CursorAcpSessionUpdate,
  type CursorCliClient,
  type CursorCliFactory,
  ProductionCursorCliFactory,
  resolveCursorAcpModelId,
} from './cli.js';
import { type Config, cursorCliLaunchOptions } from './config.js';
import { jsonSchema } from './schema.js';
import type { CursorModel } from './types.js';

export const CURSOR_PROVIDER_ID = 'cursor';
const TOKEN_SCOPE = 'provider-cursor';
const TOKEN_KEY = 'registration_token';
const MODEL_PREFIX = 'cursor/';
const FALLBACK_CONTEXT_WINDOW = 32_768;
const FALLBACK_MAX_OUTPUT_TOKENS = 8_192;
const REFRESH_INTERVAL_MS = 3 * 60_000;
const PING_INTERVAL_MS = 25_000;
const SHUTDOWN_SETTLE_MS = 5_000;

type JsonObject = Record<string, unknown>;

type ProviderWriter = {
  sendMessage(message: string): void;
  close(): void;
  stream?: {
    on(event: 'error' | 'close', listener: (...args: unknown[]) => void): unknown;
  };
};

type ProviderContent = { type: 'text'; text: string } | { type: 'thinking'; text: string };

type ProviderMessage = {
  role: 'assistant';
  content: ProviderContent[];
  stop_reason: 'end' | 'length' | 'aborted' | 'error';
  native_stop_reason?: string;
  error_message?: string | null;
  error_kind?: 'auth_expired' | 'rate_limited' | 'context_overflow' | 'transient' | 'permanent';
  warnings?: string[];
  model: string;
  provider: 'cursor';
  timestamp: number;
};

type ProviderStreamInput = {
  writer: ProviderWriter;
  systemPrompt?: string;
  model: string;
  messages: JsonObject[];
  tools?: unknown[];
  responseFormat?: JsonObject;
  thinkingLevel?: string;
  maxOutputTokens?: number;
  resolutionKey?: string;
};

type Inflight = {
  aborted: boolean;
  client: CursorCliClient | null;
  sessionId: string | null;
};

type WorkspaceFactory = {
  create(): Promise<string>;
  remove(path: string): Promise<void>;
};

const StreamRegistrationSchema = z
  .object({
    writer_ref: z.object({
      channel_id: z.string(),
      access_key: z.string(),
      direction: z.literal('write'),
    }),
    system_prompt: z.string().optional(),
    model: z.string().min(1),
    messages: z.array(z.record(z.string(), z.unknown())),
    tools: z.array(z.unknown()).optional(),
    response_format: z.record(z.string(), z.unknown()).optional(),
    thinking_level: z.string().optional(),
    max_output_tokens: z.number().int().positive().optional(),
    provider_options: z.unknown().optional(),
    model_meta: z.unknown().optional(),
    resolution_key: z.string().optional(),
    session_id: z.string().optional(),
  })
  .passthrough();

const HydratedStreamSchema = StreamRegistrationSchema.omit({ writer_ref: true }).extend({
  writer_ref: z.unknown(),
});

const StreamResponseSchema = z.object({ ok: z.boolean() });
const AbortRequestSchema = z.object({ request_id: z.string() }).passthrough();
const AbortResponseSchema = z.object({ aborted: z.boolean() });
const RefreshResponseSchema = z.object({ ok: z.boolean(), count: z.number().int().nonnegative() });
const ReadyResponseSchema = z.object({ ok: z.boolean() });

export function cursorProviderDeclaration(): JsonObject {
  return {
    id: CURSOR_PROVIDER_ID,
    display_name: 'Cursor',
    defaults: { max_tokens: FALLBACK_MAX_OUTPUT_TOKENS },
    supports_model_listing: true,
    worker_id: 'cursor',
  };
}

export function cursorCatalogModels(models: CursorModel[]): JsonObject[] {
  const seen = new Set<string>();
  return models.flatMap((model) => {
    const id = model.id.trim();
    if (!id || seen.has(id)) return [];
    seen.add(id);
    return [
      {
        id: `${MODEL_PREFIX}${id}`,
        provider: CURSOR_PROVIDER_ID,
        display_name: model.display_name || id,
        context_window: FALLBACK_CONTEXT_WINDOW,
        max_output_tokens: FALLBACK_MAX_OUTPUT_TOKENS,
        supports_tools: false,
        supports_vision: false,
        supports_cache: false,
        supports_structured_output: false,
      },
    ];
  });
}

export function cursorProviderPrompt(input: {
  systemPrompt?: string;
  messages: JsonObject[];
}): string {
  const sections = [
    'You are producing one text response for an external chat application.',
    'Use only the transcript below. Do not inspect or modify files, run commands, or invoke Cursor workspace tools.',
  ];
  if (input.systemPrompt?.trim()) {
    sections.push(`System instructions:\n${input.systemPrompt.trim()}`);
  }
  const transcript = input.messages
    .map((message) => {
      const role = typeof message.role === 'string' ? message.role : 'unknown';
      return `${role}: ${renderMessageContent(message)}`;
    })
    .join('\n\n');
  sections.push(`Conversation transcript:\n${transcript}`, 'assistant:');
  return sections.join('\n\n');
}

export class CursorProvider {
  private readonly inflight = new Map<string, Inflight>();
  private readonly inflightByResolution = new Map<string, Set<string>>();
  private readonly activeStreams = new Set<Promise<{ ok: true }>>();
  private readonly workspace: WorkspaceFactory;
  private persistedRegistrationToken: string | null = null;
  private registrationToken: string | null = null;
  private rebind: Promise<void> | null = null;
  private rebindRequested = false;
  private refreshQueue: Promise<void> = Promise.resolve();
  private refreshTimer: ReturnType<typeof setInterval> | null = null;
  private closed = false;

  constructor(
    private readonly iii: IIIClient,
    private readonly getConfig: () => Config,
    private readonly cliFactory: CursorCliFactory = new ProductionCursorCliFactory(),
    workspace: Partial<WorkspaceFactory> = {},
  ) {
    this.workspace = {
      create: workspace.create ?? (() => mkdtemp(join(tmpdir(), 'iii-cursor-provider-'))),
      remove: workspace.remove ?? ((path) => rm(path, { recursive: true, force: true })),
    };
  }

  register(): void {
    this.iii.registerFunction(
      'provider::cursor::stream',
      (payload: unknown) => this.runStream(payload),
      {
        description:
          'Stream a text-only Cursor completion through the official login-backed Cursor Agent ACP.',
        request_format: jsonSchema(StreamRegistrationSchema),
        response_format: jsonSchema(StreamResponseSchema),
        metadata: { internal: true },
      },
    );
    this.iii.registerFunction(
      'provider::cursor::abort',
      async (payload: unknown) => this.abort(AbortRequestSchema.parse(payload ?? {}).request_id),
      {
        description: 'Cancel an in-flight Cursor ACP provider request.',
        request_format: jsonSchema(AbortRequestSchema),
        response_format: jsonSchema(AbortResponseSchema),
        metadata: { internal: true },
      },
    );
    this.iii.registerFunction(
      'provider::cursor::refresh_models',
      async () => ({ ok: true, count: await this.refreshModels() }),
      {
        description:
          'Refresh the login-backed Cursor model catalog and replace its LLM Router slice.',
        request_format: { type: 'object', properties: {}, additionalProperties: true },
        response_format: jsonSchema(RefreshResponseSchema),
        metadata: { internal: true },
      },
    );
    this.iii.registerFunction(
      'provider::cursor::on_router_ready',
      async () => {
        void this.rebindAndRefresh();
        return { ok: true };
      },
      {
        description: 'Re-register the Cursor provider after an LLM Router restart.',
        request_format: { type: 'object', properties: {}, additionalProperties: true },
        response_format: jsonSchema(ReadyResponseSchema),
        metadata: { internal: true },
      },
    );
    this.iii.registerTrigger({
      type: 'router::ready',
      function_id: 'provider::cursor::on_router_ready',
      config: {},
    });
    void this.rebindAndRefresh();
    this.refreshTimer = setInterval(() => {
      void this.refreshModels().catch((error) => {
        console.warn(`cursor provider model refresh failed: ${safeError(error)}`);
      });
    }, REFRESH_INTERVAL_MS);
    this.refreshTimer.unref();
  }

  async declareOnce(): Promise<void> {
    const token = await this.loadToken();
    const payload = cursorProviderDeclaration();
    if (token) payload.token = token;
    const response = z
      .object({ registration_token: z.string().min(1) })
      .passthrough()
      .parse(
        await this.iii.trigger({
          function_id: 'router::provider::register',
          payload,
          timeoutMs: 15_000,
        }),
      );
    this.registrationToken = response.registration_token;
    if (response.registration_token !== this.persistedRegistrationToken) {
      await this.persistRegistrationToken(response.registration_token);
    }
  }

  refreshModels(): Promise<number> {
    if (this.closed) return Promise.reject(new Error('Cursor provider is closed'));
    const operation = this.refreshQueue.catch(() => undefined).then(() => this.refreshModelsOnce());
    this.refreshQueue = operation.then(
      () => undefined,
      () => undefined,
    );
    return operation;
  }

  private async refreshModelsOnce(): Promise<number> {
    if (this.closed) throw new Error('Cursor provider is closed');
    const config = this.getConfig();
    const options = cursorCliLaunchOptions(config, config.workspace);
    const auth = await this.cliFactory.authStatus(options);
    if (this.closed) throw new Error('Cursor provider is closed');
    const token = await this.loadToken();
    if (!auth.authenticated) {
      if (this.closed) throw new Error('Cursor provider is closed');
      await this.reconcile([], token);
      return 0;
    }
    const discovered = await this.cliFactory.listModels(options);
    if (this.closed) throw new Error('Cursor provider is closed');
    const models = cursorCatalogModels(discovered);
    if (models.length === 0) throw new Error('Cursor Agent ACP returned no selectable models');
    await this.reconcile(models, token);
    return models.length;
  }

  async close(): Promise<void> {
    this.closed = true;
    if (this.refreshTimer) clearInterval(this.refreshTimer);
    this.refreshTimer = null;
    const active = [...this.inflight.values()];
    await Promise.all(
      active.map(async (entry) => {
        entry.aborted = true;
        if (entry.client && entry.sessionId) {
          await entry.client.cancel(entry.sessionId).catch(() => undefined);
        }
      }),
    );
    const pending = [...this.activeStreams, this.refreshQueue];
    if (this.rebind) pending.push(this.rebind);
    await settleWithin(pending, SHUTDOWN_SETTLE_MS);
  }

  private runStream(payload: unknown): Promise<{ ok: true }> {
    if (this.closed) return Promise.reject(new Error('Cursor provider is closed'));
    const stream = this.stream(payload);
    const tracked = stream.finally(() => {
      this.activeStreams.delete(tracked);
    });
    this.activeStreams.add(tracked);
    return tracked;
  }

  private async stream(payload: unknown): Promise<{ ok: true }> {
    const input = parseStreamInput(payload);
    const key = randomUUID();
    const entry: Inflight = { aborted: false, client: null, sessionId: null };
    this.inflight.set(key, entry);
    if (input.resolutionKey !== undefined) {
      const keys = this.inflightByResolution.get(input.resolutionKey) ?? new Set<string>();
      keys.add(key);
      this.inflightByResolution.set(input.resolutionKey, keys);
    }
    const transport = { failed: false };
    let settled = false;
    const handleWriterError = () => {
      if (transport.failed) return;
      transport.failed = true;
      if (settled) return;
      entry.aborted = true;
      if (entry.client && entry.sessionId) {
        void entry.client.cancel(entry.sessionId).catch(() => undefined);
      }
    };
    input.writer.stream?.on('error', handleWriterError);
    input.writer.stream?.on('close', handleWriterError);
    const frames = new ProviderFrames(
      input.writer,
      input.model,
      providerWarnings(input),
      transport,
      handleWriterError,
    );
    frames.ping();
    let workspace: string | null = null;
    const ping = setInterval(() => frames.ping(), PING_INTERVAL_MS);
    ping.unref();
    try {
      const config = this.getConfig();
      const auth = await this.cliFactory.authStatus(
        cursorCliLaunchOptions(config, config.workspace),
      );
      if (!auth.authenticated) {
        frames.error('Cursor is not logged in; run cursor-agent login', 'auth_expired');
        return { ok: true };
      }
      if (entry.aborted || this.closed) return { ok: true };
      workspace = await this.workspace.create();
      const client = await this.cliFactory.create(cursorCliLaunchOptions(config, workspace));
      entry.client = client;
      const session = await client.newSession(workspace);
      entry.sessionId = session.sessionId;
      if (entry.aborted || this.closed) {
        await client.cancel(session.sessionId).catch(() => undefined);
        return { ok: true };
      }
      const requested = upstreamModelId(input.model);
      const selected = resolveCursorAcpModelId(requested, session.models);
      if (!selected) {
        frames.error(`Cursor model "${requested}" is not available for this login`, 'permanent');
        return { ok: true };
      }
      await client.setModel(session.sessionId, selected);
      await client.setMode(session.sessionId, 'ask');
      if (entry.aborted || this.closed) {
        await client.cancel(session.sessionId).catch(() => undefined);
        return { ok: true };
      }
      const stop = await client.prompt(
        session.sessionId,
        cursorProviderPrompt(input),
        async (notification) => frames.update(notification),
      );
      frames.finish(stop);
      return { ok: true };
    } catch (error) {
      frames.error(safeProviderError(error), classifyProviderError(error));
      return { ok: true };
    } finally {
      settled = true;
      clearInterval(ping);
      if (entry.client) await entry.client.close().catch(() => undefined);
      if (workspace) await this.workspace.remove(workspace).catch(() => undefined);
      if (this.inflight.get(key) === entry) this.inflight.delete(key);
      if (input.resolutionKey !== undefined) {
        const keys = this.inflightByResolution.get(input.resolutionKey);
        keys?.delete(key);
        if (keys?.size === 0) this.inflightByResolution.delete(input.resolutionKey);
      }
      input.writer.close();
    }
  }

  private async abort(requestId: string): Promise<{ aborted: boolean }> {
    const keys = this.inflightByResolution.get(requestId);
    if (!keys) return { aborted: false };
    let aborted = false;
    await Promise.all(
      [...keys].map(async (key) => {
        const entry = this.inflight.get(key);
        if (!entry || entry.aborted) return;
        entry.aborted = true;
        aborted = true;
        if (entry.client && entry.sessionId) {
          await entry.client.cancel(entry.sessionId).catch(() => undefined);
        }
      }),
    );
    return { aborted };
  }

  private rebindAndRefresh(): Promise<void> {
    this.rebindRequested = true;
    if (this.rebind) return this.rebind;
    const run = async () => {
      while (this.rebindRequested && !this.closed) {
        this.rebindRequested = false;
        await this.declareWithBackoff();
        if (this.closed) return;
        try {
          const count = await this.refreshModels();
          console.log(`cursor provider catalog reconciled: ${count} models`);
        } catch (error) {
          console.warn(`cursor provider catalog refresh failed: ${safeError(error)}`);
        }
      }
    };
    this.rebind = run().finally(() => {
      this.rebind = null;
      if (this.rebindRequested && !this.closed) void this.rebindAndRefresh();
    });
    return this.rebind;
  }

  private async declareWithBackoff(): Promise<void> {
    let delay = 500;
    while (!this.closed) {
      try {
        await this.declareOnce();
        return;
      } catch (error) {
        console.warn(`cursor provider registration failed: ${safeError(error)}`);
      }
      await new Promise((resolvePromise) => setTimeout(resolvePromise, delay));
      delay = Math.min(delay * 2, 10_000);
    }
  }

  private async loadToken(): Promise<string | null> {
    if (this.registrationToken) return this.registrationToken;
    const value = await this.iii.trigger({
      function_id: 'state::get',
      payload: { scope: TOKEN_SCOPE, key: TOKEN_KEY },
    });
    if (this.registrationToken) return this.registrationToken;
    const token = typeof value === 'string' && value ? value : null;
    this.persistedRegistrationToken = token;
    this.registrationToken = token;
    return token;
  }

  private async persistRegistrationToken(token: string): Promise<void> {
    let delay = 200;
    for (let attempt = 0; attempt < 5; attempt += 1) {
      try {
        await this.iii.trigger({
          function_id: 'state::set',
          payload: { scope: TOKEN_SCOPE, key: TOKEN_KEY, value: token },
        });
        this.persistedRegistrationToken = token;
        return;
      } catch (error) {
        if (attempt === 4) throw error;
        console.warn(`cursor provider registration-token persistence failed: ${safeError(error)}`);
      }
      await new Promise((resolvePromise) => setTimeout(resolvePromise, delay));
      delay = Math.min(delay * 2, 2_000);
    }
  }

  private async reconcile(models: JsonObject[], token: string | null): Promise<void> {
    await this.iii.trigger({
      function_id: 'router::models::reconcile',
      payload: {
        provider: CURSOR_PROVIDER_ID,
        models,
        ...(token ? { token } : {}),
      },
      timeoutMs: 15_000,
    });
  }
}

class ProviderFrames {
  private content: ProviderContent[] = [];
  private active: ProviderContent['type'] | null = null;
  private activeText = '';
  private started = false;
  private terminal = false;

  constructor(
    private readonly writer: ProviderWriter,
    private readonly model: string,
    private readonly warnings: string[],
    private readonly transport: { failed: boolean },
    private readonly onTransportError: (error: unknown) => void,
  ) {}

  start(): void {
    if (this.started || this.terminal) return;
    this.started = true;
    this.send({ type: 'start', partial: this.message('end') });
  }

  update(notification: CursorAcpSessionUpdate): void {
    const type = notification.update.sessionUpdate;
    const content = objectValue(notification.update.content);
    const text = typeof content?.text === 'string' ? content.text : '';
    if (type === 'agent_message_chunk' && text) this.delta('text', text);
    if (type === 'agent_thought_chunk' && text) this.delta('thinking', text);
  }

  ping(): void {
    if (!this.terminal) this.send({ type: 'ping' });
  }

  finish(stop: 'end_turn' | 'max_tokens' | 'max_turn_requests' | 'refusal' | 'cancelled'): void {
    if (this.terminal) return;
    const reason = providerStopReason(stop);
    if (reason === 'error') {
      this.error(`Cursor stopped with ${stop}`, 'permanent');
      return;
    }
    if (!this.started) this.start();
    this.closeBlock();
    const message = this.message(reason, stop);
    this.send({
      type: 'stop',
      stop_reason: reason,
    });
    this.send({ type: 'done', message });
    this.terminal = true;
  }

  error(message: string, kind: ProviderMessage['error_kind']): void {
    if (this.terminal) return;
    this.closeBlock();
    const error = this.message('error');
    error.error_message = message;
    error.error_kind = kind;
    this.send({ type: 'error', error });
    this.terminal = true;
  }

  private delta(type: ProviderContent['type'], delta: string): void {
    if (!this.started) this.start();
    if (this.active !== type) {
      this.closeBlock();
      this.active = type;
      this.activeText = '';
      this.send({
        type: `${type}_start`,
        partial: this.message('end', undefined, [...this.content, { type, text: '' }]),
      });
    }
    this.activeText += delta;
    this.send({ type: `${type}_delta`, delta });
  }

  private closeBlock(): void {
    if (!this.active) return;
    this.content.push({ type: this.active, text: this.activeText });
    this.send({ type: `${this.active}_end`, partial: this.message('end') });
    this.active = null;
    this.activeText = '';
  }

  private message(
    stopReason: ProviderMessage['stop_reason'],
    nativeStopReason?: string,
    content = this.content,
  ): ProviderMessage {
    return {
      role: 'assistant',
      content: structuredClone(content),
      stop_reason: stopReason,
      ...(nativeStopReason ? { native_stop_reason: nativeStopReason } : {}),
      ...(this.warnings.length ? { warnings: [...this.warnings] } : {}),
      model: this.model,
      provider: 'cursor',
      timestamp: Date.now(),
    };
  }

  private send(frame: JsonObject): void {
    if (this.transport.failed) return;
    try {
      this.writer.sendMessage(JSON.stringify(frame));
    } catch (error) {
      this.onTransportError(error);
    }
  }
}

function parseStreamInput(payload: unknown): ProviderStreamInput {
  const parsed = HydratedStreamSchema.parse(payload);
  return {
    writer: providerWriter(parsed.writer_ref),
    systemPrompt: parsed.system_prompt,
    model: parsed.model,
    messages: parsed.messages,
    tools: parsed.tools,
    responseFormat: parsed.response_format,
    thinkingLevel: parsed.thinking_level,
    maxOutputTokens: parsed.max_output_tokens,
    resolutionKey: parsed.resolution_key,
  };
}

function providerWriter(value: unknown): ProviderWriter {
  if (
    !value ||
    typeof value !== 'object' ||
    typeof (value as ProviderWriter).sendMessage !== 'function' ||
    typeof (value as ProviderWriter).close !== 'function'
  ) {
    throw new Error('Cursor provider requires a writable channel reference');
  }
  return value as ProviderWriter;
}

function renderMessageContent(message: JsonObject): string {
  const content = Array.isArray(message.content) ? message.content : [];
  const rendered = content.flatMap((block) => {
    if (!block || typeof block !== 'object') return [];
    const item = block as JsonObject;
    if (item.type === 'text' && typeof item.text === 'string') return [item.text];
    if (item.type === 'image')
      return [`[image omitted${typeof item.mime === 'string' ? `: ${item.mime}` : ''}]`];
    if (item.type === 'function_call') {
      return [
        `[function call ${String(item.function_id ?? '')}: ${safeJson(item.arguments ?? null)}]`,
      ];
    }
    if (item.type === 'function_result') {
      return [
        `[function result ${String(item.function_call_id ?? '')}: ${safeJson(item.content ?? null)}]`,
      ];
    }
    return [];
  });
  if (message.role === 'function_result') {
    return `[function result ${String(message.function_call_id ?? '')}] ${rendered.join('\n')}`;
  }
  if (message.role === 'custom' && typeof message.display === 'string') {
    rendered.unshift(message.display);
  }
  return rendered.join('\n');
}

function providerWarnings(input: ProviderStreamInput): string[] {
  const unsupported = [];
  if (input.tools?.length) unsupported.push('tools');
  if (input.responseFormat) unsupported.push('structured output');
  if (input.thinkingLevel) unsupported.push('thinking level');
  if (input.maxOutputTokens) unsupported.push('exact output-token limits');
  return unsupported.length
    ? [
        `Cursor Agent ACP does not expose ${unsupported.join(', ')}; those options were not forwarded.`,
      ]
    : [];
}

function upstreamModelId(model: string): string {
  return model.startsWith(MODEL_PREFIX) ? model.slice(MODEL_PREFIX.length) : model;
}

function providerStopReason(
  stop: 'end_turn' | 'max_tokens' | 'max_turn_requests' | 'refusal' | 'cancelled',
): ProviderMessage['stop_reason'] {
  if (stop === 'end_turn') return 'end';
  if (stop === 'max_tokens' || stop === 'max_turn_requests') return 'length';
  if (stop === 'cancelled') return 'aborted';
  return 'error';
}

function classifyProviderError(error: unknown): ProviderMessage['error_kind'] {
  const message = safeError(error).toLowerCase();
  if (message.includes('login') || message.includes('auth')) return 'auth_expired';
  if (message.includes('rate limit') || message.includes('too many requests'))
    return 'rate_limited';
  if (message.includes('context') && (message.includes('limit') || message.includes('long'))) {
    return 'context_overflow';
  }
  if (
    message.includes('not found') ||
    message.includes('not available') ||
    message.includes('invalid')
  ) {
    return 'permanent';
  }
  return 'transient';
}

function safeProviderError(error: unknown): string {
  const message = safeError(error);
  return message.replaceAll(/(?:key|token|secret)_[A-Za-z0-9._-]+/gi, '<redacted>');
}

function safeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function objectValue(value: unknown): JsonObject | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as JsonObject) : null;
}

function safeJson(value: unknown): string {
  try {
    return JSON.stringify(value);
  } catch {
    return '[unserializable]';
  }
}

async function settleWithin(promises: Promise<unknown>[], timeoutMs: number): Promise<void> {
  if (promises.length === 0) return;
  let timer: ReturnType<typeof setTimeout> | undefined;
  await Promise.race([
    Promise.allSettled(promises),
    new Promise<void>((resolvePromise) => {
      timer = setTimeout(resolvePromise, timeoutMs);
    }),
  ]);
  if (timer) clearTimeout(timer);
}
