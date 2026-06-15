/**
 * Compaction round-trip helpers — the harness side of the `context-manager`
 * contract (context-manager/architecture/integration.md §5, harness.md
 * § Compaction persistence).
 *
 * context-manager is stateless: it returns a budgeted context plus, when it
 * summarised, a summary + tail boundary. The harness OWNS persistence:
 *
 * 1. `loadAssembleWindow` reads the active path, finds the latest compaction
 *    bookkeeping entry, and returns the post-compaction message window (from
 *    `tail_start_entry_id` onward) plus its summary as `previousSummary`.
 *    Compaction custom entries are NEVER part of the window.
 * 2. `assembleContext` calls `context::assemble` with that window; on any
 *    failure it degrades to the raw window + base prompt (context-manager is a
 *    SOFT dependency — harness.md § Dependencies).
 * 3. `persistCompactionRoundTrip` writes a `kind:"custom"` bookkeeping entry
 *    (`custom_type: "compaction"`) when `applied.compacted`, mapping
 *    `applied.tail_start_index` onto the entry id at that position in the
 *    window we sent. The durable transcript itself is never modified.
 */

import type { ISdk } from './iii.js';
import { logger } from './otel.js';
import {
  COMPACTION_CUSTOM_TYPE,
  type CompactionData,
  type CompactionRow,
  type MessageWithEntryId,
  compactionRowsFromPath,
  messageItemsFromPath,
  readActivePath,
  sessionAppendCustom,
  type TranscriptItem,
} from './session.js';
import type { AgentMessage } from '../types/agent-message.js';
import type { Model } from '../types/model.js';

/** Outer budget: assemble/compact may make a summariser LLM call. */
const CONTEXT_TIMEOUT_MS = 320_000;

/** Inline limits accepted by `context::assemble` / `context::compact`. */
type ModelLimits = {
  context_window: number;
  max_output_tokens: number;
  input_limit?: number;
};

/** `context-manager` ModelInput (types.rs `ModelInput`). */
type ModelInput = {
  id: string;
  provider?: string;
  limits?: ModelLimits;
};

/** The subset of `context::assemble`'s `applied` block the harness reads. */
export type AppliedReport = {
  pruned: boolean;
  pruned_tokens: number;
  compacted: boolean;
  summary?: string;
  /** Index into the REQUEST messages where the tail begins; null = all summarised. */
  tail_start_index?: number | null;
  tokens_before?: number;
};

export type AssembleResult = {
  system_prompt: string;
  messages: AgentMessage[];
  applied: AppliedReport;
};

export type LoadWindowOptions = {
  /** Entry ids dropped from the window (this turn's assistant placeholder). */
  excludeEntryIds?: string[];
};

export type AssembleWindow = {
  window: MessageWithEntryId[];
  previousSummary?: string;
};

/**
 * Empty assistant messages are streaming placeholders (or the residue of a
 * run that died before its first delta); providers reject empty assistant
 * turns, so they never enter the window.
 */
function isEmptyAssistant(m: AgentMessage): boolean {
  return m.role === 'assistant' && m.content.length === 0;
}

/**
 * Latest compaction by path order — each record is appended at the
 * then-current leaf, so path order is chronological.
 */
function latestCompaction(compactions: CompactionRow[]): CompactionRow | null {
  return compactions.length > 0 ? (compactions.at(-1) ?? null) : null;
}

/**
 * Where the candidate window starts inside the path `items`:
 * - no compaction → 0 (the whole history).
 * - compaction with a `tail_start_entry_id` still on the path → that entry
 *   (the verbatim tail + everything appended after the compaction record).
 * - compaction with a null/missing boundary → right after the compaction
 *   record (the summary already covers everything before it; replaying it
 *   would double-count against `previous_summary`).
 */
function windowStartIndex(items: TranscriptItem[], compaction: CompactionRow): number {
  const afterCompaction = () => {
    const cIdx = items.findIndex((it) => it.entry_id === compaction.entry_id);
    return cIdx >= 0 ? cIdx + 1 : 0;
  };
  if (!compaction.tail_start_entry_id) return afterCompaction();
  const tailIdx = items.findIndex((it) => it.entry_id === compaction.tail_start_entry_id);
  return tailIdx >= 0 ? tailIdx : afterCompaction();
}

/**
 * The post-compaction message window plus the anchored summary. The window is
 * the raw messages (with entry ids) the harness sends to `context::assemble`;
 * the summary returns as `previousSummary` so a re-compaction updates it in
 * place instead of starting over.
 */
export async function loadAssembleWindow(
  iii: ISdk,
  session_id: string,
  opts?: LoadWindowOptions,
): Promise<AssembleWindow> {
  const items = await readActivePath(iii, session_id, { include_custom: true });
  const exclude = new Set(opts?.excludeEntryIds ?? []);
  const compaction = latestCompaction(compactionRowsFromPath(items));
  const start = compaction ? windowStartIndex(items, compaction) : 0;
  const window = messageItemsFromPath(items.slice(start)).filter(
    (e) => !exclude.has(e.entry_id) && !isEmptyAssistant(e.message),
  );
  return compaction ? { window, previousSummary: compaction.summary } : { window };
}

/** Build a `context-manager` ModelInput, preferring inline limits when known. */
export function buildModelInput(modelId: string, provider?: string, meta?: Model): ModelInput {
  const model: ModelInput = { id: modelId };
  if (provider) model.provider = provider;
  if (meta && typeof meta.context_window === 'number' && meta.context_window > 0) {
    model.limits = {
      context_window: meta.context_window,
      max_output_tokens: typeof meta.max_output_tokens === 'number' ? meta.max_output_tokens : 0,
      ...(typeof meta.input_limit === 'number' ? { input_limit: meta.input_limit } : {}),
    };
  }
  return model;
}

export type AssembleParams = {
  session_id: string;
  window: MessageWithEntryId[];
  baseSystemPrompt: string;
  modelId: string;
  provider?: string;
  modelMeta?: Model;
  thinkingLevel?: string;
  previousSummary?: string;
};

type AssembleResponse = {
  system_prompt?: string;
  messages?: AgentMessage[];
  applied?: AppliedReport;
};

/**
 * Build the model-ready context via `context::assemble`. Best-effort: any
 * failure (worker absent, trigger error, malformed response) degrades to the
 * raw window + base prompt with no compaction, so the turn still runs without
 * `context-manager` (harness.md § Dependencies — soft).
 */
export async function assembleContext(iii: ISdk, params: AssembleParams): Promise<AssembleResult> {
  const raw: AssembleResult = {
    system_prompt: params.baseSystemPrompt,
    messages: params.window.map((w) => w.message),
    applied: { pruned: false, pruned_tokens: 0, compacted: false },
  };

  const options: Record<string, unknown> = { lease_key: params.session_id };
  if (params.previousSummary) options.previous_summary = params.previousSummary;
  if (params.thinkingLevel && params.thinkingLevel !== 'off') {
    options.thinking_level = params.thinkingLevel;
  }

  try {
    const resp = await iii.trigger<unknown, AssembleResponse>({
      function_id: 'context::assemble',
      payload: {
        messages: raw.messages,
        model: buildModelInput(params.modelId, params.provider, params.modelMeta),
        system_prompt: params.baseSystemPrompt,
        options,
      },
      timeoutMs: CONTEXT_TIMEOUT_MS,
    });
    if (!resp || !Array.isArray(resp.messages) || typeof resp.system_prompt !== 'string') {
      logger.warn('context::assemble returned an unexpected shape; using raw window', {
        session_id: params.session_id,
      });
      return raw;
    }
    return {
      system_prompt: resp.system_prompt,
      messages: resp.messages,
      applied: resp.applied ?? raw.applied,
    };
  } catch (err) {
    logger.warn('context::assemble failed; falling back to raw window', {
      session_id: params.session_id,
      err: String(err),
    });
    return raw;
  }
}

/**
 * Persist the compaction round trip as an additive `kind:"custom"` bookkeeping
 * entry. Returns the new entry id, or null when nothing was compacted. The
 * transcript messages are never touched — this is loop bookkeeping the harness
 * owns (harness.md § Compaction persistence).
 */
export async function persistCompactionRoundTrip(
  iii: ISdk,
  session_id: string,
  applied: AppliedReport,
  window: MessageWithEntryId[],
): Promise<string | null> {
  if (!applied.compacted) return null;
  const idx = applied.tail_start_index;
  const tailEntry =
    typeof idx === 'number' && idx >= 0 && idx < window.length ? window[idx] : undefined;
  const tail_start_entry_id = tailEntry ? tailEntry.entry_id : null;
  const data: CompactionData = {
    summary: applied.summary ?? '',
    tokens_before: applied.tokens_before ?? 0,
    tail_start_entry_id,
    details: { read_files: [], modified_files: [] },
    timestamp: Date.now(),
  };
  const resp = await sessionAppendCustom(iii, {
    session_id,
    custom_type: COMPACTION_CUSTOM_TYPE,
    data,
  });
  return resp.entry_id;
}

/** Map console's `{ id, providerID, limit? }` shape onto a ModelInput. */
export function modelInputFromConsole(raw: unknown): ModelInput | null {
  if (!raw || typeof raw !== 'object') return null;
  const m = raw as {
    id?: unknown;
    providerID?: unknown;
    limit?: { context?: unknown; input?: unknown; output?: unknown };
  };
  if (typeof m.id !== 'string' || m.id.length === 0) return null;
  const model: ModelInput = { id: m.id };
  if (typeof m.providerID === 'string' && m.providerID.length > 0) model.provider = m.providerID;
  const lim = m.limit;
  if (lim && typeof lim.context === 'number' && lim.context > 0) {
    model.limits = {
      context_window: lim.context,
      max_output_tokens: typeof lim.output === 'number' ? lim.output : 0,
      ...(typeof lim.input === 'number' ? { input_limit: lim.input } : {}),
    };
  }
  return model;
}

export type CompactResponse =
  | {
      status: 'ok';
      summary: string;
      tail_start_index: number | null;
      tokens_before: number;
      tokens_after: number;
      used_prior_summary: boolean;
    }
  | { status: 'busy' }
  | { status: 'empty' }
  | { status: 'overflow' };

/** Call `context::compact` directly (used by the `/compact` wrapper). */
export async function compactWindow(
  iii: ISdk,
  session_id: string,
  window: MessageWithEntryId[],
  model: ModelInput,
  previousSummary?: string,
): Promise<CompactResponse> {
  const options: Record<string, unknown> = { lease_key: session_id };
  if (previousSummary) options.previous_summary = previousSummary;
  const resp = await iii.trigger<unknown, CompactResponse>({
    function_id: 'context::compact',
    payload: {
      messages: window.map((w) => w.message),
      model,
      options,
    },
    timeoutMs: CONTEXT_TIMEOUT_MS,
  });
  return resp ?? { status: 'overflow' };
}
