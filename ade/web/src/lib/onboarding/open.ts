/**
 * Open the setup wizard from anywhere — the chat's "configure a provider"
 * call to action, the command palette — without threading a callback through
 * the tree. The wizard host mounted once in `App` is the only listener.
 */

export type WizardStepId = 'welcome' | 'models' | 'judge' | 'ready'

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

/**
 * Whether the wizard may open by itself on load, before asking the router
 * anything. Only for a person on first run: never where the ADE turned it
 * off (`auto_open: false` — a deployed ADE, whose fresh data directory reads
 * as a first run), and never in a browser under automation
 * (`navigator.webdriver` — an e2e suite, an agent's browser session, a
 * stories render), which gets the page it asked for, not a modal over it.
 *
 * The host then opens it only when no model is connected yet: a project
 * whose providers already serve models is set up. Explicit requests open it
 * either way.
 */
export function shouldAutoOpenOnboarding(
  state: { status: string | null | undefined; auto_open?: boolean },
  automated: boolean,
): boolean {
  return state.status === 'new' && state.auto_open !== false && !automated
}

/** The browser reports it is driven by automation. */
export function browserIsAutomated(): boolean {
  return typeof navigator !== 'undefined' && navigator.webdriver === true
}
