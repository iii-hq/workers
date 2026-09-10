import { useCallback, useEffect, useMemo, useState } from 'react'
import { DefaultPermissionModePicker } from '@/components/permissions/DefaultPermissionModePicker'
import { FunctionAllowlistTree } from '@/components/permissions/FunctionAllowlistTree'
import { Button } from '@/components/ui/Button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { ModeToggle } from '@/components/ui/ModeToggle'
import {
  SettingsList,
  SettingsRow,
  SettingsSection,
} from '@/components/ui/Settings'
import { useFunctionsCatalog } from '@/hooks/use-functions-catalog'
import { hashForWorkersConfiguration } from '@/hooks/use-hash-route'
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

// Provider credentials + settings live in the llm-router configuration entry.
const HARNESS_CONFIG_HASH = hashForWorkersConfiguration('llm-router')
// Shell's permanent `fs.host_roots` list — do NOT build a second editor for
// it here; deep-link to the worker-owned configuration interface instead.
const SHELL_CONFIG_HASH = hashForWorkersConfiguration('shell')

interface ConsoleSettingsTabProps {
  theme: Theme
  onThemeChange: (next: Theme) => void
}

/**
 * Console-level preferences: theme + provider API keys. Extracted from the
 * modal shell so global Settings can host worker sections and future surfaces
 * without nesting unrelated logic.
 *
 * Keyboard nav (number keys to open a provider, arrow keys to walk rows) is
 * scoped to this section — the listener self-removes when it unmounts so
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
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto flex max-w-3xl flex-col gap-8 px-4 py-6 sm:px-6 sm:py-8">
        <SettingsSection
          title="Appearance"
          description="Choose how the console looks in this browser."
        >
          <SettingsList>
            <SettingsRow
              label="Theme"
              description="Applied immediately and stored for this browser."
              layout="inline"
              control={
                <ModeToggle<Theme>
                  value={theme}
                  onChange={onThemeChange}
                  variant="radio"
                  aria-label="Theme"
                  options={[
                    { value: 'light', label: 'Light' },
                    { value: 'dark', label: 'Dark' },
                  ]}
                />
              }
            />
          </SettingsList>
        </SettingsSection>

        {approvalGateAvailable ? (
          <SettingsSection
            title="Permissions"
            description="Set the default behavior for new conversations. Existing conversations keep their current permissions."
          >
            <SettingsList>
              <SettingsRow
                label="Default mode"
                description="Manual asks before every function. Auto skips functions on the allowlist. Full skips all prompts."
                control={
                  <DefaultPermissionModePicker
                    value={loaded ? defaultMode : undefined}
                    onChange={handleModeChange}
                  />
                }
              />
              {defaultMode === 'auto' ? (
                <SettingsRow
                  label="Auto-mode allowlist"
                  description="Functions that new conversations can run without asking."
                  meta={
                    allowlist.length === 0
                      ? 'No functions selected'
                      : `${allowlist.length} function${allowlist.length === 1 ? '' : 's'} selected`
                  }
                  action={
                    <Button
                      type="button"
                      variant="pill"
                      size="sm"
                      onClick={() => setAllowlistOpen(true)}
                    >
                      Manage
                    </Button>
                  }
                />
              ) : null}
            </SettingsList>
          </SettingsSection>
        ) : null}

        <Dialog open={allowlistOpen} onOpenChange={setAllowlistOpen}>
          <DialogContent className="flex max-h-[80vh] max-w-xl flex-col overflow-hidden">
            <DialogTitle>Auto-mode allowlist</DialogTitle>
            <DialogDescription className="mt-1">
              Selected functions run automatically in new conversations using
              Auto mode. Existing conversations keep their current selection.
            </DialogDescription>
            <div className="-mx-2 mt-4 min-h-[280px] flex-1 overflow-y-auto border border-rule-2 px-2 py-2">
              <FunctionAllowlistTree
                functions={allowlistCandidates}
                allowlist={allowlistSet}
                onAdd={addAllow}
                onRemove={removeAllow}
                emptyHint="The function catalog has not loaded yet."
              />
            </div>
            <div className="mt-4 flex justify-end">
              <Button type="button" onClick={() => setAllowlistOpen(false)}>
                Done
              </Button>
            </div>
          </DialogContent>
        </Dialog>

        <SettingsSection
          title="Providers"
          description="Manage model credentials, endpoints, and provider-specific settings."
        >
          <SettingsList>
            <SettingsRow
              label="Model providers"
              description="API keys are masked and remain in the llm-router configuration."
              action={
                <Button asChild variant="pill" size="sm">
                  <a href={HARNESS_CONFIG_HASH}>Open settings</a>
                </Button>
              }
            />
          </SettingsList>
        </SettingsSection>

        <SettingsSection
          title="Filesystem access"
          description="A conversation can always use its workspace. Access outside it requires a temporary or permanent grant."
        >
          <SettingsList>
            <SettingsRow
              label="Permanent roots"
              description="Folders available to every conversation. Workspace and session grants are managed from each chat."
              action={
                <Button asChild variant="pill" size="sm">
                  <a href={SHELL_CONFIG_HASH}>Edit roots</a>
                </Button>
              }
            />
          </SettingsList>
        </SettingsSection>
      </div>
    </div>
  )
}
