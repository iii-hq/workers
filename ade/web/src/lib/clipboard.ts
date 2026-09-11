/**
 * Clipboard writes that survive insecure origins. The console is used over
 * `http://<LAN-IP>`, where `navigator.clipboard` is undefined (it is a
 * secure-context API) — so copy affordances funnel through this helper:
 * async clipboard API first, hidden-textarea `document.execCommand('copy')`
 * second.
 */

function execCommandFallback(text: string): boolean {
  if (typeof document === 'undefined') return false
  const prior = document.activeElement
  const textarea = document.createElement('textarea')
  textarea.value = text
  // Off-screen but selectable; readonly keeps mobile keyboards closed.
  textarea.setAttribute('readonly', '')
  textarea.style.position = 'fixed'
  textarea.style.left = '-9999px'
  document.body.appendChild(textarea)
  textarea.select()
  let ok = false
  try {
    ok = document.execCommand('copy')
  } catch {
    ok = false
  }
  textarea.remove()
  if (typeof HTMLElement !== 'undefined' && prior instanceof HTMLElement) {
    prior.focus()
  }
  return ok
}

/** True only when a strategy succeeded; never throws. */
export async function copyTextToClipboard(text: string): Promise<boolean> {
  if (typeof navigator !== 'undefined' && navigator.clipboard) {
    try {
      await navigator.clipboard.writeText(text)
      return true
    } catch {
      // Permissions can reject even on secure origins — try the fallback.
    }
  }
  return execCommandFallback(text)
}
