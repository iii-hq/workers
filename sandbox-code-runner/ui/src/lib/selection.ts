const KEY = 'sandbox-ui:selected'

let selected: string | null = null
const listeners = new Set<(id: string | null) => void>()

function selectSandbox(id: string | null) {
  selected = id
  try {
    if (id) sessionStorage.setItem(KEY, id)
    else sessionStorage.removeItem(KEY)
  } catch {
    /* storage unavailable */
  }
  for (const fn of listeners) fn(id)
}

export function takeSelectedSandbox(): string | null {
  if (selected) return selected
  try {
    return sessionStorage.getItem(KEY)
  } catch {
    return null
  }
}

export function onSandboxSelected(fn: (id: string | null) => void): () => void {
  listeners.add(fn)
  return () => listeners.delete(fn)
}

export function jumpToSandbox(id: string) {
  selectSandbox(id)
  window.location.hash = '#/ext/sandbox'
}
