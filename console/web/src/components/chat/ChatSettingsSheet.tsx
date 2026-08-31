import { Bot, Brain, Folder, ShieldCheck, Sparkles } from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { FullModeConfirmContent } from '@/components/permissions/FullModeConfirmDialog'
import { PermissionModePickerPanel } from '@/components/permissions/PermissionModePicker'
import { BottomSheet, BottomSheetContent } from '@/components/ui/BottomSheet'
import {
  SheetMenuGroup,
  SheetMenuRow,
  SheetPage,
  useSheetNavigation,
} from '@/components/ui/SheetNavigation'
import type { PermissionMode } from '@/lib/backend/approval-settings'
import { useUnsavedGuard } from '@/pages/Configuration/tabs/WorkersTab/useUnsavedGuard'
import type { Mode, ModelId, ModelOption, ThinkingLevel } from '@/types/chat'
import { BankPickerPanel } from './BankPicker'
import { DirectoryPicker, type WorktreePickerOptions } from './DirectoryPicker'
import { ModelPickerPanel, ReasoningEffortPanel } from './ModelPicker'
import { ModePickerPanel } from './ModePicker'
import {
  formatProviderLabel,
  getModelPresentation,
} from './model-picker-presentation'
import { ProviderConfigurationPanel } from './ProviderConfigurationPanel'

type ChatSettingsPage =
  | 'settings'
  | 'model'
  | 'reasoning'
  | 'provider-config'
  | 'mode'
  | 'permissions'
  | 'full-permissions'
  | 'memory'
  | 'directory'

interface ChatSettingsSheetProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  mode: Mode
  model: ModelId | null
  modelOptions: ModelOption[]
  catalogLoading?: boolean
  permissionMode: PermissionMode
  permissionModeLoading?: boolean
  showPermissionMode?: boolean
  thinkingLevel: ThinkingLevel
  showWorkingDir?: boolean
  workingDir?: string | null
  showMemoryBank?: boolean
  memoryBank?: string | null
  workingDirLocked?: boolean
  workingDirError?: string | null
  defaultWorkingDir?: string | null
  worktreePicker?: WorktreePickerOptions
  disabled?: boolean
  /** Disable only model/reasoning while leaving other chat settings usable. */
  modelDisabled?: boolean
  onModeChange: (next: Mode) => void
  onModelChange: (next: ModelId) => void
  onMemoryBankChange?: (next: string | null) => void
  onWorkingDirChange?: (next: string) => void
  onThinkingLevelChange: (next: ThinkingLevel) => void
  onPermissionModeChange: (next: PermissionMode) => void
}

interface SettingsSectionProps {
  id: string
  label: string
  children: ReactNode
  className?: string
}

function SettingsSection({
  id,
  label,
  children,
  className,
}: SettingsSectionProps) {
  return (
    <section aria-labelledby={id} className={className}>
      <h3
        id={id}
        className="mb-2 px-1 font-sans text-[11px] font-medium text-ink-ghost"
      >
        {label}
      </h3>
      <SheetMenuGroup>{children}</SheetMenuGroup>
    </section>
  )
}

function directoryName(path: string | null | undefined): string {
  if (!path) return 'Choose directory'
  return path.split('/').filter(Boolean).at(-1) ?? path
}

/**
 * Mobile chat settings as one dialog with an in-place navigation stack.
 * Every drill-in selector replaces the current page instead of opening a
 * competing portal, so pointer events and focus stay owned by one sheet.
 */
export function ChatSettingsSheet({
  open,
  onOpenChange,
  mode,
  model,
  modelOptions,
  catalogLoading,
  permissionMode,
  permissionModeLoading,
  showPermissionMode = true,
  thinkingLevel,
  showWorkingDir,
  workingDir,
  showMemoryBank,
  memoryBank,
  workingDirLocked,
  workingDirError,
  defaultWorkingDir,
  worktreePicker,
  disabled,
  modelDisabled,
  onModeChange,
  onModelChange,
  onMemoryBankChange,
  onWorkingDirChange,
  onThinkingLevelChange,
  onPermissionModeChange,
}: ChatSettingsSheetProps) {
  const navigation = useSheetNavigation<ChatSettingsPage>('settings')
  const [configurationProvider, setConfigurationProvider] = useState<
    string | null
  >(null)
  const configurationGuard = useUnsavedGuard()
  const modelPresentation = getModelPresentation(model, modelOptions)
  const selectedModel = modelOptions.find((option) => option.id === model)

  function handleOpenChange(next: boolean) {
    if (next) {
      onOpenChange(true)
      return
    }
    configurationGuard.tryNavigate(() => {
      navigation.reset()
      setConfigurationProvider(null)
      onOpenChange(false)
    })
  }

  return (
    <BottomSheet open={open} onOpenChange={handleOpenChange}>
      <BottomSheetContent className="mx-auto max-w-[460px]">
        {navigation.page === 'settings' ? (
          <SheetPage
            title="Chat settings"
            description="Applies to your next message."
            contentClassName="space-y-5 px-4 pb-1"
          >
            <SettingsSection id="chat-settings-model" label="Model">
              <SheetMenuRow
                label={modelPresentation.label}
                value={
                  modelPresentation.provider ? (
                    <span className="rounded-full bg-surface-active px-2 py-0.5 text-[11px] font-medium">
                      {modelPresentation.provider}
                    </span>
                  ) : undefined
                }
                icon={<Sparkles className="size-[18px]" aria-hidden />}
                disabled={disabled || modelDisabled || catalogLoading}
                onClick={() => navigation.push('model')}
              />
            </SettingsSection>

            <SettingsSection id="chat-settings-behavior" label="Behavior">
              <SheetMenuRow
                label="Mode"
                value={<span className="capitalize">{mode}</span>}
                icon={<Bot className="size-[18px]" aria-hidden />}
                disabled={disabled}
                onClick={() => navigation.push('mode')}
              />
              {showPermissionMode ? (
                <SheetMenuRow
                  label="Permissions"
                  value={<span className="capitalize">{permissionMode}</span>}
                  icon={<ShieldCheck className="size-[18px]" aria-hidden />}
                  disabled={disabled || permissionModeLoading}
                  onClick={() => navigation.push('permissions')}
                />
              ) : null}
              {showMemoryBank && onMemoryBankChange ? (
                <SheetMenuRow
                  label="Memory"
                  value={memoryBank ?? 'Auto'}
                  icon={<Brain className="size-[18px]" aria-hidden />}
                  disabled={disabled}
                  onClick={() => navigation.push('memory')}
                />
              ) : null}
            </SettingsSection>

            {showWorkingDir && onWorkingDirChange ? (
              <SettingsSection
                id="chat-settings-environment"
                label="Environment"
              >
                <SheetMenuRow
                  label="Working directory"
                  value={directoryName(workingDir)}
                  icon={<Folder className="size-[18px]" aria-hidden />}
                  disabled={disabled || workingDirLocked}
                  onClick={() => navigation.push('directory')}
                />
              </SettingsSection>
            ) : null}
          </SheetPage>
        ) : null}

        {navigation.page === 'model' ? (
          <SheetPage
            title="Model & reasoning"
            description="Choose the model and its reasoning effort."
            onBack={navigation.back}
            backLabel="Back to chat settings"
            contentClassName="flex overflow-hidden"
          >
            <ModelPickerPanel
              value={model}
              options={modelOptions}
              thinkingLevel={thinkingLevel}
              onChange={onModelChange}
              onThinkingLevelChange={onThinkingLevelChange}
              onConfigureProvider={(providerId) => {
                setConfigurationProvider(providerId)
                navigation.push('provider-config')
              }}
              onOpenReasoning={() => navigation.push('reasoning')}
              disabled={disabled || modelDisabled}
              loading={catalogLoading}
            />
          </SheetPage>
        ) : null}

        {navigation.page === 'reasoning' ? (
          <SheetPage
            title="Reasoning effort"
            description="Choose how much reasoning this model should use."
            onBack={navigation.back}
            backLabel="Back to models"
            contentClassName="px-4 pb-4"
          >
            <ReasoningEffortPanel
              model={selectedModel}
              value={thinkingLevel}
              onChange={(next) => {
                onThinkingLevelChange(next)
                navigation.back()
              }}
              disabled={disabled || modelDisabled}
            />
          </SheetPage>
        ) : null}

        {navigation.page === 'provider-config' && configurationProvider ? (
          <SheetPage
            title={
              formatProviderLabel(configurationProvider) ??
              configurationProvider
            }
            description="Credentials and provider-specific settings."
            onBack={() =>
              configurationGuard.tryNavigate(() => {
                navigation.back()
                setConfigurationProvider(null)
              })
            }
            backLabel="Back to models"
            contentClassName="overflow-hidden"
          >
            <ProviderConfigurationPanel
              providerId={configurationProvider}
              onDirtyChange={configurationGuard.setDirty}
            />
          </SheetPage>
        ) : null}

        {navigation.page === 'mode' ? (
          <SheetPage
            title="Mode"
            description="Choose how the assistant handles your message."
            onBack={navigation.back}
            backLabel="Back to chat settings"
            contentClassName="px-4 pb-4"
          >
            <ModePickerPanel
              value={mode}
              disabled={disabled}
              onChange={(next) => {
                onModeChange(next)
                navigation.back()
              }}
            />
          </SheetPage>
        ) : null}

        {navigation.page === 'permissions' ? (
          <SheetPage
            title="Permissions"
            description="Control when functions require your approval."
            onBack={navigation.back}
            backLabel="Back to chat settings"
            contentClassName="px-4 pb-4"
          >
            <PermissionModePickerPanel
              value={permissionMode}
              disabled={disabled || permissionModeLoading}
              onRequestFull={() => navigation.push('full-permissions')}
              onChange={(next) => {
                onPermissionModeChange(next)
                navigation.back()
              }}
            />
          </SheetPage>
        ) : null}

        {navigation.page === 'full-permissions' ? (
          <SheetPage
            title="Enable full permissions?"
            onBack={navigation.back}
            backLabel="Back to permissions"
            contentClassName="px-4 pb-4"
          >
            <FullModeConfirmContent
              onCancel={navigation.back}
              onConfirm={() => {
                onPermissionModeChange('full')
                navigation.reset()
              }}
            />
          </SheetPage>
        ) : null}

        {navigation.page === 'memory' && onMemoryBankChange ? (
          <SheetPage
            title="Memory"
            description="Choose which memory bank feeds the next turn."
            onBack={navigation.back}
            backLabel="Back to chat settings"
            contentClassName="px-4 pb-4"
          >
            <BankPickerPanel
              value={memoryBank ?? null}
              disabled={disabled}
              onChange={(next) => {
                onMemoryBankChange(next)
                navigation.back()
              }}
            />
          </SheetPage>
        ) : null}

        {navigation.page === 'directory' && onWorkingDirChange ? (
          <SheetPage
            title="Working directory"
            description={
              worktreePicker?.enabled
                ? 'Choose a recent project, folder, or managed worktree.'
                : 'Choose a recent project or browse folders.'
            }
            onBack={navigation.back}
            backLabel="Back to chat settings"
            contentClassName="overflow-hidden"
          >
            <DirectoryPicker
              value={workingDir ?? null}
              onChange={onWorkingDirChange}
              onSelect={navigation.back}
              presentation="embedded"
              locked={workingDirLocked}
              disabled={disabled || workingDirLocked}
              externalError={workingDirError}
              defaultDir={defaultWorkingDir}
              worktrees={worktreePicker}
            />
          </SheetPage>
        ) : null}
      </BottomSheetContent>
    </BottomSheet>
  )
}
