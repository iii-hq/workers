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
 * Whether the wizard opens by itself on load: on first run (`new`), or after
 * setup was finished or skipped when no model is connected (`connectedModels`
 * is 0; `null` when the router was not asked or could not answer). Never where
 * the ADE turned it off (`auto_open: false` — a deployed ADE, whose fresh data
 * directory reads as a first run), and never in a browser under automation
 * (`navigator.webdriver` — an e2e suite, an agent's browser session, a
 * stories render), which gets the page it asked for, not a modal over it.
 * Explicit requests open it either way.
 */
export function shouldAutoOpenOnboarding(
  state: { status: string | null | undefined; auto_open?: boolean },
  automated: boolean,
  connectedModels: number | null,
): boolean {
  if (state.auto_open === false || automated) return false
  return state.status === 'new' || connectedModels === 0
}

/** The browser reports it is driven by automation. */
export function browserIsAutomated(): boolean {
  return typeof navigator !== 'undefined' && navigator.webdriver === true
}
