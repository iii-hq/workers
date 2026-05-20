import type { ISdk } from '../runtime/iii.js';
import { pruneMinFree, pruneProtect, pruneProtectedTools } from './config.js';
import { handleAsync } from './handler-async.js';
import { type CompactNowInput, handleSync } from './handler-sync.js';
import { acquireLease, releaseLease } from './lease.js';
import {
  fetchModelLimit,
  resolveModelFromRunRequest,
  resolveModelFromSession,
} from './model-resolver.js';
import { prune } from './prune.js';

const AGENT_EVENTS_STREAM = 'agent::events';

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
    'context-compaction::on_agent_event',
    async (frame: unknown) => {
      await handleAsync(iii, frame);
      return null;
    },
    {
      description:
        'Internal: subscribes to agent::events; triggers async compaction on TurnEnd when running tokens exceed usable(model).',
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
        return await prune(iii, session_id, {
          protectTokens: pruneProtect(),
          minFree: pruneMinFree(),
          protectedTools: pruneProtectedTools(),
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

      // First-hit chain: explicit payload → session-tree → run_request.
      // The last fallback covers first-turn sessions with no mirrored assistant.
      let model = await resolveExplicitModel(iii, obj.model);
      if (!model) model = await resolveModelFromSession(iii, session_id);
      if (!model) model = await resolveModelFromRunRequest(iii, session_id);
      if (!model) {
        throw new Error(
          `context-compaction::compact_session: could not resolve model for session ${session_id}`,
        );
      }

      const resp = await iii.trigger<
        unknown,
        { messages?: Array<{ entry_id?: string; message?: { role?: string } }> }
      >({
        function_id: 'session-tree::messages',
        payload: { session_id },
        timeoutMs: 10_000,
      });
      const messages = resp?.messages ?? [];
      let last_user_message_id = '';
      for (let i = messages.length - 1; i >= 0; i--) {
        if (messages[i]?.message?.role === 'user') {
          last_user_message_id = messages[i]?.entry_id ?? '';
          break;
        }
      }

      return handleSync(iii, {
        session_id,
        projected_tokens: 999_999,
        last_user_message_id,
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

  iii.registerTrigger({
    type: 'stream',
    function_id: 'context-compaction::on_agent_event',
    config: { stream_name: AGENT_EVENTS_STREAM },
  });
}
