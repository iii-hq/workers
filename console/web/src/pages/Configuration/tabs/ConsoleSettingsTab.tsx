import { useCallback, useMemo, useState } from 'react'
import { DefaultPermissionModePicker } from '@/components/permissions/DefaultPermissionModePicker'
import { FunctionAllowlistTree } from '@/components/permissions/FunctionAllowlistTree'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { ModeToggle } from '@/components/ui/ModeToggle'
import { useFunctionsCatalog } from '@/hooks/use-functions-catalog'
import type { Theme } from '@/hooks/use-theme'
import { getDefaultBackend } from '@/lib/backend'
import type { PermissionMode } from '@/lib/backend/approval-settings'
import { filterAllowlistCandidates } from '@/lib/permissions/allowlist-filter'
import {
  loadDefaultAllowlist,
  loadDefaultPermissionMode,
  saveDefaultAllowlist,
} from '@/lib/storage'

// Provider credentials + settings now live in the llm-router `configuration`
// entry, edited via the schema-driven form on the workers tab.
const HARNESS_CONFIG_HASH = '#/configuration/workers/llm-router'

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
  // Controlled default-permission-mode + per-user allowlist. Both back to
  // localStorage. The allowlist section only renders while mode === 'auto'
  // (it has no effect under manual/full, so showing it would mislead).
  const [defaultMode, setDefaultMode] = useState<PermissionMode>(() =>
    loadDefaultPermissionMode(),
  )
  const [allowlist, setAllowlist] = useState<string[]>(() =>
    loadDefaultAllowlist(),
  )
  const allowlistSet = useMemo(() => new Set(allowlist), [allowlist])

  const addAllow = useCallback((functionId: string) => {
    setAllowlist((prev) => {
      if (prev.includes(functionId)) return prev
      const next = [...prev, functionId]
      saveDefaultAllowlist(next)
      return next
    })
  }, [])

  const removeAllow = useCallback((functionId: string) => {
    setAllowlist((prev) => {
      if (!prev.includes(functionId)) return prev
      const next = prev.filter((id) => id !== functionId)
      saveDefaultAllowlist(next)
      return next
    })
  }, [])

  const { functionEntries } = useFunctionsCatalog(getDefaultBackend().id)
  const allowlistCandidates = useMemo(
    () => filterAllowlistCandidates(functionEntries),
    [functionEntries],
  )
  const [allowlistOpen, setAllowlistOpen] = useState(false)

  return (
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

      <Section
        title="permissions"
        description="default mode applied to NEW conversations only. existing ones keep their own mode."
      >
        <Row
          label="default mode"
          control={
            <DefaultPermissionModePicker
              value={defaultMode}
              onChange={setDefaultMode}
            />
          }
          meta="manual prompts for everything · auto skips functions on your allowlist · full skips everything"
        />
        {defaultMode === 'auto' ? (
          <Row
            label="allowlist"
            control={
              <button
                type="button"
                onClick={() => setAllowlistOpen(true)}
                className="font-mono text-[12px] px-3 py-1 border border-rule text-ink hover:border-ink transition-colors"
              >
                manage
                {allowlist.length > 0 ? ` (${allowlist.length})` : ''}
              </button>
            }
            meta="functions that auto-approve while a new conversation is in auto mode. edits apply to NEW conversations only."
          />
        ) : null}
      </Section>

      <Dialog open={allowlistOpen} onOpenChange={setAllowlistOpen}>
        <DialogContent className="max-w-xl max-h-[80vh] overflow-hidden flex flex-col">
          <DialogTitle className="text-[14px]">auto-mode allowlist</DialogTitle>
          <DialogDescription className="mt-1">
            checked functions auto-approve while a new conversation is in auto
            mode. existing conversations keep their own snapshot.
          </DialogDescription>
          <div className="mt-4 flex-1 overflow-y-auto border border-rule-2 -mx-2 px-2 py-2 min-h-[280px]">
            <FunctionAllowlistTree
              functions={allowlistCandidates}
              allowlist={allowlistSet}
              onAdd={addAllow}
              onRemove={removeAllow}
              emptyHint="catalog hasn't loaded any functions yet."
            />
          </div>
          <div className="mt-4 flex justify-end">
            <button
              type="button"
              onClick={() => setAllowlistOpen(false)}
              className="font-mono text-[12px] px-3 py-1 border border-ink bg-ink text-bg hover:bg-bg hover:text-ink transition-colors"
            >
              done
            </button>
          </div>
        </DialogContent>
      </Dialog>

      <Section
        title="providers"
        description="api keys, endpoints, and per-provider settings."
      >
        <Row
          label="manage"
          control={
            <a
              href={HARNESS_CONFIG_HASH}
              className="font-mono text-[12px] px-3 py-1 border border-rule text-ink hover:border-ink transition-colors"
            >
              open provider settings
            </a>
          }
          meta="credentials + settings live in the harness configuration (workers tab). the form's shape grows with each provider that registers; api keys are masked."
        />
      </Section>
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
 * Generic settings-page row: label on the left, optional meta in the
 * middle, control on the right — so every section reads as one
 * consistent settings document instead of separate idioms.
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
