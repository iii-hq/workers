/* One shell page can be open several times inside a workspace tab, each
   pane on its own folder with its own terminals. Everything an instance
   keeps for itself (persisted UI state, terminal leases, the engine
   functions its live triggers call) is keyed by the pane, not the tab. */

/** The key one page instance keeps its state under: the pane id when the
    console provides one, else the workspace tab id (older consoles host
    one pane per tab, and that is what their saves were keyed by). */
export function paneStateKey(tabId: string, paneId: string | undefined): string {
  return typeof paneId === 'string' && paneId !== '' ? paneId : tabId
}

/** The key as an engine function-id segment: the engine's ids are `::`
    separated words, and the console's pane ids carry single colons. */
export function paneScopeToken(key: string): string {
  return key.replace(/[^A-Za-z0-9_-]+/g, '-')
}
