/**
 * "Is this function registered right now?" answered from the cached
 * `engine::functions::list` catalog (10 s TTL, see `functions-catalog.ts`).
 *
 * The console calls into OPTIONAL workers — shell, editor, the worker
 * supervisor — from plain async helpers that have no React presence hook to
 * lean on. Calling a function nobody registered makes the engine log
 * `[ERROR] Function not found` every time; a project without those workers
 * accumulated 1 296 such lines in 50 minutes, most of it the console. One
 * cheap catalog read per TTL window avoids the whole class.
 *
 * Fail-open: when the catalog itself cannot be read (engine unreachable) the
 * caller proceeds exactly as before and surfaces its own error.
 */
import { fetchFunctionsCatalog } from './functions-catalog'

export async function functionRegistered(functionId: string): Promise<boolean> {
  try {
    const entries = await fetchFunctionsCatalog()
    return entries.some((entry) => entry.id === functionId)
  } catch {
    return true
  }
}
