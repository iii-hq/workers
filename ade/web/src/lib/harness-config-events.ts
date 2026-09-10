/** Fired after `configuration::set` succeeds for the llm-router entry (provider credentials + settings). */
const LLM_ROUTER_CONFIG_ID = 'llm-router'

type Listener = () => void

const listeners = new Set<Listener>()

export function onHarnessConfigSaved(listener: Listener): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export function notifyHarnessConfigSaved(configId: string): void {
  if (configId !== LLM_ROUTER_CONFIG_ID) return
  for (const listener of listeners) {
    listener()
  }
}
