/** Fired after `configuration::set` succeeds for the harness entry. */
const HARNESS_CONFIG_ID = 'harness'

type Listener = () => void

const listeners = new Set<Listener>()

export function onHarnessConfigSaved(listener: Listener): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export function notifyHarnessConfigSaved(configId: string): void {
  if (configId !== HARNESS_CONFIG_ID) return
  for (const listener of listeners) {
    listener()
  }
}
