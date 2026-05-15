import { ModeToggle } from '@/components/ui/ModeToggle'
import { MODES, type Mode } from '@/types/chat'

interface ModePickerProps {
  value: Mode
  onChange: (next: Mode) => void
  className?: string
}

export function ModePicker({ value, onChange, className }: ModePickerProps) {
  return (
    <ModeToggle<Mode>
      value={value}
      onChange={onChange}
      options={MODES.map((m) => ({ value: m.id, label: m.label }))}
      className={className}
    />
  )
}
