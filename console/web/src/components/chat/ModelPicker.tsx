import { Select, type SelectGroup } from '@/components/ui/Select'
import {
  CATALOG_MODEL_KEY_SEP,
  type ModelId,
  type ModelOption,
} from '@/types/chat'

interface ModelPickerProps {
  value: ModelId
  options: ModelOption[]
  onChange: (next: ModelId) => void
  disabled?: boolean
  loading?: boolean
  className?: string
}

function groupByProvider(options: ModelOption[]): SelectGroup<ModelId>[] {
  const byProvider = new Map<string, { value: ModelId; label: string }[]>()
  for (const opt of options) {
    const provider = opt.id.split(CATALOG_MODEL_KEY_SEP)[0] || '—'
    const bucket = byProvider.get(provider) ?? []
    bucket.push({ value: opt.id, label: opt.label })
    byProvider.set(provider, bucket)
  }
  return [...byProvider.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([label, opts]) => ({ label, options: opts }))
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
      groups={groupByProvider(pickerOptions)}
      onChange={onChange}
      disabled={disabled || loading}
      aria-label={loading ? 'model (loading catalog)' : 'model'}
      aria-busy={loading || undefined}
      className={className}
    />
  )
}
