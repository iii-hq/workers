/**
 * Minimal class-name joiner for the injected UI. The console's Tailwind
 * `cn` (clsx + tailwind-merge) is not available here — injected UI ships its
 * own scoped stylesheet with semantic classes, so there are no conflicting
 * utility classes to merge. This just filters falsy values and space-joins.
 */
export function cn(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(' ')
}
