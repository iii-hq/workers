import type { AuthStatusResult } from '@/lib/providers'

interface StatusBadgeProps {
  status: AuthStatusResult | undefined
  isLoading: boolean
  /** True when the status query failed (e.g., harness unreachable). */
  isError?: boolean
  /** Env var name for tooltip context when source === 'environment'. */
  envVar?: string
}

/**
 * Four states: loading, error, configured, unconfigured. Each has
 * accessible semantics: `role="status"` + `aria-live="polite"` on the
 * loading state so screen readers announce when status arrives;
 * `role="alert"` on error; `title` tooltips on the bracketed badges
 * so operators who don't immediately know what `[stored]` vs `[env]`
 * means get an inline explanation on hover. Distinguishing the error
 * state from "not set" matters operationally — a network-partitioned
 * machine would otherwise look like an unconfigured one.
 */
export function StatusBadge({
  status,
  isLoading,
  isError,
  envVar,
}: StatusBadgeProps) {
  if (isError) {
    return (
      <span
        role="alert"
        className="font-mono text-[11px] text-alert"
        title="couldn't reach the harness to read this provider's status"
      >
        [unreachable]
      </span>
    )
  }
  if (isLoading) {
    return (
      <span
        role="status"
        aria-live="polite"
        aria-label="loading provider status"
        className="font-mono text-[11px] text-ink-faint inline-flex items-center gap-1"
      >
        <span
          aria-hidden
          className="inline-block w-1.5 h-1.5 rounded-full bg-ink-ghost animate-pulse"
        />
        loading
      </span>
    )
  }

  if (!status?.configured) {
    return (
      <span
        className="font-mono text-[11px] text-ink-ghost"
        title="no key is set for this provider yet"
      >
        not set
      </span>
    )
  }

  const isStored = status.source === 'stored'
  // `[stored]` earns the accent. `[env-var]` was previously `text-ink-faint`,
  // which made it visually indistinguishable from surrounding muted prose.
  // Lift to full `text-ink` so the badge reads as a distinct token —
  // operators scanning the column see "this provider's key source" as
  // foreground content, not annotation noise.
  const badgeClass = isStored ? 'text-accent' : 'text-ink'
  const tooltip = isStored
    ? 'you set this key here. the harness reads it from its own store.'
    : envVar
      ? `inherited from the ${envVar} environment variable; takes precedence over a stored key.`
      : 'inherited from an environment variable; takes precedence over a stored key.'

  return (
    <span
      className="inline-flex items-center gap-1.5 font-mono text-[11px] min-w-0"
      title={tooltip}
    >
      <span className={badgeClass}>[{isStored ? 'stored' : 'env-var'}]</span>
      {status.label ? (
        <span className="text-ink-faint truncate">{status.label}</span>
      ) : null}
    </span>
  )
}
