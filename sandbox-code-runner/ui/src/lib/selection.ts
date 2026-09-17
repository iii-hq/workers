import type { Host } from '@iii-dev/console-ui'

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

let boundHost: Host | null = null

/** Bound once in `setup(host)`: the chat chips that jump have no host. */
export function bindSandboxHost(host: Host): void {
  boundHost = host
}

/** Select a sandbox and open the fleet page on it (the page reads the
    selection on mount and again through `panelContext`). */
export function jumpToSandbox(id: string) {
  selectSandbox(id)
  boundHost?.panels?.open({ pageId: 'sandbox', context: { sandboxId: id } })
}
