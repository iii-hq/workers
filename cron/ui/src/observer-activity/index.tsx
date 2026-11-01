import type { ObserverActivityMessage, ObserverActivityRenderer } from '@iii-dev/console-ui'
import { ON_TICK_FN } from '../lib/api'
import { describeCron } from '../lib/cron'

/** The expression of a `cron::on-tick` observer's args, or null. */
export function readCronArgs(value: unknown): { expression: string } | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const args = value as Record<string, unknown>
  if (typeof args.expression !== 'string') return null
  const expression = args.expression.trim()
  return expression ? { expression } : null
}

function CronObserverSource({ activity }: { activity: ObserverActivityMessage }) {
  const args = readCronArgs(activity.args)
  if (!args) return null

  const description = describeCron(args.expression)
  return (
    <section className="cron-observer-activity" aria-label="Cron schedule">
      <div className="cron-observer-activity__heading">
        <span className="cron-observer-activity__schedule">{description ?? 'Custom cron schedule'}</span>
        <span className="cron-observer-activity__timezone">UTC</span>
      </div>
      <code className="cron-observer-activity__expression">{args.expression}</code>
    </section>
  )
}

export function createCronObserverActivityRenderer(): ObserverActivityRenderer {
  return {
    id: 'cron/page.js#observer-activity',
    isMatch: (functionId) => functionId === ON_TICK_FN,
    tryRender: (activity) => {
      if (activity.functionId !== ON_TICK_FN || !readCronArgs(activity.args)) {
        return null
      }
      return <CronObserverSource activity={activity} />
    },
  }
}
