import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { ModelLimit } from './overflow.js';

export type ResolvedModel = {
  providerID: string;
  modelID: string;
  modelLimit: ModelLimit;
} | null;

export async function fetchModelLimit(
  iii: ISdk,
  providerID: string,
  modelID: string,
): Promise<ResolvedModel> {
  try {
    const entry = await iii.trigger<
      unknown,
      {
        id?: string;
        provider?: string;
        context_window?: number;
        max_output_tokens?: number;
      } | null
    >({
      function_id: 'models::get',
      payload: { provider: providerID, model_id: modelID },
      timeoutMs: 5_000,
    });

    if (!entry) {
      logger.debug('model-resolver: model not found in catalog', { providerID, modelID });
      return null;
    }

    const context = typeof entry.context_window === 'number' ? entry.context_window : 0;
    const output = typeof entry.max_output_tokens === 'number' ? entry.max_output_tokens : 0;

    return {
      providerID,
      modelID,
      modelLimit: { context, input: context, output },
    };
  } catch (err) {
    logger.debug('model-resolver: models::get failed', {
      providerID,
      modelID,
      err: String(err),
    });
    return null;
  }
}

export async function resolveModelFromSession(
  iii: ISdk,
  session_id: string,
): Promise<ResolvedModel> {
  try {
    const resp = await iii.trigger<
      unknown,
      { messages?: Array<{ entry_id?: string; message?: Record<string, unknown> }> }
    >({
      function_id: 'session-tree::messages',
      payload: { session_id },
      timeoutMs: 10_000,
    });

    const messages = resp?.messages ?? [];
    let providerID: string | null = null;
    let modelID: string | null = null;

    for (let i = messages.length - 1; i >= 0; i--) {
      const m = messages[i]?.message;
      if (!m || m.role !== 'assistant') continue;
      if (!providerID && typeof m.provider === 'string' && m.provider) providerID = m.provider;
      if (!modelID && typeof m.model === 'string' && m.model) modelID = m.model;
      if (providerID && modelID) break;
    }

    if (!providerID || !modelID) return null;

    return fetchModelLimit(iii, providerID, modelID);
  } catch (err) {
    logger.debug('model-resolver: session scan failed', { session_id, err: String(err) });
    return null;
  }
}

// Fallback when no assistant message carries provider/model yet (first-turn
// sessions, error-only sessions). The orchestrator writes run_request at
// agent::session/<sid>/run_request during run::start.
export async function resolveModelFromRunRequest(
  iii: ISdk,
  session_id: string,
): Promise<ResolvedModel> {
  try {
    const req = await iii.trigger<unknown, { provider?: string; model?: string } | null>({
      function_id: 'state::get',
      payload: { scope: 'agent', key: `session/${session_id}/run_request` },
      timeoutMs: 5_000,
    });
    const providerID = typeof req?.provider === 'string' && req.provider ? req.provider : null;
    const modelID = typeof req?.model === 'string' && req.model ? req.model : null;
    if (!providerID || !modelID) return null;
    return fetchModelLimit(iii, providerID, modelID);
  } catch (err) {
    logger.debug('model-resolver: run_request fetch failed', { session_id, err: String(err) });
    return null;
  }
}
