import { useId } from 'react'
import inkUrl from '@/icons/iii-ink.svg?url'
import whiteUrl from '@/icons/iii-white.svg?url'
import { cn } from '@/lib/utils'

interface WordmarkProps {
  className?: string
  /** A recessed, low-contrast treatment for large decorative placements. */
  appearance?: 'default' | 'inset' | 'loading'
  /**
   * `auto` swaps ink/white with `[data-theme]` (default). `ink` and `inverse`
   * pin a variant for panels that don't follow the page theme.
   */
  tone?: 'auto' | 'ink' | 'inverse'
}

const IMG_CLASS = 'h-[32px] mx-[6px] w-auto shrink-0'
const WORDMARK_COLUMNS = [0, 403.4, 806.81] as const

function WordmarkGlyphs({
  animated = false,
  trademark = true,
}: {
  animated?: boolean
  trademark?: boolean
}) {
  return (
    <>
      {WORDMARK_COLUMNS.map((x, index) => (
        <g
          key={x}
          className={cn(
            animated && 'model-waiting-wordmark-segment',
            animated && index === 1 && '[animation-delay:120ms]',
            animated && index === 2 && '[animation-delay:240ms]',
          )}
        >
          <rect x={x} y="403.45" width="268.94" height="672.24" />
          <rect x={x} y=".05" width="268.94" height="268.94" />
        </g>
      ))}
      {trademark ? (
        <path d="M1008.82,1025.81h-13.67v-6.06h34.68v6.06h-13.82v37.22h-7.19v-37.22ZM1043.08,1028.77v34.26h-6.77v-43.28h9.73l13.67,27.91,13.11-27.91h9.3v43.28h-7.05v-34.82l-13.39,27.77h-5.22l-13.39-27.21Z" />
      ) : null}
    </>
  )
}

/**
 * The "iii" wordmark from brand SVGs (`iii-ink.svg` / `iii-white.svg`).
 */
export function Wordmark({
  className,
  appearance = 'default',
  tone = 'auto',
}: WordmarkProps) {
  const filterId = `iii-inset-${useId().replaceAll(':', '')}`

  if (appearance === 'loading') {
    return (
      <svg
        viewBox="0 0 1075.74 1075.74"
        aria-hidden="true"
        className={cn('size-6 shrink-0 fill-ink', className)}
      >
        <WordmarkGlyphs animated trademark={false} />
      </svg>
    )
  }

  if (appearance === 'inset') {
    return (
      <svg
        viewBox="0 0 1075.74 1075.74"
        role="img"
        aria-label="iii"
        className={cn('size-16 shrink-0 fill-ink-disabled/45', className)}
      >
        <defs>
          <filter
            id={filterId}
            x="-10%"
            y="-10%"
            width="120%"
            height="120%"
            colorInterpolationFilters="sRGB"
          >
            <feGaussianBlur in="SourceAlpha" stdDeviation="20" result="blur" />
            <feOffset in="blur" dy="18" result="offsetBlur" />
            <feComposite
              in="SourceAlpha"
              in2="offsetBlur"
              operator="out"
              result="innerCut"
            />
            <feFlood
              result="shadowColor"
              className="[flood-color:#0a0a0a] [flood-opacity:0.32]"
            />
            <feComposite
              in="shadowColor"
              in2="innerCut"
              operator="in"
              result="innerShadow"
            />
            <feComposite in="innerShadow" in2="SourceGraphic" operator="over" />
          </filter>
        </defs>
        <g filter={`url(#${filterId})`}>
          <WordmarkGlyphs />
        </g>
      </svg>
    )
  }

  const sizeClass = cn(IMG_CLASS, className)

  if (tone === 'ink') {
    return <img src={inkUrl} alt="iii" className={sizeClass} />
  }

  if (tone === 'inverse') {
    return <img src={whiteUrl} alt="iii" className={sizeClass} />
  }

  return (
    <span role="img" aria-label="iii" className="inline-flex">
      <img
        src={inkUrl}
        alt=""
        aria-hidden
        className={cn(sizeClass, '[html[data-theme=dark]_&]:hidden')}
      />
      <img
        src={whiteUrl}
        alt=""
        aria-hidden
        className={cn(sizeClass, 'hidden [html[data-theme=dark]_&]:inline')}
      />
    </span>
  )
}
