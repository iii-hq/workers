import type { View } from '@/hooks/use-hash-route'

/**
 * Header nav entries. Optional-worker surfaces appear only while their
 * worker is present, so the nav never advertises a page whose functions
 * don't exist (a direct hash hit still lands on that page's install
 * notice).
 */
export function buildViewOptions(
  browserAvailable: boolean,
  githubAvailable: boolean,
): { value: View; label: string }[] {
  const options: { value: View; label: string }[] = [
    { value: 'traces', label: 'traces' },
    { value: 'workers', label: 'workers' },
  ]
  if (browserAvailable) {
    options.push({ value: 'browser', label: 'browser' })
  }
  if (githubAvailable) {
    options.push({ value: 'github', label: 'github' })
  }
  return options
}
