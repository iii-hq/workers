import inkUrl from '@/icons/iii-ink.svg?url'
import whiteUrl from '@/icons/iii-white.svg?url'
import { cn } from '@/lib/utils'

interface WordmarkProps {
  className?: string
  /**
   * `auto` swaps ink/white with `[data-theme]` (default). `ink` and `inverse`
   * pin a variant for panels that don't follow the page theme.
   */
  tone?: 'auto' | 'ink' | 'inverse'
}

const IMG_CLASS = 'h-[19px] w-auto shrink-0'

/**
 * The "iii" wordmark from brand SVGs (`iii-ink.svg` / `iii-white.svg`).
 */
export function Wordmark({ className, tone = 'auto' }: WordmarkProps) {
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
