import { Select } from '@/components/ui/Select'
import { MODELS, type ModelId } from '@/types/chat'

interface ModelPickerProps {
  value: ModelId
  onChange: (next: ModelId) => void
  disabled?: boolean
  className?: string
}

export function ModelPicker({
  value,
  onChange,
  disabled,
  className,
}: ModelPickerProps) {
  return (
    <Select<ModelId>
      value={value}
      options={MODELS.map((m) => ({ value: m.id, label: m.label }))}
      onChange={onChange}
      disabled={disabled}
      aria-label="model"
      className={className}
    />
  )
}
