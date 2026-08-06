/**
 * Minimal class-name joiner for the injected UI. The console's Tailwind `cn`
 * is not available here — injected UI ships its own scoped stylesheet with
 * semantic classes, so there are no utility classes to merge.
 */
export function cn(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(' ')
}
