import { cn } from '@/lib/utils'
import { KeyCombo } from './KeyCombo'

const CAP = {
  /** Hairline boxes that read as labels, beside a quiet action. */
  outline:
    'h-5 min-w-5 rounded-[5px] border border-border bg-transparent px-1 font-inter text-meta font-medium text-muted-foreground shadow-none',
  /** The same caps inside a primary button, on its ink fill. */
  'on-primary':
    'h-5 min-w-5 rounded-[5px] bg-primary-foreground/15 px-1 font-inter text-meta font-medium text-primary-foreground shadow-none',
} as const

/**
 * A shortcut shown inside a button or a footer (the Add folder picker): the
 * `KeyCombo` caps as flat labels rather than raised keys.
 */
export function KeyHint({
  binding,
  tone = 'outline',
  className,
}: {
  binding: string
  tone?: keyof typeof CAP
  /** Layout only (a margin, shrink). */
  className?: string
}) {
  return (
    <KeyCombo
      binding={binding}
      className={cn('gap-0.5', className)}
      capClassName={CAP[tone]}
    />
  )
}
