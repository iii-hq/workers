import { useEffect } from 'react'
import { ProviderRow } from '@/components/providers/ProviderRow'
import {
  ACTIVE_PROVIDERS,
  ENV_VAR_MAP,
} from '@/components/providers/provider-registry'
import { ModeToggle } from '@/components/ui/ModeToggle'
import { useProviderStatuses } from '@/hooks/use-providers'
import type { Theme } from '@/hooks/use-theme'

const ENV_VAR_BY_ID = new Map(ENV_VAR_MAP)

interface ConfigurationProps {
  theme: Theme
  onThemeChange: (next: Theme) => void
}

export function Configuration({ theme, onThemeChange }: ConfigurationProps) {
  // Aggregate provider statuses to detect the zero-config empty state.
  // React Query dedupes by query key, so this shares cache with each row's
  // own `useAuthStatus` call — no extra fetches.
  const statuses = useProviderStatuses(ACTIVE_PROVIDERS)
  const isStatusLoading = statuses.some((s) => s.isLoading)
  const configuredCount = statuses.filter((s) => s.data?.configured).length
  const showZeroConfig = !isStatusLoading && configuredCount === 0

  // H7 — keyboard navigation for the provider list:
  //   1–N           : open provider N's dialog directly
  //   ↑ / ↓         : move focus between provider rows (when one is focused)
  // Both are suppressed inside text inputs / contenteditable so operators
  // can type freely in dialogs and the chat dock.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null
      if (target) {
        const tag = target.tagName
        if (
          tag === 'INPUT' ||
          tag === 'TEXTAREA' ||
          target.isContentEditable
        ) {
          return
        }
      }
      if (e.metaKey || e.ctrlKey || e.altKey) return

      // Arrow-nav between rows when a row currently has focus.
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        const rows = Array.from(
          document.querySelectorAll<HTMLElement>('[data-provider-row]'),
        )
        const activeIdx = target ? rows.indexOf(target) : -1
        if (activeIdx === -1 || rows.length === 0) return
        e.preventDefault()
        const nextIdx =
          e.key === 'ArrowDown'
            ? Math.min(activeIdx + 1, rows.length - 1)
            : Math.max(activeIdx - 1, 0)
        rows[nextIdx]?.focus()
        return
      }

      // Number-key open.
      const idx = Number.parseInt(e.key, 10)
      if (!Number.isFinite(idx) || idx < 1 || idx > ACTIVE_PROVIDERS.length) {
        return
      }
      const provider = ACTIVE_PROVIDERS[idx - 1]
      e.preventDefault()
      window.dispatchEvent(
        new CustomEvent('iii:open-provider', { detail: provider }),
      )
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  return (
    <main className="flex-1 overflow-y-auto" aria-label="configuration">
      <div className="mx-auto max-w-3xl px-6 py-10">
        <header className="pb-2">
          <h1 className="font-mono text-[20px] text-ink lowercase tracking-[-0.01em]">
            configuration
          </h1>
        </header>

        <Section
          title="appearance"
          description="theme preference, stored per browser."
        >
          <Row
            label="theme"
            control={
              <ModeToggle<Theme>
                value={theme}
                onChange={onThemeChange}
                variant="radio"
                aria-label="theme"
                options={[
                  { value: 'light', label: 'light' },
                  { value: 'dark', label: 'dark' },
                ]}
              />
            }
          />
        </Section>

        <Section
          title="providers"
          description="api keys and runtime overrides."
        >
          {showZeroConfig ? (
            <div
              role="status"
              aria-live="polite"
              className="py-4 border-b border-rule"
            >
              <p className="font-mono text-[13px] text-ink">
                <span className="text-accent" aria-hidden>
                  ·
                </span>{' '}
                no api keys configured yet
              </p>
              <p className="font-mono text-[11px] text-ink-faint mt-1">
                open any provider below to add a key. local providers
                (lmstudio, llamacpp) don't need one.
              </p>
            </div>
          ) : null}
          {/* Legend renders BEFORE the rows so operators meet the
              vocabulary (`[stored]`, `[env]`, `[local]`) before they
              encounter the badges that use it. Lifted to `text-[12px]
              text-ink` (from earlier `text-ink-faint`) so it reads as
              instruction, not annotation — H6 critiques flagged the
              ink-faint version as visually skipped.
              The legend itself is where badge vocabulary is taught; the
              keyboard shortcut hint shares the same block. Section
              description (above) intentionally short ("api keys and
              runtime overrides"); security/provenance details live in
              the dialog where the operator is actually entering the
              key, not as permanent header prose. */}
          <div className="font-mono text-[12px] text-ink py-3 border-b border-rule space-y-1">
            <p>
              <span className="text-accent">[stored]</span> set here ·{' '}
              <span className="text-ink">[env-var]</span> from environment
              variable (takes precedence) ·{' '}
              <span className="text-ink-ghost">[local]</span> no key needed
              ·{' '}
              <span className="text-ink">[overridden]</span> routes via
              custom endpoint or max-tokens cap
            </p>
            <p className="text-ink-faint">
              keyboard shortcuts: ↑↓ navigate rows · press 1–
              {ACTIVE_PROVIDERS.length} to open a provider directly
            </p>
          </div>
          {ACTIVE_PROVIDERS.map((id, idx) => (
            <ProviderRow
              key={id}
              id={id}
              envVar={ENV_VAR_BY_ID.get(id) ?? ''}
              index={idx}
            />
          ))}
        </Section>
      </div>
    </main>
  )
}

/* ---------------------------------------------------------------------- */
/*  Section + Row primitives                                              */
/* ---------------------------------------------------------------------- */

interface SectionProps {
  title: string
  description?: string
  children: React.ReactNode
}

/**
 * Settings-page section: a small heading + optional one-liner + a
 * vertically-stacked list of rows underneath, joined by a thin top rule.
 * The rule above the list visually anchors the heading to its content
 * without the heavier "h1 + border" treatment used for the page header.
 */
function Section({ title, description, children }: SectionProps) {
  return (
    <section className="mt-10">
      <h2 className="font-mono text-[12px] text-ink lowercase tracking-[0.06em] mb-1">
        {title}
      </h2>
      {description ? (
        <p className="font-mono text-[11px] text-ink-faint mb-3">
          {description}
        </p>
      ) : null}
      <div className="border-t border-rule">{children}</div>
    </section>
  )
}

interface RowProps {
  label: string
  control: React.ReactNode
  meta?: React.ReactNode
}

/**
 * Generic settings-page row. Mirrors the layout `ProviderRow` already
 * uses (label on the left, optional meta in the middle, control on the
 * right) so the appearance section and provider section read as one
 * consistent settings document instead of two different idioms.
 */
function Row({ label, control, meta }: RowProps) {
  return (
    <div className="flex items-center gap-4 py-3 border-b border-rule last:border-b-0">
      <span className="font-mono text-[13px] text-ink w-24 shrink-0 truncate">
        {label}
      </span>
      <span className="flex-1 min-w-0 font-mono text-[11px] text-ink-faint truncate">
        {meta}
      </span>
      <span className="shrink-0">{control}</span>
    </div>
  )
}
