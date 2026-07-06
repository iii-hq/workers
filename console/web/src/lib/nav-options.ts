import type { View } from '@/hooks/use-hash-route'

/**
 * Header nav entries. Optional-worker surfaces appear only while their
 * worker is present, so the nav never advertises a page whose functions
 * don't exist (a direct hash hit still lands on that page's install
 * notice).
 */
export function buildViewOptions(
  worktreeAvailable: boolean,
): { value: View; label: string }[] {
  const options: { value: View; label: string }[] = [
    { value: 'traces', label: 'traces' },
    { value: 'traces-v2', label: 'traces v2' },
    { value: 'workers', label: 'workers' },
  ]
  if (worktreeAvailable) {
    options.push({ value: 'worktrees', label: 'worktrees' })
  }
  return options
}
