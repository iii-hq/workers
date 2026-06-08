import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { compactionConfig } from './config.js';
import { handleAsync } from './handler-async.js';
import { type CompactNowInput, handleSync } from './handler-sync.js';
import { acquireLease, releaseLease } from './lease.js';
import {
  fetchModelLimit,
  resolveModelFromRunRequest,
  resolveModelFromSession,
} from './model-resolver.js';
import { prune } from './prune.js';

// Sized so preserveRecentBudget clamps to its 2k minimum when the real
// model is unknown — compaction is best-effort, not fatal.
const FALLBACK_MODEL_LIMIT = {
  context: 32_000,
  input: 32_000,
  output: 4_000,
};
const FALLBACK_MODEL_ID = 'unknown';
const FALLBACK_PROVIDER_ID = 'unknown';

// payload.model: { id, providerID } with optional limit; null on malformed.
async function resolveExplicitModel(
  iii: ISdk,
  raw: unknown,
): Promise<{
  providerID: string;
  modelID: string;
  modelLimit: { context: number; input: number; output: number };
} | null> {
  if (!raw || typeof raw !== 'object') return null;
  const m = raw as Record<string, unknown>;
  const providerID = typeof m.providerID === 'string' && m.providerID ? m.providerID : null;
  const modelID = typeof m.id === 'string' && m.id ? m.id : null;
  if (!providerID || !modelID) return null;
  const lim = m.limit as { context?: number; input?: number; output?: number } | undefined;
  if (lim && typeof lim.context === 'number' && lim.context > 0) {
    return {
      providerID,
      modelID,
      modelLimit: {
        context: lim.context,
        input: typeof lim.input === 'number' ? lim.input : lim.context,
        output: typeof lim.output === 'number' ? lim.output : 0,
      },
    };
  }
  return fetchModelLimit(iii, providerID, modelID);
}

export async function register(iii: ISdk): Promise<void> {
  iii.registerFunction(
    'context-compaction::on_turn_end',
    async (payload: unknown) => {
      await handleAsync(iii, payload);
      return null;
    },
    {
      description:
        'Internal: woken by a turn-orchestrator queue message at turn_end; triggers async compaction when running tokens exceed usable(model).',
    },
  );

  iii.registerFunction(
    'context-compaction::compact_now',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;

      const session_id_raw = obj.session_id;
      if (typeof session_id_raw !== 'string' || session_id_raw.length === 0) {
        throw new Error('context-compaction::compact_now: session_id is required');
      }
      const session_id = session_id_raw;

      const modelObj = (obj.model ?? {}) as {
        id?: string;
        providerID?: string;
        limit?: { context?: number; input?: number; output?: number };
      };
      const input: CompactNowInput = {
        session_id,
        projected_tokens: typeof obj.projected_tokens === 'number' ? obj.projected_tokens : 0,
        last_user_message_id: String(obj.last_user_message_id ?? ''),
        model: {
          id: String(modelObj.id ?? ''),
          providerID: String(modelObj.providerID ?? ''),
          limit: {
            context: modelObj.limit?.context ?? 0,
            input: modelObj.limit?.input ?? 0,
            output: modelObj.limit?.output ?? 0,
          },
        },
      };
      return handleSync(iii, input);
    },
    {
      description:
        'Sync pre-turn compaction triggered by turn-orchestrator when a turn would overflow. Performs prune+summarise+reinject+continue.',
    },
  );

  iii.registerFunction(
    'context-compaction::prune_tool_outputs',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const session_id_raw = obj.session_id;
      if (typeof session_id_raw !== 'string' || session_id_raw.length === 0) {
        throw new Error('context-compaction::prune_tool_outputs: session_id is required');
      }
      const session_id = session_id_raw;
      const nonce = await acquireLease(iii, session_id, 'prune');
      if (!nonce) return { pruned_tokens: 0, pruned_parts: 0, scanned_parts: 0, busy: true };
      try {
        const cfg = compactionConfig();
        return await prune(iii, session_id, {
          protectTokens: cfg.pruneProtect,
          minFree: cfg.pruneMinFree,
          protectedTools: cfg.pruneProtectedTools,
        });
      } finally {
        await releaseLease(iii, session_id, nonce, 'prune');
      }
    },
    { description: 'Prune older tool outputs without summarisation (cheap path).' },
  );

  iii.registerFunction(
    'context-compaction::compact_session',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const session_id_raw = obj.session_id;
      if (typeof session_id_raw !== 'string' || session_id_raw.length === 0) {
        throw new Error('context-compaction::compact_session: session_id is required');
      }
      const session_id = session_id_raw;

      const explicitObj = obj.model as { id?: string; providerID?: string } | undefined;
      let model = await resolveExplicitModel(iii, obj.model);
      if (!model) model = await resolveModelFromSession(iii, session_id);
      if (!model) model = await resolveModelFromRunRequest(iii, session_id);
      if (!model) {
        const providedId =
          typeof explicitObj?.id === 'string' && explicitObj.id ? explicitObj.id : null;
        const providedProvider =
          typeof explicitObj?.providerID === 'string' && explicitObj.providerID
            ? explicitObj.providerID
            : null;
        logger.warn('compact_session: model resolution failed; using fallback limit', {
          session_id,
          requestedProvider: providedProvider,
          requestedModel: providedId,
        });
        model = {
          providerID: providedProvider ?? FALLBACK_PROVIDER_ID,
          modelID: providedId ?? FALLBACK_MODEL_ID,
          modelLimit: FALLBACK_MODEL_LIMIT,
        };
      }

      // Empty last_user_message_id skips handleSync's replay branch.
      // Replay belongs to compact_now (orchestrator overflow); /compact
      // runs against a conversation at rest with nothing to re-inject.
      return handleSync(iii, {
        session_id,
        projected_tokens: 999_999,
        last_user_message_id: '',
        model: {
          id: model.modelID,
          providerID: model.providerID,
          limit: model.modelLimit,
        },
      });
    },
    {
      description:
        'User-initiated synchronous compaction of a session. Required: session_id. Optional: model { id, providerID, limit? } to skip auto-resolution. If model is omitted, falls back to (1) most recent assistant message in session-tree, (2) orchestrator run_request.',
    },
  );
}
