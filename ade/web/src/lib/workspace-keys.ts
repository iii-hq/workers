/**
 * The workspace's own chords, handed back by a field that holds the caret
 * (see `allowsWhileTyping`): modifier chords that never spell text — the
 * launcher's page search and the chat composer both carry it, so ⌃T, ⌃W,
 * ⌃⇧←/→ and the split keys work wherever the caret is.
 */
export const WORKSPACE_KEYS_WHILE_SEARCHING = [
  'workspace.selectByIndex',
  'workspace.create',
  'workspace.close',
  'workspace.next',
  'workspace.previous',
  'panel.split',
  'panel.splitLeft',
  'panel.next',
  'panel.previous',
].join(' ')
