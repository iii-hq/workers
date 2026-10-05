/**
 * Open the setup wizard from anywhere — the chat's "configure a provider"
 * call to action, the command palette — without threading a callback through
 * the tree. The wizard host mounted once in `App` is the only listener.
 */

export type WizardStepId = 'welcome' | 'machine' | 'models' | 'judge' | 'ready'

type Listener = (step: WizardStepId | undefined) => void

const listeners = new Set<Listener>()

export function requestOnboardingWizard(step?: WizardStepId): void {
  for (const listener of listeners) listener(step)
}

export function onOnboardingWizardRequest(listener: Listener): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

/** Whether a host is mounted to answer `requestOnboardingWizard`. */
export function onboardingWizardAvailable(): boolean {
  return listeners.size > 0
}
