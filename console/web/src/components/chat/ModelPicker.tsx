import { Select } from '@/components/ui/Select'
import type { ModelId, ModelOption } from '@/types/chat'

interface ModelPickerProps {
  value: ModelId
  options: ModelOption[]
  onChange: (next: ModelId) => void
  disabled?: boolean
  loading?: boolean
  className?: string
}

export function ModelPicker({
  value,
  options,
  onChange,
  disabled,
  loading,
  className,
}: ModelPickerProps) {
  const pickerOptions =
    options.length > 0 ? options : [{ id: value, label: value }]
  const safeValue = pickerOptions.some((o) => o.id === value)
    ? value
    : pickerOptions[0].id

  return (
    <Select<ModelId>
      value={safeValue}
      options={pickerOptions.map((m) => ({ value: m.id, label: m.label }))}
      onChange={onChange}
      disabled={disabled || loading}
      aria-label={loading ? 'model (loading catalog)' : 'model'}
      aria-busy={loading || undefined}
      className={className}
    />
  )
}
