/**
 * Typed dependency ports for provisioning.
 */

import { logger } from '../../runtime/otel.js';
import type { ISdk } from '../../runtime/iii.js';
import type { TurnOrchestratorConfig } from '../config.js';
import type { RunRequest } from '../run-request.js';
import { createTurnStore } from '../state-runtime/store.js';

const FETCH_TIMEOUT_MS = 10_000;

/** Decode directory skill responses from iii trigger payloads. */
export function parseDirectoryBody(resp: unknown): string | null {
  if (typeof resp === 'string') return resp;
  if (resp && typeof resp === 'object') {
    const body = (resp as { body?: unknown }).body;
    if (typeof body === 'string') return body;
  }
  return null;
}

export type ProvisioningPorts = {
  defaultSkillUris: readonly string[];
  loadRunRequest(session_id: string): Promise<RunRequest>;
  saveRunRequest(session_id: string, request: RunRequest): Promise<void>;
  fetchSkillsIndex(): Promise<string | null>;
  fetchSkillBody(id: string): Promise<string | null>;
};

export function createProvisioningPorts(
  iii: ISdk,
  cfg: TurnOrchestratorConfig,
): ProvisioningPorts {
  const store = createTurnStore(iii);

  return {
    defaultSkillUris: cfg.system_default_skills,

    loadRunRequest(session_id) {
      return store.loadRunRequest(session_id);
    },

    saveRunRequest(session_id, request) {
      return store.saveRunRequest(session_id, request);
    },

    async fetchSkillsIndex() {
      try {
        const resp = await iii.trigger<unknown, unknown>({
          function_id: 'directory::skills::index',
          payload: {},
          timeoutMs: FETCH_TIMEOUT_MS,
        });
        const body = parseDirectoryBody(resp);
        return body && body.length > 0 ? body : null;
      } catch (err) {
        logger.warn('directory::skills::index failed', { err: String(err) });
        return null;
      }
    },

    async fetchSkillBody(id) {
      try {
        const resp = await iii.trigger<unknown, unknown>({
          function_id: 'directory::skills::get',
          payload: { id },
          timeoutMs: FETCH_TIMEOUT_MS,
        });
        return parseDirectoryBody(resp);
      } catch (err) {
        logger.warn('directory::skills::get failed', { id, err: String(err) });
        return null;
      }
    },
  };
}
