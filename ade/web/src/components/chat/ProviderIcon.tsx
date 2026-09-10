import type { CSSProperties } from 'react'
import { providerIconMaskUrl, providerInitial } from '@/lib/provider-icon'
import { cn } from '@/lib/utils'
import './ProviderIcon.css'

interface ProviderIconProps {
  /** Inline SVG declared by the provider worker; absent → initial-letter tile. */
  iconSvg?: string | null
  /** Provider display name, for the fallback initial. */
  label: string
  /** Size classes; defaults to the shared 16px icon baseline. */
  className?: string
}

/**
 * The provider's mark at the shared icon size. Workers ship the SVG with their
 * router declaration (`icon_svg`); a provider without one gets a quiet
 * initial-letter tile so the rail stays scannable either way.
 */
export function ProviderIcon({ iconSvg, label, className }: ProviderIconProps) {
  const mask = providerIconMaskUrl(iconSvg)
  if (mask) {
    return (
      <span
        aria-hidden
        data-provider-icon="mark"
        className={cn('provider-icon size-4 shrink-0', className)}
        style={{ '--provider-icon-mask': mask } as CSSProperties}
      />
    )
  }
  return (
    <span
      aria-hidden
      data-provider-icon="initial"
      className={cn(
        'flex size-4 shrink-0 items-center justify-center rounded-xs bg-surface-selected font-sans text-[10px] font-semibold leading-none text-ink-faint',
        className,
      )}
    >
      {providerInitial(label)}
    </span>
  )
}
