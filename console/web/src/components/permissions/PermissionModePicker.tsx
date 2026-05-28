import { useState } from 'react'
import { ModeToggle } from '@/components/ui/ModeToggle'
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
 * In-chat permission mode picker. Lives next to the composer; commits
 * each change directly to the backend through `onChange`. Selecting Full
 * gates on the shared confirmation dialog. Allowlist management lives
 * on the Configuration screen — keep this control narrow.
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
      <ModeToggle<PermissionMode>
        value={value}
        onChange={handleSelect}
        variant="radio"
        aria-label="permission mode"
        options={[
          { value: 'manual', label: 'manual' },
          { value: 'auto', label: 'auto' },
          { value: 'full', label: 'full' },
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
