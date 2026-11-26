import { List, ListItem } from '@iii-dev/console-ui'
import { Check } from 'lucide-react'

/** `reached`: where the flow is, while an earlier step shows (a number, and a link back to it). */
export type StepState = 'done' | 'current' | 'reached' | 'todo'
export type StepItem<T extends string = string> = { id: T; title: string; sub?: string; state: StepState }

/**
 * The step rail of Setup and Deploy: a number (a check once done) and the title, `aria-current` on the
 * step shown, a line down to the next. Done steps and the reached one go back to themselves; steps ahead
 * are plain rows, legible but not links.
 */
export function Steps<T extends string>({
  label,
  steps,
  onGo,
}: {
  label: string
  steps: StepItem<T>[]
  onGo: (id: T) => void
}) {
  return (
    <List aria-label={label} className="iii-cloud-rail">
      {steps.map((s, i) => (
        <ListItem
          key={s.id}
          as={s.state === 'todo' ? 'div' : 'button'}
          className="iii-cloud-rail-step"
          data-step={s.state}
          aria-current={s.state === 'current' ? 'step' : undefined}
          leading={
            <span className="iii-cloud-rail-mark" aria-hidden>
              {s.state === 'done' ? <Check /> : i + 1}
            </span>
          }
          label={s.title}
          description={s.sub}
          onClick={s.state === 'todo' ? undefined : () => onGo(s.id)}
        />
      ))}
    </List>
  )
}

/**
 * Each step's state: the one shown current; before `reached` (the step the flow is at, the shown one by default)
 * done; `reached` itself, when an earlier one shows, a link back; after it todo.
 */
export function stepStates<T extends string>(
  order: readonly T[],
  at: T,
  titles: Record<T, { title: string; sub?: string }>,
  reached: T = at,
) {
  const n = order.indexOf(at)
  const r = order.indexOf(reached)
  return order.map(
    (id, i): StepItem<T> => ({
      id,
      ...titles[id],
      state: i === n ? 'current' : i < r ? 'done' : i === r ? 'reached' : 'todo',
    }),
  )
}
