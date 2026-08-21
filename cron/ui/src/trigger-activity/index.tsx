import type { TriggerActivityMessage, TriggerActivityRenderer } from '@iii-dev/console-ui'
import { describeCron } from '../lib/cron'

interface CronTriggerConfig {
  expression: string
  conditionFunctionId?: string
}

export function readCronConfig(value: unknown): CronTriggerConfig | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const config = value as Record<string, unknown>
  if (typeof config.expression !== 'string') return null
  const expression = config.expression.trim()
  if (!expression) return null

  const conditionFunctionId =
    typeof config.condition_function_id === 'string' ? config.condition_function_id.trim() || undefined : undefined
  return { expression, conditionFunctionId }
}

function CronTriggerSource({ activity }: { activity: TriggerActivityMessage }) {
  const config = readCronConfig(activity.config)
  if (!config) return null

  const description = describeCron(config.expression)
  return (
    <section className="cron-trigger-activity" aria-label="Cron schedule">
      <div className="cron-trigger-activity__heading">
        <span className="cron-trigger-activity__schedule">{description ?? 'Custom cron schedule'}</span>
        <span className="cron-trigger-activity__timezone">UTC</span>
      </div>
      <code className="cron-trigger-activity__expression">{config.expression}</code>
      {config.conditionFunctionId ? (
        <div className="cron-trigger-activity__condition">
          <span className="cron-trigger-activity__condition-label">Condition</span>
          <code className="cron-trigger-activity__condition-value">{config.conditionFunctionId}</code>
        </div>
      ) : null}
    </section>
  )
}

export function createCronTriggerActivityRenderer(): TriggerActivityRenderer {
  return {
    id: 'cron/page.js#trigger-activity',
    isMatch: (triggerType) => triggerType === 'cron',
    tryRender: (activity) => {
      if (activity.triggerType !== 'cron' || !readCronConfig(activity.config)) {
        return null
      }
      return <CronTriggerSource activity={activity} />
    },
  }
}
