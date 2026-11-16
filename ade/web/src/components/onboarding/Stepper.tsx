import { Check } from 'lucide-react'
import type { WizardStepId } from '@/lib/onboarding/open'
import { cn } from '@/lib/utils'

export interface StepperStep {
  id: WizardStepId
  title: string
  /** One short line under the title, shown in the vertical rail. */
  description?: string
  optional?: boolean
}

/**
 * Setup's progress: a numbered dot per step joined by a hairline that fills
 * as steps complete. Finished steps carry a check and, while setup is still
 * in progress, go back to that step on click; the current one — the last
 * included — is marked current (`aria-current="step"`, its number in a
 * ringed dot), never done; the ones ahead are plain text, so the tab order
 * only ever stops on something that does something.
 *
 * `vertical` is the dialog's left rail, with a line under each title.
 * `horizontal` is the narrow fallback, one row of dots. Below the `@md`
 * container width the inline titles fold away and the current step is named
 * on a line under the dots ("Step 2 of 5 · Models"): an inline title that
 * came and went with the step moved every dot along the row, and on a phone
 * cut "Judge" to "J…".
 */
export function Stepper({
  steps,
  current,
  reachable,
  onSelect,
  orientation = 'horizontal',
  className,
}: {
  steps: readonly StepperStep[]
  current: WizardStepId
  /** Steps the person may jump back to. */
  reachable: ReadonlySet<WizardStepId>
  onSelect: (step: WizardStepId) => void
  orientation?: 'horizontal' | 'vertical'
  className?: string
}) {
  const vertical = orientation === 'vertical'
  const index = steps.findIndex((step) => step.id === current)
  const list = (
    <ol
      aria-label="Setup steps"
      className={cn(
        'flex',
        vertical ? 'flex-col' : 'flex-row items-center',
        vertical && className,
      )}
    >
      {steps.map((step, position) => {
        const isCurrent = position === index
        const last = position === steps.length - 1
        // Done means behind the current step; the current one, the last
        // included, is current until setup moves past it.
        const done = position < index
        const clickable = reachable.has(step.id) && !isCurrent
        const content = (
          <>
            <span
              aria-hidden
              className={cn(
                'relative z-10 flex size-6 shrink-0 items-center justify-center rounded-full border text-xs font-semibold tabular-nums transition-[background-color,border-color,color,box-shadow] duration-200 ease-[var(--motion-ease-standard)]',
                done
                  ? 'border-input bg-ink text-bg'
                  : isCurrent
                    ? 'border-input bg-card text-ink ring-[3px] ring-ink/10 '
                    : 'border-input bg-card text-muted-foreground  ',
                clickable && 'group-hover:border-input',
              )}
            >
              <span
                className={cn(
                  'absolute transition-[opacity,transform,filter] duration-200 ease-[var(--motion-ease-standard)] motion-reduce:transition-none',
                  done
                    ? 'scale-25 opacity-0 blur-[4px]'
                    : 'scale-100 opacity-100',
                )}
              >
                {position + 1}
              </span>
              <Check
                className={cn(
                  'absolute size-4 transition-[opacity,transform,filter] duration-200 ease-[var(--motion-ease-standard)] motion-reduce:transition-none',
                  done
                    ? 'scale-100 opacity-100'
                    : 'scale-25 opacity-0 blur-[4px]',
                )}
                strokeWidth={2.5}
              />
            </span>
            <span
              className={cn(
                'min-w-0 flex-col transition-colors duration-200',
                vertical
                  ? 'mt-0.5 flex gap-px'
                  : cn(
                      'whitespace-nowrap',
                      // Only visually hidden: the label names the step's
                      // button. Every step's alike, so no dot moves.
                      'sr-only @md:not-sr-only @md:flex',
                    ),
              )}
            >
              <span
                className={cn(
                  'text-sm font-medium leading-5',
                  !vertical && 'truncate',
                  done || isCurrent ? 'text-ink' : 'text-muted-foreground ',
                  clickable && 'group-hover:text-ink',
                )}
              >
                {step.title}
                {step.optional ? (
                  <span
                    className={cn(
                      'ml-1 text-xs font-normal text-muted-foreground',
                      !vertical && 'hidden @md:inline',
                    )}
                  >
                    optional
                  </span>
                ) : null}
                <span className="sr-only">
                  {done ? ', completed' : isCurrent ? ', current step' : ''}
                </span>
              </span>
              {vertical && step.description ? (
                <span
                  className={cn(
                    'truncate text-body leading-4',
                    isCurrent
                      ? 'text-muted-foreground '
                      : 'text-muted-foreground ',
                  )}
                >
                  {step.description}
                </span>
              ) : null}
            </span>
          </>
        )
        // In the rail the dot sits at the row's top edge, so the connector
        // can run from one dot's bottom edge to the next dot's top edge.
        const itemClass = cn(
          'flex min-w-0 gap-2.5',
          vertical ? 'w-full shrink-0 items-start' : 'h-8 items-center py-1',
        )
        return (
          <li
            key={step.id}
            className={cn(
              'relative flex min-w-0',
              vertical
                ? cn('flex-col', !last && 'pb-5')
                : cn('items-center gap-2', !last && 'flex-1'),
            )}
          >
            {clickable ? (
              <button
                type="button"
                onClick={() => onSelect(step.id)}
                className={cn(
                  itemClass,
                  'group -mx-2 -my-1 rounded-lg px-2 py-1 text-left font-sans transition-colors duration-150 hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:ring-offset-2 focus-visible:ring-offset-white',
                  vertical ? 'w-[calc(100%+16px)]' : 'h-8',
                )}
              >
                {content}
              </button>
            ) : (
              <span
                aria-current={isCurrent ? 'step' : undefined}
                className={itemClass}
              >
                {content}
              </span>
            )}
            {!last ? (
              <span
                aria-hidden
                className={cn(
                  'overflow-hidden rounded-full bg-surface-selected',
                  vertical
                    ? 'absolute top-6 bottom-0 left-[11.5px] w-px'
                    : 'relative h-px min-w-3 flex-1',
                )}
              >
                <span
                  className={cn(
                    'absolute inset-0 bg-ink transition-transform duration-300 ease-[var(--motion-ease-standard)] motion-reduce:transition-none',
                    vertical ? 'origin-top' : 'origin-left',
                    position < index
                      ? 'scale-100'
                      : vertical
                        ? 'scale-y-0'
                        : 'scale-x-0',
                  )}
                />
              </span>
            ) : null}
          </li>
        )
      })}
    </ol>
  )
  if (vertical) return list
  const here = steps[index]
  return (
    <div className={cn('flex flex-col gap-1', className)}>
      {list}
      {here ? (
        // The steps are named in the list for screen readers already.
        <p
          aria-hidden
          data-stepper-caption
          className="truncate text-sm leading-5 text-muted-foreground @md:hidden"
        >
          Step {index + 1} of {steps.length} ·{' '}
          <span className="font-medium text-ink">{here.title}</span>
        </p>
      ) : null}
    </div>
  )
}
