/* A command typed into a terminal pane as soon as its shell opens, for the
   verbs that hand the user a terminal with something already running in it
   ("Sign in with browser" runs `gh auth login`). Keyed by pane, taken once:
   a pane that reattaches to its session never types it again. */

const pending = new Map<string, string>()

export function queueTerminalStartupCommand(paneId: string, command: string): void {
  pending.set(paneId, command)
}

export function takeTerminalStartupCommand(paneId: string): string | null {
  const command = pending.get(paneId)
  if (command === undefined) return null
  pending.delete(paneId)
  return command
}
