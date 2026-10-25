import { ArrowUpRight } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { FullModeConfirmDialog } from '@/components/permissions/FullModeConfirmDialog'
import { FunctionAllowlistTree } from '@/components/permissions/FunctionAllowlistTree'
import { Button } from '@/components/shadcn/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/shadcn/dialog'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/shadcn/select'
import { Switch } from '@/components/shadcn/switch'
import { useFunctionsCatalog } from '@/hooks/use-functions-catalog'
import { hashForWorkersConfiguration } from '@/hooks/use-hash-route'
import { useTabLimit } from '@/hooks/use-tab-limit'
import type { ThemePreference } from '@/hooks/use-theme'
import { getDefaultBackend } from '@/lib/backend'
import {
  autoAllowSeedFromRules,
  loadApprovalGateDefaults,
  saveApprovalGateDefaults,
} from '@/lib/backend/approval-gate-config'
import type { PermissionMode } from '@/lib/backend/approval-settings'
import {
  loadCompletionBellEnabled,
  playCompletionBell,
  saveCompletionBellEnabled,
  subscribeCompletionBellPreference,
} from '@/lib/completion-bell'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { filterAllowlistCandidates } from '@/lib/permissions/allowlist-filter'
import { settingsFade } from '../components/motion'
import {
  SettingsList,
  SettingsPageBody,
  SettingsRow,
  SettingsSection,
} from '../components/primitives'

// Provider credentials + settings live in the llm-router configuration entry.
const HARNESS_CONFIG_HASH = hashForWorkersConfiguration('llm-router')
// Shell's permanent `fs.host_roots` list — do NOT build a second editor for
// it here; deep-link to the worker-owned configuration interface instead.
const SHELL_CONFIG_HASH = hashForWorkersConfiguration('ide')
// The workspace tab limit lives in the ADE's own configuration entry
// (`tabs.limit`), edited through its schema-driven form like the port.
const TAB_LIMIT_CONFIG_HASH = hashForWorkersConfiguration('console', [
  'tabs',
  'limit',
])

const THEME_OPTIONS: { value: ThemePreference; label: string }[] = [
  { value: 'light', label: 'Light' },
  { value: 'system', label: 'System' },
  { value: 'dark', label: 'Dark' },
]

const PERMISSION_MODE_OPTIONS: { value: PermissionMode; label: string }[] = [
  { value: 'manual', label: 'Manual' },
  { value: 'auto', label: 'Auto' },
  { value: 'full', label: 'Full' },
]

/* A row's dropdown: the shadcn trigger at Linear's width, white on the
   card so it reads as a control and not a chip. */
const ROW_SELECT_TRIGGER =
  'h-9 min-w-40 border-neutral-200 bg-white text-[15px] hover:bg-neutral-50 dark:border-neutral-800 dark:bg-neutral-900/60 dark:hover:bg-neutral-900'

interface ConsoleSettingsTabProps {
  theme: ThemePreference
  onThemeChange: (next: ThemePreference) => void
}

/**
 * Console-level preferences: theme, sounds, permission defaults, workspace
 * limits and links into the worker-owned provider and filesystem forms.
 * Extracted from the modal shell so global Settings can host worker sections
 * and future surfaces without nesting unrelated logic.
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
  const tabLimit = useTabLimit()

  // Deployment defaults from the approval-gate configuration entry (single source).
  const [defaultMode, setDefaultMode] = useState<PermissionMode>('manual')
  const [allowlist, setAllowlist] = useState<string[]>([])
  const [loaded, setLoaded] = useState(false)
  const [pendingFull, setPendingFull] = useState(false)
  const [completionBellEnabled, setCompletionBellEnabled] = useState(
    loadCompletionBellEnabled,
  )

  useEffect(
    () =>
      subscribeCompletionBellPreference(() => {
        setCompletionBellEnabled(loadCompletionBellEnabled())
      }),
    [],
  )

  const handleCompletionBellChange = useCallback((enabled: boolean) => {
    setCompletionBellEnabled(enabled)
    saveCompletionBellEnabled(enabled)
  }, [])

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

  const commitMode = useCallback(
    (next: PermissionMode) => {
      setDefaultMode(next)
      void persistDefaults(next, allowlist)
    },
    [allowlist, persistDefaults],
  )

  // Full mode skips every prompt, so it asks once before it becomes the
  // default for new conversations.
  const handleModeChange = useCallback(
    (next: PermissionMode) => {
      if (next === defaultMode) return
      if (next === 'full') {
        setPendingFull(true)
        return
      }
      commitMode(next)
    },
    [commitMode, defaultMode],
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
    <SettingsPageBody>
      <SettingsSection
        title="Appearance"
        description="Choose how the ADE looks in this browser."
      >
        <SettingsList>
          <SettingsRow
            label="Interface theme"
            description="Applied immediately and stored for this browser. System follows the OS as it changes."
            control={
              <Select
                value={theme}
                onValueChange={(value) =>
                  onThemeChange(value as ThemePreference)
                }
                items={THEME_OPTIONS}
              >
                <SelectTrigger
                  aria-label="Interface theme"
                  className={ROW_SELECT_TRIGGER}
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent align="end">
                  {THEME_OPTIONS.map((option) => (
                    <SelectItem key={option.value} value={option.value}>
                      {option.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            }
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title="Notifications"
        description="Local sounds from this open Console. No notification permission is required."
      >
        <SettingsList>
          <SettingsRow
            label="Completion bell"
            description="Play a gentle bell when a top-level conversation finishes. Failures use a distinct tone; stopped turns stay silent."
            layout="inline"
            control={
              <Switch
                aria-label="Completion bell"
                checked={completionBellEnabled}
                onCheckedChange={handleCompletionBellChange}
              />
            }
          />
          <SettingsRow
            label="Sound check"
            description="Play the success tone now. Your first tap also enables audio for later completions."
            action={
              <Button
                type="button"
                variant="outline"
                onClick={() => void playCompletionBell('completed', true)}
              >
                Test bell
              </Button>
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
                <Select
                  value={loaded ? defaultMode : 'manual'}
                  onValueChange={(value) =>
                    handleModeChange(value as PermissionMode)
                  }
                  items={PERMISSION_MODE_OPTIONS}
                >
                  <SelectTrigger
                    aria-label="Default permission mode"
                    className={ROW_SELECT_TRIGGER}
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent align="end">
                    {PERMISSION_MODE_OPTIONS.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        {option.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              }
            />
            {defaultMode === 'auto' ? (
              <SettingsRow
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                transition={settingsFade}
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
                    variant="outline"
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

      <FullModeConfirmDialog
        open={pendingFull}
        onOpenChange={setPendingFull}
        onConfirm={() => commitMode('full')}
        scope="default"
      />

      <Dialog open={allowlistOpen} onOpenChange={setAllowlistOpen}>
        <DialogContent className="flex max-h-[80vh] flex-col overflow-hidden [--dialog-max-width:36rem]">
          <DialogHeader>
            <DialogTitle>Auto-mode allowlist</DialogTitle>
            <DialogDescription>
              Selected functions run automatically in new conversations using
              Auto mode. Existing conversations keep their current selection.
            </DialogDescription>
          </DialogHeader>
          <div className="min-h-[280px] flex-1 overflow-y-auto overscroll-contain rounded-lg border border-neutral-200 p-2 dark:border-neutral-800">
            <FunctionAllowlistTree
              functions={allowlistCandidates}
              allowlist={allowlistSet}
              onAdd={addAllow}
              onRemove={removeAllow}
              emptyHint="The function catalog has not loaded yet."
            />
          </div>
          <DialogFooter>
            <Button type="button" onClick={() => setAllowlistOpen(false)}>
              Done
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <SettingsSection
        title="Workspace"
        description="How many workspace tabs this ADE lets you open at once."
      >
        <SettingsList>
          <SettingsRow
            label="Tab limit"
            description="New-tab controls disable at the limit; tabs already open stay open."
            meta={`${tabLimit} ${tabLimit === 1 ? 'tab' : 'tabs'}`}
            action={
              <Button
                variant="outline"
                nativeButton={false}
                render={<a href={TAB_LIMIT_CONFIG_HASH} />}
              >
                Edit limit
              </Button>
            }
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title="Providers"
        description="Manage model credentials, endpoints, and provider-specific settings."
      >
        <SettingsList>
          <SettingsRow
            label="Model providers"
            description="API keys are masked and remain in the llm-router configuration."
            action={
              <Button
                variant="outline"
                nativeButton={false}
                render={<a href={HARNESS_CONFIG_HASH} />}
              >
                Open LLM Router
                <ArrowUpRight data-icon="inline-end" />
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
              <Button
                variant="outline"
                nativeButton={false}
                render={<a href={SHELL_CONFIG_HASH} />}
              >
                Open IDE
                <ArrowUpRight data-icon="inline-end" />
              </Button>
            }
          />
        </SettingsList>
      </SettingsSection>
    </SettingsPageBody>
  )
}
