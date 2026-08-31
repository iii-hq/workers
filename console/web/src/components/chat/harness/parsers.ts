/**
 * Helpers for the harness tool family.
 */
import { unwrapEnvelope } from '@/components/chat/sandbox/parsers'

export { unwrapEnvelope }

/* Synthetic + namespaced harness tools. `submit_result` is the
   output-contract fallback the harness injects into a turn;
   `harness::spawn` is the sub-agent pending trigger — it renders through
   the plain default view, but still carries the id so its label brands
   `harness::`. */
export const HARNESS_FUNCTION_IDS = ['submit_result', 'harness::spawn'] as const
export type HarnessFunctionId = (typeof HARNESS_FUNCTION_IDS)[number]

const HARNESS_FUNCTION_ID_SET: ReadonlySet<string> = new Set<string>(
  HARNESS_FUNCTION_IDS,
)

export function isHarnessFunction(id: string): id is HarnessFunctionId {
  return HARNESS_FUNCTION_ID_SET.has(id)
}
