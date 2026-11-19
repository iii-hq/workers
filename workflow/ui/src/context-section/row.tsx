import uiClasses from '@iii-dev/console-ui/ui-classes'
import { ChevronDown } from 'lucide-react'
import type { ButtonHTMLAttributes, CSSProperties, ReactNode } from 'react'

/**
 * The rows a section shows: `cap` of them, or all when expanded, or when the
 * only row left would be the one a "Show 1 more" button would cost.
 */
export function capRows<T>(
  rows: readonly T[],
  cap: number,
  expanded: boolean,
): { shown: readonly T[]; hidden: number } {
  if (expanded || rows.length <= cap + 1) return { shown: rows, hidden: 0 }
  return { shown: rows.slice(0, cap), hidden: rows.length - cap }
}

interface RowProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'title'> {
  icon: ReactNode
  label: ReactNode
  /** A second line under the label. */
  description?: ReactNode
  /** Status at the row's end. */
  meta?: ReactNode
  title?: string
  ariaLabel?: string
  onActivate?: () => void
}

/** A two-line row keeps the recipe's padding and grows to the list height. */
const TWO_LINE_ROW = { '--iii-ui-tree-row': '2.75rem' } as CSSProperties

/** One tree row of the Context panel, the button itself. */
export function Row({
  icon,
  label,
  description,
  meta,
  title,
  ariaLabel,
  onActivate,
  onClick,
  style,
  ...rest
}: RowProps) {
  const labelNode = <span className={`${uiClasses.treeItemLabel} wf-ctx-row-label`}>{label}</span>
  return (
    <button
      type="button"
      title={title}
      aria-label={ariaLabel}
      className={uiClasses.treeItem}
      style={description ? { ...TWO_LINE_ROW, ...style } : style}
      onClick={(event) => {
        onClick?.(event)
        if (!event.defaultPrevented) onActivate?.()
      }}
      {...rest}
    >
      <span className={uiClasses.treeItemIcon}>{icon}</span>
      {description ? (
        <span className="wf-ctx-row-text">
          {labelNode}
          <span className="wf-ctx-row-line">{description}</span>
        </span>
      ) : (
        labelNode
      )}
      {/* Not the recipe's trailing slot: that one hides on hover for actions this row has none of. */}
      {meta ? <span className="wf-ctx-row-meta">{meta}</span> : null}
    </button>
  )
}

export function ShowMore({
  hidden,
  expanded,
  total,
  cap,
  onToggle,
  style,
}: {
  hidden: number
  expanded: boolean
  total: number
  cap: number
  onToggle: () => void
  style?: CSSProperties
}) {
  // "Show less" only once something was actually folded away.
  if (hidden === 0 && !(expanded && total > cap + 1)) return null
  return (
    <button type="button" className={uiClasses.treeItem} style={style} aria-expanded={expanded} onClick={onToggle}>
      <span className={uiClasses.treeItemIcon} />
      <span className={`${uiClasses.treeItemLabel} wf-ctx-more-label`}>
        {expanded ? 'Show less' : `Show ${hidden} more`}
      </span>
      <span className={uiClasses.treeItemTrailing}>
        <ChevronDown aria-hidden className={expanded ? 'wf-ctx-more-caret wf-ctx-open' : 'wf-ctx-more-caret'} />
      </span>
    </button>
  )
}
