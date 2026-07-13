import { ModeToggle } from '@/components/ui/ModeToggle'
import { ConsoleExtensionSlot } from '@/extensions/ConsoleExtensions'
import type { Theme } from '@/hooks/use-theme'

// Provider credentials + settings now live in the llm-router `configuration`
// entry, edited via the schema-driven form in the Workers modal.
const HARNESS_CONFIG_HASH = '#/workers/configuration/llm-router'

interface ConsoleSettingsTabProps {
  theme: Theme
  onThemeChange: (next: Theme) => void
}

/**
 * Console-level preferences: theme + provider API keys. Extracted from the
 * page shell so the Configuration page can host additional tabs (workers,
 * future surfaces) without nesting unrelated logic.
 *
 * Keyboard nav (number keys to open a provider, arrow keys to walk rows) is
 * scoped to this tab — the listener self-removes when the tab unmounts so
 * it never fights other surfaces.
 */
export function ConsoleSettingsTab({
  theme,
  onThemeChange,
}: ConsoleSettingsTabProps) {
  return (
    <div className="flex-1 min-h-0 overflow-y-auto">
      <div className="mx-auto max-w-3xl px-6 py-10">
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

        <ConsoleExtensionSlot name="settings.sections" context={{}} />

        <Section
          title="providers"
          description="api keys, endpoints, and per-provider settings."
        >
          <Row
            label="manage"
            control={
              <a
                href={HARNESS_CONFIG_HASH}
                className="font-sans text-[12px] px-3 py-1 border border-rule text-ink hover:border-ink transition-colors"
              >
                open provider settings
              </a>
            }
            meta="credentials + settings live in the harness configuration. the form's shape grows with each provider that registers; api keys are masked."
          />
        </Section>
      </div>
    </div>
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
    <section className="mt-10 first:mt-0">
      <h2 className="font-sans text-[14px] text-ink capitalize tracking-[0.06em] mb-1">
        {title}
      </h2>
      {description ? (
        <p className="font-sans text-[12px] text-ink-faint mb-3">
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
 * Generic settings-page row: label on the left, optional meta in the
 * middle, control on the right — so every section reads as one
 * consistent settings document instead of separate idioms.
 */
function Row({ label, control, meta }: RowProps) {
  return (
    <div className="flex items-center gap-4 py-3 border-b border-rule last:border-b-0">
      <span className="font-sans text-[13px] text-ink w-24 shrink-0 truncate">
        {label}
      </span>
      <span className="flex-1 min-w-0 font-sans text-[11px] text-ink-faint truncate">
        {meta}
      </span>
      <span className="shrink-0">{control}</span>
    </div>
  )
}
