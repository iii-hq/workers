import { Button } from '@/components/ui/Button'
import { mentionsChromiumMissing } from '@/lib/onboarding/chromium'
import {
  onboardingWizardAvailable,
  requestOnboardingWizard,
} from '@/lib/onboarding/open'

/**
 * A browser call failed because the machine has no Chromium (its error
 * carries the worker's `chromium_missing` marker).
 */
export function isChromiumMissingCall(
  functionId: string,
  output: unknown,
): boolean {
  return functionId.startsWith('browser::') && mentionsChromiumMissing(output)
}

/**
 * Under a browser tool error that says Chromium is missing: one button to
 * the setup wizard's Browser step, where it downloads in a click.
 */
export function ChromiumMissingAction() {
  if (!onboardingWizardAvailable()) return null
  return (
    <div
      data-chromium-missing=""
      className="flex flex-wrap items-center gap-2 border-t border-rule-2 px-3 py-2"
    >
      <p className="min-w-0 flex-1 font-sans text-[13px] text-ink">
        The browser worker has no Chromium to open pages with.
      </p>
      <Button size="sm" onClick={() => requestOnboardingWizard('browser')}>
        Install Chromium
      </Button>
    </div>
  )
}
