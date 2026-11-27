import { Settings2, ShieldCheck } from 'lucide-react'
import { useState } from 'react'
import { Select } from '@/components/ui/Select'
import { SheetOptionList } from '@/components/ui/SheetNavigation'
import type { PermissionMode } from '@/lib/backend/approval-settings'
import { GENERAL_ANCHOR } from '@/pages/Configuration/lib/general-settings-index'
import { openGeneralSettingsRow } from '@/pages/Configuration/lib/reveal-general-row'
import { FullModeConfirmDialog } from './FullModeConfirmDialog'

interface PermissionModePickerProps {
  /** Backend-owned mode for THIS conversation. */
  value: PermissionMode
  /** Sets mode for THIS conversation; selecting full goes through a confirm. */
  onChange: (next: PermissionMode) => void
  /** Disabled while the backend is still loading the initial value. */
  disabled?: boolean
}

/**
 * In-chat permission mode picker. Lives next to the composer as one compact
 * dropdown and commits each change directly to the backend through `onChange`.
 * Selecting Full gates on the shared confirmation dialog. Allowlist
 * management lives in global Settings, so keep this control narrow.
 */
export function PermissionModePicker({
  value,
  onChange,
  disabled,
}: PermissionModePickerProps) {
  const [pendingFull, setPendingFull] = useState(false)

  function handleSelect(next: PermissionMode) {
    if (disabled || next === value) return
    if (next === 'full') {
      setPendingFull(true)
      return
    }
    onChange(next)
  }

  return (
    <>
      {/* One pill like the model and agent beside it: icon, label, chevron,
          quiet until hovered. Auto carries a gear to the allowlist that
          decides what it runs. */}
      <span className="inline-flex min-w-0 shrink-0 items-center">
        <Select<PermissionMode>
          value={value}
          onChange={handleSelect}
          disabled={disabled}
          aria-label="approval mode"
          leading={<ShieldCheck className="size-4" />}
          appearance="pill"
          popupClassName="min-w-64"
          options={[
            {
              value: 'manual',
              label: 'Manual',
              description: 'Asks before each function.',
            },
            {
              value: 'auto',
              label: 'Auto',
              description: 'Allowed functions run, the rest ask.',
            },
            {
              value: 'full',
              label: 'Full',
              description: 'Every function runs, no prompts.',
            },
          ]}
        />
        {value === 'auto' ? (
          <button
            type="button"
            aria-label="Configure the Auto mode allowlist"
            title="Configure the Auto mode allowlist"
            disabled={disabled}
            onClick={() =>
              openGeneralSettingsRow([
                GENERAL_ANCHOR.allowlist,
                GENERAL_ANCHOR.permissions,
              ])
            }
            className="flex size-8 shrink-0 items-center justify-center rounded-lg text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-40"
          >
            <Settings2 aria-hidden className="size-4" />
          </button>
        ) : null}
      </span>
      <FullModeConfirmDialog
        open={pendingFull}
        onOpenChange={setPendingFull}
        onConfirm={() => onChange('full')}
        scope="conversation"
      />
    </>
  )
}

interface PermissionModePickerPanelProps {
  value: PermissionMode
  onChange: (next: PermissionMode) => void
  onRequestFull: () => void
  disabled?: boolean
  className?: string
}

/** Inline permission choices for an existing navigable sheet. */
export function PermissionModePickerPanel({
  value,
  onChange,
  onRequestFull,
  disabled,
  className,
}: PermissionModePickerPanelProps) {
  return (
    <SheetOptionList
      value={value}
      disabled={disabled}
      className={className}
      onChange={(next) => {
        if (next === 'full') onRequestFull()
        else onChange(next)
      }}
      options={[
        {
          value: 'manual',
          label: 'Manual',
          description: 'Asks before each function.',
        },
        {
          value: 'auto',
          label: 'Auto',
          description: 'Allowed functions run, the rest ask.',
        },
        {
          value: 'full',
          label: 'Full',
          description: 'Every function runs, no prompts.',
        },
      ]}
    />
  )
}
