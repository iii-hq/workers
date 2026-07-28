import type { View } from '@/hooks/use-hash-route'

/**
 * Header nav entries. Every optional per-worker surface moved to injected UI
 * (`#/ext/<id>`), so the first-party nav is just traces + workers; injected
 * pages are appended by the caller from the live script registry.
 */
export function buildViewOptions(): { value: View; label: string }[] {
  return [
    { value: 'traces', label: 'traces' },
    { value: 'workers', label: 'workers' },
  ]
}
