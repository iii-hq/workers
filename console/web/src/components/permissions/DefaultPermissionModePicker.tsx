import { useState } from 'react'
import { ModeToggle } from '@/components/ui/ModeToggle'
import {
  loadDefaultPermissionMode,
  type PermissionMode,
  saveDefaultPermissionMode,
} from '@/lib/storage'
import { FullModeConfirmDialog } from './FullModeConfirmDialog'

interface DefaultPermissionModePickerProps {
  /** Lifted state so a parent can react (banner, telemetry) without re-reading localStorage. */
  value?: PermissionMode
  onChange?: (next: PermissionMode) => void
}

/**
 * User-level default mode applied only to NEW conversations. Existing
 * conversations own their own mode independently after creation.
 *
 * Selecting Full opens a confirmation dialog before localStorage is
 * written. Cancel keeps the previous value.
 */
export function DefaultPermissionModePicker({
  value,
  onChange,
}: DefaultPermissionModePickerProps) {
  const [internal, setInternal] = useState<PermissionMode>(() =>
    value ?? loadDefaultPermissionMode(),
  )
  const [pendingFull, setPendingFull] = useState(false)
  const current = value ?? internal

  function commit(next: PermissionMode) {
    saveDefaultPermissionMode(next)
    setInternal(next)
    onChange?.(next)
  }

  function handleSelect(next: PermissionMode) {
    if (next === current) return
    if (next === 'full') {
      setPendingFull(true)
      return
    }
    commit(next)
  }

  return (
    <>
      <ModeToggle<PermissionMode>
        value={current}
        onChange={handleSelect}
        variant="radio"
        aria-label="default permission mode"
        options={[
          { value: 'manual', label: 'manual' },
          { value: 'auto', label: 'auto' },
          { value: 'full', label: 'full' },
        ]}
      />
      <FullModeConfirmDialog
        open={pendingFull}
        onOpenChange={setPendingFull}
        onConfirm={() => commit('full')}
        scope="default"
      />
    </>
  )
}
