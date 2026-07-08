/**
 * Common label + description + error chrome for primitive (leaf) field
 * components. Object / dictionary / array fields render their own headers
 * because the spacing and emphasis differ, so this shell intentionally
 * targets only the single-control case.
 *
 * The label is rendered as a small uppercased eyebrow label so it stays
 * visually distinct from the schema's `title`, which the section header
 * uses for object groups. The schema description is surfaced through a
 * `?` tooltip next to the label instead of inline text: the registered
 * schemas carry multi-line technical prose, and rendering it under every
 * control detached labels from their inputs and made long forms
 * unscannable. Assistive tech still gets the full text via an sr-only
 * paragraph wired to the control with `aria-describedby`.
 */

import type { ReactNode } from 'react'
import { useId } from 'react'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
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
          {showDescription ? <FieldHelp description={description} /> : null}
        </div>
      )}
      <div aria-describedby={showDescription ? descriptionId : undefined}>
        {children(controlId)}
      </div>
      {showDescription ? (
        <p id={descriptionId} className="sr-only">
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

/**
 * `?` glyph that reveals a schema description in a tooltip on hover or
 * keyboard focus. Wraps its own TooltipProvider so it works in any mount
 * (Storybook has no app-root provider). `normal-case` overrides the
 * lowercase default of `TooltipContent` — descriptions are prose, not
 * console chrome.
 */
export function FieldHelp({ description }: { description: string }) {
  return (
    <TooltipProvider delayDuration={150}>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            aria-label="field help"
            className={cn(
              wt.micro,
              'inline-flex h-4 w-4 items-center justify-center border border-rule',
              'text-ink-ghost hover:text-ink hover:border-ink transition-colors',
              'align-middle select-none',
            )}
          >
            ?
          </button>
        </TooltipTrigger>
        <TooltipContent
          side="right"
          align="start"
          className="normal-case max-w-[420px] text-left leading-relaxed text-ink-faint"
        >
          {description}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
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
