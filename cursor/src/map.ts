import { createHash } from 'node:crypto';
import type { Emit } from './events.js';
import type {
  AgentInfo,
  AgentMessage,
  AgentUsage,
  AssistantMessage,
  ContentBlock,
  CursorModel,
  FunctionResultMessage,
  RunSnapshot,
  RunStreamMessageWire,
  TokenUsage,
  UsageCost,
} from './types.js';

type WireTokenUsage = {
  inputTokens?: string | number;
  outputTokens?: string | number;
  cacheReadTokens?: string | number;
  cacheWriteTokens?: string | number;
  totalTokens?: string | number;
  reasoningTokens?: string | number;
};

type ToolState = {
  functionId: string;
  startedAt: number;
  ended: boolean;
};

export class RunAccumulator {
  runId: string | null = null;
  status = 'RUN_LIFECYCLE_STATUS_UNSPECIFIED';
  resultText = '';
  usage: TokenUsage | null = null;
  errorCode: string | null = null;
  errorMessage: string | null = null;
  private text = '';
  private thinking = '';
  private emittedText = '';
  private emittedThinking = '';
  private readonly calls = new Map<string, ToolState>();
  private readonly functionResults: FunctionResultMessage[] = [];
  private finalized = false;
  private deliveryKey: string | undefined;
  private deliveryOrdinal = 0;

  constructor(
    private readonly sessionId: string,
    private model: string,
    private readonly emit: Emit,
  ) {}

  get resolvedModel(): string {
    return this.model;
  }

  async ingest(
    frame: RunStreamMessageWire,
    emitUpdates = true,
    deliveryKey?: string,
  ): Promise<void> {
    this.deliveryKey = deliveryKey;
    this.deliveryOrdinal = 0;
    const frameRunId = extractRunId(frame);
    if (frameRunId) this.runId = frameRunId;
    if (frame.sdkMessage) await this.ingestSdkMessage(frame.sdkMessage, emitUpdates);
    if (frame.interactionUpdate) {
      await this.ingestInteractionUpdate(
        frame.interactionUpdate.type,
        frame.interactionUpdate.update,
        emitUpdates,
      );
    }
    if (frame.result) {
      this.status = normalizeEnum(frame.result.status);
      this.errorCode = frame.result.errorCode ?? null;
      const result = frame.result.result;
      if (result) {
        this.runId = result.runId ?? frame.result.runId ?? this.runId;
        this.status = normalizeEnum(result.status ?? frame.result.status);
        this.resultText = result.result ?? '';
        if (result.model?.id) this.model = result.model.id;
        const terminalUsage = mapTokenUsage(result.usage);
        if (terminalUsage) this.usage = terminalUsage;
      }
    }
  }

  async finalize(): Promise<{
    message: AssistantMessage;
    messages: AgentMessage[];
    functionResults: FunctionResultMessage[];
  }> {
    if (this.finalized) throw new Error('Cursor run accumulator was finalized twice');
    this.finalized = true;
    this.deliveryKey = this.runId
      ? `cursor-${createHash('sha256').update(`${this.sessionId}:${this.runId}:terminal`).digest('hex').slice(0, 32)}`
      : undefined;
    this.deliveryOrdinal = 0;
    const finalText = this.resultText || this.text;
    await this.flushMissing(finalText, this.thinking);
    const bodyStreamed =
      Boolean(this.emittedText || this.emittedThinking) &&
      this.emittedText === finalText &&
      this.emittedThinking === this.thinking;
    const content: ContentBlock[] = [];
    if (this.thinking) content.push({ type: 'thinking', text: this.thinking });
    if (finalText) content.push({ type: 'text', text: finalText });
    const message: AssistantMessage = {
      role: 'assistant',
      content,
      stop_reason: stopReason(this.status),
      error_message:
        stopReason(this.status) === 'end' ? null : (this.errorMessage ?? this.errorCode),
      usage: this.usage,
      model: this.model,
      provider: 'cursor',
      timestamp: Date.now(),
    };
    const messages: AgentMessage[] = [...this.functionResults, message];
    await this.emitEvent(
      {
        type: 'message_complete',
        message,
        ...(bodyStreamed ? { body_streamed: true } : {}),
      },
      'message-complete',
    );
    await this.emitEvent(
      {
        type: 'turn_end',
        message,
        function_results: this.functionResults,
      },
      'turn-end',
    );
    await this.emitEvent({ type: 'agent_end', messages }, 'agent-end');
    return { message, messages, functionResults: [...this.functionResults] };
  }

  private async ingestSdkMessage(
    sdkMessage: { type: string; message: Record<string, unknown> },
    emitUpdates: boolean,
  ): Promise<void> {
    const message = sdkMessage.message;
    const runId = stringValue(message.run_id) ?? stringValue(message.runId);
    if (runId) this.runId = runId;
    if (sdkMessage.type === 'assistant') {
      const content = nestedArray(message, ['message', 'content']);
      const text = content
        .filter((block) => block.type === 'text' && typeof block.text === 'string')
        .map((block) => String(block.text))
        .join('');
      if (text) {
        this.text = text;
        if (emitUpdates && text.startsWith(this.emittedText)) {
          const delta = text.slice(this.emittedText.length);
          if (delta) {
            const delivered = await this.emitEvent(
              { type: 'message_update', llm_event: { type: 'text_delta', delta } },
              'text-delta',
            );
            if (delivered !== false) this.emittedText += delta;
          }
        }
      }
      return;
    }
    if (sdkMessage.type === 'thinking') {
      const text = stringValue(message.text);
      if (text) {
        this.thinking = text;
        if (emitUpdates && text.startsWith(this.emittedThinking)) {
          const delta = text.slice(this.emittedThinking.length);
          if (delta) {
            const delivered = await this.emitEvent(
              { type: 'message_update', llm_event: { type: 'thinking_delta', delta } },
              'thinking-delta',
            );
            if (delivered !== false) this.emittedThinking += delta;
          }
        }
      }
      return;
    }
    if (sdkMessage.type === 'status') {
      const status = message.status;
      if (status !== undefined) this.status = normalizeEnum(status);
      const text = stringValue(message.message);
      if (text) this.errorMessage = text;
      return;
    }
    if (sdkMessage.type === 'usage') {
      const usage = mapStructTokenUsage(objectValue(message.usage));
      if (usage) this.usage = usage;
      return;
    }
    if (sdkMessage.type !== 'tool_call') return;
    const callId = stringValue(message.call_id) ?? stringValue(message.callId);
    const name = stringValue(message.name);
    if (!callId || !name) return;
    const status = stringValue(message.status) ?? '';
    if (status === 'running') {
      await this.startTool(callId, name, message.args ?? {}, emitUpdates);
      return;
    }
    if (status === 'completed' || status === 'error') {
      await this.endTool(
        callId,
        name,
        message.args ?? {},
        message.result,
        status === 'error',
        emitUpdates,
      );
    }
  }

  private async ingestInteractionUpdate(
    type: string,
    update: Record<string, unknown>,
    emitUpdates: boolean,
  ): Promise<void> {
    if (type === 'text-delta') {
      const delta = stringValue(update.text) ?? stringValue(update.delta) ?? '';
      if (!delta) return;
      this.text += delta;
      if (emitUpdates) {
        const delivered = await this.emitEvent(
          {
            type: 'message_update',
            llm_event: { type: 'text_delta', delta },
          },
          'text-delta',
        );
        if (delivered !== false) this.emittedText += delta;
      }
      return;
    }
    if (type === 'thinking-delta') {
      const delta = stringValue(update.text) ?? stringValue(update.delta) ?? '';
      if (!delta) return;
      this.thinking += delta;
      if (emitUpdates) {
        const delivered = await this.emitEvent(
          {
            type: 'message_update',
            llm_event: { type: 'thinking_delta', delta },
          },
          'thinking-delta',
        );
        if (delivered !== false) this.emittedThinking += delta;
      }
      return;
    }
    const callId = stringValue(update.callId) ?? stringValue(update.call_id);
    const toolCall = objectValue(update.toolCall) ?? objectValue(update.tool_call);
    const name = toolCall ? (stringValue(toolCall.type) ?? stringValue(toolCall.name)) : undefined;
    if (!callId || !name || !toolCall) return;
    if (type === 'tool-call-started') {
      await this.startTool(callId, name, toolCall.args ?? {}, emitUpdates);
    } else if (type === 'tool-call-completed') {
      const result = objectValue(toolCall.result);
      const isError = stringValue(result?.status) === 'error';
      await this.endTool(callId, name, toolCall.args ?? {}, result, isError, emitUpdates);
    }
  }

  private async startTool(
    callId: string,
    name: string,
    args: unknown,
    emitUpdates: boolean,
  ): Promise<void> {
    if (this.calls.has(callId)) return;
    const state = { functionId: toolFunctionId(name), startedAt: Date.now(), ended: false };
    this.calls.set(callId, state);
    if (!emitUpdates) return;
    await this.emitEvent(
      {
        type: 'function_execution_start',
        function_call_id: callId,
        function_id: state.functionId,
        args,
      },
      'function-start',
    );
  }

  private async endTool(
    callId: string,
    name: string,
    args: unknown,
    result: unknown,
    isError: boolean,
    emitUpdates: boolean,
  ): Promise<void> {
    if (!this.calls.has(callId)) await this.startTool(callId, name, args, emitUpdates);
    const state = this.calls.get(callId);
    if (!state || state.ended) return;
    state.ended = true;
    const content: ContentBlock[] = [{ type: 'text', text: formatUnknown(result) }];
    const functionResult: FunctionResultMessage = {
      role: 'function_result',
      function_call_id: callId,
      function_id: state.functionId,
      content,
      details: result ?? null,
      is_error: isError,
      timestamp: Date.now(),
    };
    this.functionResults.push(functionResult);
    if (!emitUpdates) return;
    await this.emitEvent(
      {
        type: 'function_execution_end',
        function_call_id: callId,
        function_id: state.functionId,
        result: { content, details: result ?? null },
        is_error: isError,
        duration_ms: Date.now() - state.startedAt,
      },
      'function-end',
    );
  }

  private async flushMissing(finalText: string, finalThinking: string): Promise<boolean> {
    let streamed = Boolean(this.emittedText || this.emittedThinking);
    if (finalThinking.startsWith(this.emittedThinking)) {
      const delta = finalThinking.slice(this.emittedThinking.length);
      if (delta && streamed) {
        const delivered = await this.emitEvent(
          {
            type: 'message_update',
            llm_event: { type: 'thinking_delta', delta },
          },
          'thinking-final',
        );
        if (delivered !== false) this.emittedThinking += delta;
        streamed = delivered !== false;
      }
    }
    if (finalText.startsWith(this.emittedText)) {
      const delta = finalText.slice(this.emittedText.length);
      if (delta && streamed) {
        const delivered = await this.emitEvent(
          {
            type: 'message_update',
            llm_event: { type: 'text_delta', delta },
          },
          'text-final',
        );
        if (delivered !== false) this.emittedText += delta;
        streamed = delivered !== false;
      }
    }
    return streamed;
  }

  private emitEvent(event: unknown, kind: string): Promise<unknown> {
    const itemId = this.deliveryKey
      ? `${this.deliveryKey}-${kind}-${this.deliveryOrdinal++}`
      : undefined;
    return this.emit(this.sessionId, event, itemId);
  }
}

export function extractRunId(frame: RunStreamMessageWire): string | null {
  const message = frame.sdkMessage?.message;
  return (
    frame.result?.runId ??
    frame.result?.result?.runId ??
    frame.done?.runId ??
    stringValue(message?.run_id) ??
    stringValue(message?.runId) ??
    null
  );
}

export function mapTokenUsage(raw: WireTokenUsage | undefined): TokenUsage | null {
  if (
    !raw ||
    raw.inputTokens === undefined ||
    raw.outputTokens === undefined ||
    raw.cacheReadTokens === undefined ||
    raw.cacheWriteTokens === undefined ||
    raw.totalTokens === undefined
  ) {
    return null;
  }
  const usage: TokenUsage = {
    input_tokens: safeInt(raw.inputTokens),
    output_tokens: safeInt(raw.outputTokens),
    cache_read_tokens: safeInt(raw.cacheReadTokens),
    cache_write_tokens: safeInt(raw.cacheWriteTokens),
    total_tokens: safeInt(raw.totalTokens),
  };
  if (raw.reasoningTokens !== undefined) usage.reasoning_tokens = safeInt(raw.reasoningTokens);
  return usage;
}

export function mapStructTokenUsage(raw: Record<string, unknown> | undefined): TokenUsage | null {
  if (!raw) return null;
  return mapTokenUsage({
    inputTokens: intValue(raw.input_tokens ?? raw.inputTokens),
    outputTokens: intValue(raw.output_tokens ?? raw.outputTokens),
    cacheReadTokens: intValue(raw.cache_read_tokens ?? raw.cacheReadTokens),
    cacheWriteTokens: intValue(raw.cache_write_tokens ?? raw.cacheWriteTokens),
    totalTokens: intValue(raw.total_tokens ?? raw.totalTokens),
    reasoningTokens: intValue(raw.reasoning_tokens ?? raw.reasoningTokens),
  });
}

export function mapCost(
  raw: { rawCostCents?: number; chargedCents?: number } | undefined,
): UsageCost | null {
  if (raw?.rawCostCents === undefined || raw.chargedCents === undefined) return null;
  return {
    raw_cost_cents: raw.rawCostCents,
    charged_cents: raw.chargedCents,
  };
}

export function mapUsageResponse(raw: {
  usage?: {
    usage?: WireTokenUsage;
    cost?: { rawCostCents?: number; chargedCents?: number };
    runs?: Array<{
      runId: string;
      usage?: WireTokenUsage;
      cost?: { rawCostCents?: number; chargedCents?: number };
    }>;
  };
}): AgentUsage {
  return {
    usage: mapTokenUsage(raw.usage?.usage),
    cost: mapCost(raw.usage?.cost),
    runs: (raw.usage?.runs ?? []).flatMap((run) => {
      const usage = mapTokenUsage(run.usage);
      return usage ? [{ run_id: run.runId, usage, cost: mapCost(run.cost) }] : [];
    }),
  };
}

export function mapRunSnapshot(raw: {
  runId?: string;
  agentId?: string;
  status?: string | number;
  result?: string;
  model?: { id?: string };
  durationMs?: string | number;
  createdAt?: string;
  usage?: WireTokenUsage;
}): RunSnapshot {
  return {
    run_id: raw.runId ?? '',
    agent_id: raw.agentId ?? '',
    status: normalizeEnum(raw.status),
    result: raw.result ?? '',
    model: raw.model?.id ?? '',
    duration_ms: raw.durationMs === undefined ? null : safeInt(raw.durationMs),
    created_at: raw.createdAt ?? null,
    usage: mapTokenUsage(raw.usage),
  };
}

export function mapAgentInfo(raw: {
  agentId: string;
  name?: string;
  summary?: string;
  status?: string | number;
  createdAt?: string;
  lastModified?: string;
  archived?: boolean;
  local?: { cwd?: string };
  cloud?: { repos?: string[]; metadata?: Record<string, string> };
}): AgentInfo {
  return {
    agent_id: raw.agentId,
    name: raw.name ?? '',
    summary: raw.summary ?? '',
    status: normalizeEnum(raw.status),
    archived: raw.archived ?? false,
    created_at: raw.createdAt ?? null,
    last_modified: raw.lastModified ?? null,
    runtime: raw.local ? 'local' : raw.cloud ? 'cloud' : null,
    cwd: raw.local?.cwd ?? null,
    repositories: raw.cloud?.repos ?? [],
    metadata: raw.cloud?.metadata ?? {},
  };
}

export function mapModels(raw: {
  items: Array<{
    id: string;
    displayName?: string;
    description?: string;
    parameters?: Array<{
      id: string;
      displayName?: string;
      values?: Array<{ value: string; displayName?: string }>;
    }>;
    variants?: Array<{
      params?: Array<{ id: string; value: string }>;
      displayName?: string;
      description?: string;
      isDefault?: boolean;
    }>;
  }>;
}): CursorModel[] {
  return raw.items.map((model) => ({
    id: model.id,
    display_name: model.displayName ?? '',
    description: model.description ?? '',
    parameters: (model.parameters ?? []).map((parameter) => ({
      id: parameter.id,
      display_name: parameter.displayName ?? '',
      values: (parameter.values ?? []).map((value) => ({
        value: value.value,
        display_name: value.displayName ?? '',
      })),
    })),
    variants: (model.variants ?? []).map((variant) => ({
      params: variant.params ?? [],
      display_name: variant.displayName ?? '',
      description: variant.description ?? '',
      is_default: variant.isDefault ?? false,
    })),
  }));
}

export function stopReason(status: string): 'end' | 'length' | 'aborted' | 'error' {
  const normalized = normalizeEnum(status);
  if (normalized === 'FINISHED') return 'end';
  if (normalized === 'MAX_TOKENS' || normalized === 'MAX_TURN_REQUESTS') return 'length';
  if (normalized === 'CANCELLED') return 'aborted';
  return 'error';
}

export function normalizeEnum(value: unknown): string {
  if (typeof value === 'number') return String(value);
  if (typeof value !== 'string') return 'UNSPECIFIED';
  return value.replace(/^RUN_LIFECYCLE_STATUS_/, '').replace(/^AGENT_INFO_STATUS_/, '');
}

function safeInt(value: string | number | undefined): number {
  if (value === undefined) return 0;
  const integer = typeof value === 'number' ? BigInt(value) : BigInt(value);
  if (integer < 0n || integer > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new Error(`Cursor sdk.v1 int64 is outside the safe JavaScript integer range: ${value}`);
  }
  return Number(integer);
}

function intValue(value: unknown): string | number | undefined {
  return typeof value === 'string' || typeof value === 'number' ? value : undefined;
}

function stringValue(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined;
}

function objectValue(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function nestedArray(
  root: Record<string, unknown>,
  path: string[],
): Array<Record<string, unknown>> {
  let value: unknown = root;
  for (const segment of path) value = objectValue(value)?.[segment];
  return Array.isArray(value)
    ? value.filter(
        (entry): entry is Record<string, unknown> =>
          Boolean(entry) && typeof entry === 'object' && !Array.isArray(entry),
      )
    : [];
}

function toolFunctionId(name: string): string {
  const normalized = name.replace(/[^A-Za-z0-9_-]+/g, '-').replace(/^-+|-+$/g, '') || 'unknown';
  return `cursor::tool::${normalized}`;
}

function formatUnknown(value: unknown): string {
  if (typeof value === 'string') return value;
  if (value === undefined) return '';
  return JSON.stringify(value);
}
