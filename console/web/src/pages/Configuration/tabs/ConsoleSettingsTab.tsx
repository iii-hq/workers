import { useCallback, useEffect, useMemo, useState } from 'react'
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
import {
  autoAllowSeedFromRules,
  loadApprovalGateDefaults,
  saveApprovalGateDefaults,
} from '@/lib/backend/approval-gate-config'
import type { PermissionMode } from '@/lib/backend/approval-settings'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { filterAllowlistCandidates } from '@/lib/permissions/allowlist-filter'

// Provider credentials + settings now live in the llm-router `configuration`
// entry, edited via the schema-driven form on the workers tab.
const HARNESS_CONFIG_HASH = '#/configuration/workers/llm-router'
// Shell's permanent `fs.host_roots` list — do NOT build a second editor for
// it here; deep-link to the existing schema-driven form instead.
const SHELL_CONFIG_HASH = '#/configuration/workers/shell'

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
  // The permissions section only applies when the optional approval-gate worker
  // is connected (it owns `approval::*` + the config entry). Absent → hide the
  // whole section and skip the config read so it can't error. Outside the
  // provider (Storybook) the context is null; treat that as available.
  const ctx = useConversationsCtxOptional()
  const approvalGateAvailable = ctx ? ctx.approvalGateAvailable : true

  // Deployment defaults from the approval-gate configuration entry (single source).
  const [defaultMode, setDefaultMode] = useState<PermissionMode>('manual')
  const [allowlist, setAllowlist] = useState<string[]>([])
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    if (!approvalGateAvailable) return
    let cancelled = false
    void (async () => {
      try {
        const cfg = await loadApprovalGateDefaults()
        if (cancelled) return
        setDefaultMode(cfg.defaultMode)
        setAllowlist(cfg.allowlist)
      } catch (err) {
        console.error(
          '[console-settings] failed to load approval-gate config',
          err,
        )
      } finally {
        if (!cancelled) setLoaded(true)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [approvalGateAvailable])

  const persistDefaults = useCallback(
    async (mode: PermissionMode, list: string[]) => {
      try {
        const next = await saveApprovalGateDefaults(mode, list)
        setDefaultMode(next.default_mode)
        setAllowlist(autoAllowSeedFromRules(next.rules))
      } catch (err) {
        console.error(
          '[console-settings] failed to save approval-gate config',
          err,
        )
      }
    },
    [],
  )

  const handleModeChange = useCallback(
    (next: PermissionMode) => {
      setDefaultMode(next)
      void persistDefaults(next, allowlist)
    },
    [allowlist, persistDefaults],
  )

  const addAllow = useCallback(
    (functionId: string) => {
      setAllowlist((prev) => {
        if (prev.includes(functionId)) return prev
        const next = [...prev, functionId]
        void persistDefaults(defaultMode, next)
        return next
      })
    },
    [defaultMode, persistDefaults],
  )

  const removeAllow = useCallback(
    (functionId: string) => {
      setAllowlist((prev) => {
        if (!prev.includes(functionId)) return prev
        const next = prev.filter((id) => id !== functionId)
        void persistDefaults(defaultMode, next)
        return next
      })
    },
    [defaultMode, persistDefaults],
  )

  const allowlistSet = useMemo(() => new Set(allowlist), [allowlist])

  const { functionEntries } = useFunctionsCatalog(getDefaultBackend().id)
  const allowlistCandidates = useMemo(
    () => filterAllowlistCandidates(functionEntries),
    [functionEntries],
  )
  const [allowlistOpen, setAllowlistOpen] = useState(false)

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

        {approvalGateAvailable ? (
          <Section
            title="permissions"
            description="default mode and auto allowlist stored in the approval-gate configuration entry. applies to NEW conversations only."
          >
            <Row
              label="default mode"
              control={
                <DefaultPermissionModePicker
                  value={loaded ? defaultMode : undefined}
                  onChange={handleModeChange}
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
        ) : null}

        <Dialog open={allowlistOpen} onOpenChange={setAllowlistOpen}>
          <DialogContent className="max-w-xl max-h-[80vh] overflow-hidden flex flex-col">
            <DialogTitle className="text-[14px]">
              auto-mode allowlist
            </DialogTitle>
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

        <Section
          title="filesystem access"
          description="the agent can always use a conversation's chosen workspace. touching anything outside it prompts in the chat — allow once, for the session, or permanently."
        >
          <Row
            label="filesystem access"
            control={
              <a
                href={SHELL_CONFIG_HASH}
                className="font-mono text-[12px] px-3 py-1 border border-rule text-ink hover:border-ink transition-colors"
              >
                edit permanent roots
              </a>
            }
            meta="workspace + per-session grants are managed from the chat's filesystem access dialog. permanent roots (allowed for every conversation) live in shell configuration."
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
