import { ProviderIcon } from '@/components/chat/ProviderIcon'
import { bundledProviderMark } from '@/lib/provider-marks'

// Use the same marks the workers declare to the router, even before install.
// CSS masks keep them crisp, monochrome, and legible in either theme.
export function ProviderMark({
  id,
  label,
  className,
}: {
  id: string
  label: string
  className?: string
}) {
  return (
    <ProviderIcon
      iconSvg={bundledProviderMark(id)}
      label={label}
      className={className}
    />
  )
}
