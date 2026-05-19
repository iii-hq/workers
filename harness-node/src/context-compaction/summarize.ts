import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { AgentMessage, AssistantMessage } from '../types/agent-message.js';
import {
  preserveRecentTokensOverride,
  summarizerModel,
  summarizerProvider,
  tailTurns,
  toolOutputMaxChars,
} from './config.js';
import { stampLastCompaction } from './lease.js';
import { type ModelLimit, preserveRecentBudget } from './overflow.js';
import {
  type CompactionEntryLike,
  type MessageWithEntryId,
  completedCompactions,
  selectWithEntryIds,
} from './selection.js';
import { streamAndCollect } from './stream-collect.js';
import { stripMedia } from './strip-media.js';
import { buildPrompt } from './template.js';

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
    timeoutMs: 10_000,
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

  const budget = preserveRecentBudget({
    model: { id: model.modelID, limit: model.modelLimit },
    override: preserveRecentTokensOverride(),
  });

  const sel = selectWithEntryIds({
    entries,
    budget,
    tailTurns: tailTurns(),
    estimate: estimateTokenCount,
  });
  if (sel.head.length === 0) {
    return 'empty';
  }

  const head_messages = sel.head.map((e) => e.message);
  const tail_messages: AgentMessage[] = entries.slice(sel.head.length).map((e) => e.message);
  const tokens_before = estimateTokenCount(head_messages);
  const stripped = stripMedia(head_messages, { toolOutputMaxChars: toolOutputMaxChars() });

  const systemPrompt = buildPrompt({ previousSummary, context: [] });
  const userPrompt = renderUserPrompt(stripped);

  const providerName = summarizerProvider() ?? 'anthropic';
  const summariserId =
    providerName === 'openai' ? 'provider::openai::stream' : 'provider::anthropic::stream';
  const modelId = summarizerModel() ?? model.modelID;

  let final: AssistantMessage;
  try {
    final = await streamAndCollect(
      iii,
      {
        system_prompt: systemPrompt,
        model: modelId,
        messages: [
          {
            role: 'user',
            content: [{ type: 'text', text: userPrompt }],
            timestamp: Date.now(),
          },
        ],
        tools: [],
      },
      summariserId,
    );
  } catch (err) {
    const reason = err instanceof Error ? err.message : String(err);
    logger.warn('summariser stream failed', { session_id, err: reason });
    return { kind: 'compact', reason };
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
