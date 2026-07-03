/**
 * Common label + description + error chrome for primitive (leaf) field
 * components. Object / dictionary / array fields render their own headers
 * because the spacing and emphasis differ, so this shell intentionally
 * targets only the single-control case.
 *
 * The label is rendered as a small uppercased eyebrow label so it stays
 * visually distinct from the schema's `title`, which the section header
 * uses for object groups. Description (when present) lives directly under
 * the control as small ink-faint helper text so the operator can scan
 * down the form without bouncing back up between the label and the input.
 */

import type { ReactNode } from 'react'
import { useId } from 'react'
import { cn } from '@/lib/utils'
import { wt } from '../typography'

interface FieldShellProps {
  label: string
  description?: string
  required?: boolean
  errorMessage?: string | null
  /**
   * Render-prop for the control itself. Receives the generated DOM id
   * so the control's `id` matches the rendered `<label htmlFor>` (the
   * shell does its own labelling so callers never forget).
   */
  children: (controlId: string) => ReactNode
  className?: string
  anchorId?: string
  /**
   * Suppress the label row + description so the bare control can sit
   * inline (e.g. as the value cell of a dictionary row or an array item,
   * where the row's key / position already supplies identity). The error
   * message is still rendered so per-row validation surfaces.
   */
  hideLabel?: boolean
}

export function FieldShell({
  label,
  description,
  required,
  errorMessage,
  children,
  className,
  anchorId,
  hideLabel,
}: FieldShellProps) {
  const generatedControlId = useId()
  const controlId = anchorId ? `${anchorId}-control` : generatedControlId
  const descriptionId = `${controlId}-desc`
  const errorId = `${controlId}-err`
  const showDescription = !hideLabel && !!description

  return (
    <div
      id={anchorId}
      tabIndex={anchorId ? -1 : undefined}
      className={cn(
        'space-y-1.5 scroll-mt-24 focus:outline-none focus:ring-1 focus:ring-accent/60 focus:ring-offset-4 focus:ring-offset-bg',
        className,
      )}
    >
      {hideLabel ? null : (
        <div className="flex items-baseline gap-2">
          <label
            htmlFor={controlId}
            className={cn(
              wt.caption,
              'uppercase tracking-[0.06em] text-ink-faint',
            )}
          >
            {label}
          </label>
          {required ? (
            <span
              className={cn(wt.micro, 'text-accent')}
              title="required by schema"
            >
              [required]
            </span>
          ) : null}
        </div>
      )}
      <div aria-describedby={showDescription ? descriptionId : undefined}>
        {children(controlId)}
      </div>
      {showDescription ? (
        <p
          id={descriptionId}
          className={cn(wt.caption, 'text-ink-faint leading-relaxed')}
        >
          {description}
        </p>
      ) : null}
      {errorMessage ? (
        <p id={errorId} role="alert" className={cn(wt.caption, 'text-alert')}>
          {errorMessage}
        </p>
      ) : null}
    </div>
  )
}

/* ---------------------------------------------------------------------- */
/*  Shared helpers                                                       */
/* ---------------------------------------------------------------------- */

import type { FieldProps } from './FieldDispatch'
import { pathToPointer } from './path'

/**
 * Look up the server-side error message for a field at the given path.
 * Returns `null` (not `undefined`) when there is no error, so JSX
 * conditionals stay tidy (`errorMessage ?? null`).
 */
export function errorForField(
  props: Pick<FieldProps, 'errors' | 'path'>,
): string | null {
  if (!props.errors) return null
  return props.errors.get(pathToPointer(props.path)) ?? null
}
