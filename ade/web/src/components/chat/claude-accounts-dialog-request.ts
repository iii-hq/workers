/**
 * Opening "Claude Code accounts" from anywhere in a chat pane (the picker's
 * "Open in Settings", the account chip, the limit card's "Other…"): one host
 * renders the dialog, the most recently mounted one, as the project dialog's
 * request does. Settings renders its own.
 */
export type ClaudeAccountsTab = 'accounts' | 'projects'

type Listener = (tab: ClaudeAccountsTab) => void

const hosts: Listener[] = []

export function requestClaudeAccountsDialog(
  tab: ClaudeAccountsTab = 'accounts',
): boolean {
  const host = hosts.at(-1)
  if (!host) return false
  host(tab)
  return true
}

export function onClaudeAccountsDialogRequest(listener: Listener): () => void {
  hosts.push(listener)
  return () => {
    const index = hosts.lastIndexOf(listener)
    if (index !== -1) hosts.splice(index, 1)
  }
}
