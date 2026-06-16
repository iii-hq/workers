/**
 * Typed dependency ports for function_awaiting_approval. Decisions arrive
 * via `harness::function::resolve` rows in the `function_resolutions`
 * scope; the wake consumes (reads, applies, deletes) them.
 */

import type { ISdk } from '../../runtime/iii.js';
import { deleteResolution, readResolution, type FunctionResolution } from '../function-resolve.js';

export type AwaitingApprovalPorts = {
  readDecision(session_id: string, function_call_id: string): Promise<FunctionResolution | null>;
  deleteDecision(session_id: string, function_call_id: string): Promise<void>;
};

export function createAwaitingApprovalPorts(iii: ISdk): AwaitingApprovalPorts {
  return {
    readDecision(session_id, function_call_id) {
      return readResolution(iii, session_id, function_call_id);
    },
    deleteDecision(session_id, function_call_id) {
      return deleteResolution(iii, session_id, function_call_id);
    },
  };
}
