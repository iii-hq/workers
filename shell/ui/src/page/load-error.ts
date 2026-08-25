/** A load failure as a sentence, not a wire dump. The worker's read errors
    arrive as `handler error: {json}`; a missing file (deleted or moved after
    the tab opened) is the common case and deserves plain words. */
export function loadErrorMessage(raw: string): string {
  const brace = raw.indexOf('{')
  if (brace !== -1) {
    try {
      const parsed = JSON.parse(raw.slice(brace)) as {
        code?: string
        message?: string
      }
      const detail = typeof parsed.message === 'string' ? parsed.message : ''
      if (parsed.code === 'C211' || detail.includes('not found or not accessible')) {
        return 'this file no longer exists on disk: deleted or moved after this tab was opened'
      }
      if (detail.length > 0) return detail
    } catch {
      // not JSON after all: fall through to the raw text
    }
  }
  if (raw.includes('not found or not accessible')) {
    return 'this file no longer exists on disk: deleted or moved after this tab was opened'
  }
  return raw
}
