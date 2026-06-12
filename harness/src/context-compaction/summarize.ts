import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { AgentMessage, AssistantMessage } from '../types/agent-message.js';
import { compactionConfig } from './config.js';
import { stampLastCompaction } from './lease.js';
import { type ModelLimit, preserveRecentBudget } from './overflow.js';
import {
  type CompactionEntryLike,
  type MessageWithEntryId,
  completedCompactions,
  selectWithEntryIds,
} from './selection.js';
import { stripMedia } from './strip-media.js';
import { buildPrompt } from './template.js';

/** Outer trigger budget; must exceed the router's 300s stream budget. */
const SUMMARIZER_TIMEOUT_MS = 320_000;

export type SummarizeMode = 'async' | 'sync';

export type SummarizeOptions = {
  mode: SummarizeMode;
  /** Sync path passes its already-truncated set so we don't double-load. */
  truncatedEntries?: MessageWithEntryId[];
};

export type SummarizeOk = {
  kind: 'ok';
  tail_start_id: string | null;
  tokens_before: number;
  compaction_entry_id: string;
  summary_text: string;
  tail_messages: AgentMessage[];
};

export type SummarizeFailure = { kind: 'compact'; reason: string };
export type SummarizeOutcome = SummarizeOk | SummarizeFailure | 'empty';

// Auth / 4xx / malformed requests won't fix on retry. Everything else
// (5xx, 429, network, timeout, unknown) gets one retry. Conservative
// default: when in doubt, retry.
export function isRetryableStreamError(err: unknown): boolean {
  if (!(err instanceof Error)) return true;
  const msg = err.message.toLowerCase();
  if (
    /\b(40[01345]|auth|unauthorized|forbidden|invalid[_ ]?(api[_ ]?key|key|request)|malformed)\b/.test(
      msg,
    )
  ) {
    return false;
  }
  return true;
}

export function renderUserPrompt(older: AgentMessage[]): string {
  let out =
    'Summarise the conversation below following the system-prompt structure exactly. Keep identifiers verbatim.\n\n';
  out += '<conversation>\n';
  for (const msg of older) {
    out += `\n[${msg.role}]\n`;
    for (const block of msg.content) {
      if ((block as { type?: string }).type === 'text') {
        out += `${(block as { type: 'text'; text: string }).text}\n`;
      } else if ((block as { type?: string }).type === 'function_call') {
        const fc = block as unknown as { function_id: string; arguments: unknown };
        out += `[tool_call] ${fc.function_id} ${JSON.stringify(fc.arguments)}\n`;
      }
    }
  }
  out += '</conversation>\n';
  return out;
}

export function estimateTokenCount(messages: AgentMessage[]): number {
  let chars = 0;
  for (const m of messages) chars += JSON.stringify(m).length;
  return Math.floor(chars / 4);
}

async function loadActiveWithIds(iii: ISdk, session_id: string): Promise<MessageWithEntryId[]> {
  const resp = await iii.trigger<
    unknown,
    { messages?: Array<{ entry_id?: string; message?: AgentMessage }> }
  >({
    function_id: 'session-tree::messages',
    payload: { session_id },
    timeoutMs: 10_000,
  });
  const arr = resp?.messages;
  if (!Array.isArray(arr)) throw new Error('session-tree::messages returned non-array');
  return arr
    .filter(
      (e): e is { entry_id: string; message: AgentMessage } =>
        typeof e?.entry_id === 'string' && Boolean(e?.message),
    )
    .map((e) => ({ entry_id: e.entry_id, message: e.message }));
}

async function loadCompactionEntries(
  iii: ISdk,
  session_id: string,
): Promise<CompactionEntryLike[]> {
  const resp = await iii.trigger<unknown, { entries?: CompactionEntryLike[] }>({
    function_id: 'session-tree::compactions',
    payload: { session_id },
    timeoutMs: 10_000,
  });
  const entries = resp?.entries;
  return Array.isArray(entries) ? entries : [];
}

async function appendCompaction(
  iii: ISdk,
  session_id: string,
  summary: string,
  tokens_before: number,
  tail_start_id: string | null,
): Promise<string> {
  const resp = await iii.trigger<unknown, { entry_id?: string }>({
    function_id: 'session-tree::compact',
    payload: {
      session_id,
      summary,
      tokens_before,
      tail_start_id,
      details: { read_files: [], modified_files: [] },
    },
    timeoutMs: 30_000,
  });
  return resp?.entry_id ?? '';
}

export async function summarizeAndAppend(
  iii: ISdk,
  session_id: string,
  opts: SummarizeOptions,
  model: { providerID: string; modelID: string; modelLimit: ModelLimit },
): Promise<SummarizeOutcome> {
  const entries = opts.truncatedEntries ?? (await loadActiveWithIds(iii, session_id));
  const prior = completedCompactions(await loadCompactionEntries(iii, session_id));
  const previousSummary = prior.at(-1)?.summary;

  const cfg = compactionConfig();
  const budget = preserveRecentBudget({
    model: { id: model.modelID, limit: model.modelLimit },
    override: cfg.preserveRecentTokensOverride,
  });

  const sel = selectWithEntryIds({
    entries,
    budget,
    tailTurns: cfg.tailTurns,
    estimate: estimateTokenCount,
  });
  if (sel.head.length === 0) {
    return 'empty';
  }

  const head_messages = sel.head.map((e) => e.message);
  const tail_messages: AgentMessage[] = entries.slice(sel.head.length).map((e) => e.message);
  const tokens_before = estimateTokenCount(head_messages);
  const stripped = stripMedia(head_messages, { toolOutputMaxChars: cfg.toolOutputMaxChars });

  const systemPrompt = buildPrompt({ previousSummary, context: [] });
  const userPrompt = renderUserPrompt(stripped);

  // One 250ms retry for transient failures (429/5xx/network). Permanent
  // failures (auth, malformed request) skip the retry to release the lease
  // sooner. The session's own provider/model are pinned explicitly so
  // /compact runs on exactly the provider the session streams on.
  const completeInput = {
    model: model.modelID,
    provider: model.providerID,
    system_prompt: systemPrompt,
    messages: [
      {
        role: 'user' as const,
        content: [{ type: 'text' as const, text: userPrompt }],
        timestamp: Date.now(),
      },
    ],
    tools: [],
  };
  let final: AssistantMessage;
  try {
    final = await routerComplete(iii, completeInput);
  } catch (firstErr) {
    const firstReason = firstErr instanceof Error ? firstErr.message : String(firstErr);
    if (!isRetryableStreamError(firstErr)) {
      logger.warn('summariser stream failed (non-retryable)', { session_id, err: firstReason });
      return { kind: 'compact', reason: firstReason };
    }
    logger.warn('summariser stream failed; retrying once', { session_id, err: firstReason });
    await new Promise<void>((resolve) => {
      setTimeout(resolve, 250);
    });
    try {
      final = await routerComplete(iii, completeInput);
    } catch (secondErr) {
      const reason = secondErr instanceof Error ? secondErr.message : String(secondErr);
      logger.warn('summariser stream failed after retry', {
        session_id,
        err: reason,
        firstErr: firstReason,
      });
      return { kind: 'compact', reason };
    }
  }

  let summary = '';
  for (const block of final.content) {
    if ((block as { type?: string }).type === 'text') {
      const t = (block as { type: 'text'; text: string }).text;
      summary += summary.length > 0 ? `\n${t}` : t;
    }
  }
  if (summary.length === 0) {
    logger.warn('summariser produced empty summary; skipping append', { session_id });
    return 'empty';
  }

  const tail_start_id = sel.tail_start_id ?? null;
  const compaction_entry_id = await appendCompaction(
    iii,
    session_id,
    summary,
    tokens_before,
    tail_start_id,
  );
  await stampLastCompaction(iii, session_id);
  return {
    kind: 'ok',
    tail_start_id,
    tokens_before,
    compaction_entry_id,
    summary_text: summary,
    tail_messages,
  };
}

/**
 * Run the summariser turn through `router::complete`. Pre-stream failures
 * (unknown provider, not configured) throw from the trigger; a mid-stream
 * failure comes back as an error-shaped AssistantMessage, surfaced here as a
 * throw so the caller's retry logic treats both uniformly.
 */
async function routerComplete(
  iii: ISdk,
  input: Record<string, unknown>,
): Promise<AssistantMessage> {
  const resp = await iii.trigger<unknown, { message?: AssistantMessage }>({
    function_id: 'router::complete',
    payload: input,
    timeoutMs: SUMMARIZER_TIMEOUT_MS,
  });
  const message = resp?.message;
  if (!message) {
    throw new Error('router::complete returned no message');
  }
  if (message.stop_reason === 'error') {
    const detail =
      typeof message.error_message === 'string' && message.error_message.length > 0
        ? message.error_message
        : extractTextFromMessage(message);
    throw new Error(`summariser stream error: ${detail || 'unknown provider error'}`);
  }
  return message;
}

function extractTextFromMessage(msg: AssistantMessage): string {
  for (const block of msg.content ?? []) {
    if ((block as { type?: string }).type === 'text') {
      return (block as { type: 'text'; text: string }).text;
    }
  }
  return '';
}
