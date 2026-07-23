import { useState } from 'react'
import { Select } from '@/components/ui/Select'
import type { PermissionMode } from '@/lib/backend/approval-settings'
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
 * management lives on the Configuration screen, keep this control narrow.
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
      <Select<PermissionMode>
        value={value}
        onChange={handleSelect}
        disabled={disabled}
        aria-label="approval mode"
        className="min-w-[92px]"
        options={[
          {
            value: 'manual',
            label: 'manual',
            title:
              'manual: pause every function trigger until you approve or deny it',
          },
          {
            value: 'auto',
            label: 'auto',
            title:
              'auto: use configured safe functions automatically; ask for the rest',
          },
          {
            value: 'full',
            label: 'full',
            title: 'full: run every function without asking',
          },
        ]}
      />
      <FullModeConfirmDialog
        open={pendingFull}
        onOpenChange={setPendingFull}
        onConfirm={() => onChange('full')}
        scope="conversation"
      />
    </>
  )
}
