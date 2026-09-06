/** A load failure as a sentence, not a wire dump. The worker's read errors
    arrive as `handler error: {json}`; a missing file (deleted or moved after
    the tab opened) is the common case and deserves plain words — and its
    own pane state, so `isMissingFileError` tells it apart. */

export const MISSING_FILE_MESSAGE = 'this file no longer exists on disk: deleted or moved after this tab was opened'

function parseHandlerError(raw: string): { code?: string; message?: string } | null {
  const brace = raw.indexOf('{')
  if (brace === -1) return null
  try {
    return JSON.parse(raw.slice(brace)) as { code?: string; message?: string }
  } catch {
    return null
  }
}

/** True for the worker's "not found or not accessible" read failure
    (`C211`, which deliberately folds the two so a path never leaks). */
export function isMissingFileError(raw: string): boolean {
  const parsed = parseHandlerError(raw)
  if (parsed?.code === 'C211') return true
  const detail = typeof parsed?.message === 'string' ? parsed.message : raw
  return detail.includes('not found or not accessible')
}

export function loadErrorMessage(raw: string): string {
  if (isMissingFileError(raw)) return MISSING_FILE_MESSAGE
  const parsed = parseHandlerError(raw)
  if (parsed && typeof parsed.message === 'string' && parsed.message.length > 0) return parsed.message
  return raw
}
