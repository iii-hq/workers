import type { CSSProperties } from 'react'
import { providerBrandColor } from '@/lib/provider-brand'
import { providerIconMaskUrl, providerInitial } from '@/lib/provider-icon'
import { cn } from '@/lib/utils'
import './ProviderIcon.css'

interface ProviderIconProps {
  /** Inline SVG declared by the provider worker; absent → initial-letter tile. */
  iconSvg?: string | null
  /** Provider display name, for the fallback initial. */
  label: string
  /**
   * The provider's id: a brand with a colour of its own paints its mark in
   * it (`lib/provider-brand`); absent or unknown, the mark takes the text
   * colour like a Lucide glyph.
   */
  providerId?: string | null
  /** Size classes; defaults to the shared 16px icon baseline. */
  className?: string
}

/**
 * The provider's mark at the shared icon size. Workers ship the SVG with their
 * router declaration (`icon_svg`); a provider without one gets a quiet
 * initial-letter tile so the rail stays scannable either way.
 */
export function ProviderIcon({
  iconSvg,
  label,
  providerId,
  className,
}: ProviderIconProps) {
  const mask = providerIconMaskUrl(iconSvg)
  const brand = providerBrandColor(providerId)
  if (mask) {
    return (
      <span
        aria-hidden
        data-provider-icon="mark"
        data-provider-brand={brand ? '' : undefined}
        className={cn('provider-icon size-4 shrink-0', className)}
        style={
          {
            '--provider-icon-mask': mask,
            ...(brand ? { color: brand } : {}),
          } as CSSProperties
        }
      />
    )
  }
  return (
    <span
      aria-hidden
      data-provider-icon="initial"
      className={cn(
        'flex size-4 shrink-0 items-center justify-center rounded-xs bg-surface-selected font-sans text-[11px] font-semibold leading-none text-ink-faint',
        className,
      )}
    >
      {providerInitial(label)}
    </span>
  )
}
