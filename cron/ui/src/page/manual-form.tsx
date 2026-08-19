import { Button, CodeEditor, Input, Select, Selector, StatusPanel } from '@iii-dev/console-ui'
import { useMemo, useState } from 'react'
import { Field, TextArea } from '../components'
import type { FunctionSummary } from '../lib/api'
import { describeCron, nextCronRun, validateCron } from '../lib/cron'

export interface ManualTaskSpec {
  label: string
  expression: string
  delivery: 'notify' | 'call'
  functionId?: string
  payload?: unknown
  eventInto?: string
  maxFires?: number
}

const PRESETS = [
  { value: 'hourly', label: 'Every hour', expression: '0 0 * * * *' },
  { value: 'daily', label: 'Daily at 09:00 UTC', expression: '0 0 9 * * *' },
  { value: 'weekdays', label: 'Weekdays at 09:00 UTC', expression: '0 0 9 * * 2-6' },
  { value: 'weekly', label: 'Mondays at 09:00 UTC', expression: '0 0 9 * * 2' },
  { value: 'custom', label: 'Custom expression', expression: '' },
] as const

export function ManualTaskForm({
  functions,
  sending,
  onClose,
  onSubmit,
}: {
  functions: FunctionSummary[]
  sending: boolean
  onClose: () => void
  onSubmit: (spec: ManualTaskSpec) => void
}) {
  const [label, setLabel] = useState('')
  const [preset, setPreset] = useState('daily')
  const [expression, setExpression] = useState('0 0 9 * * *')
  const [delivery, setDelivery] = useState<'notify' | 'call'>('notify')
  const [functionId, setFunctionId] = useState<string | undefined>()
  const [payloadText, setPayloadText] = useState('{}')
  const [eventInto, setEventInto] = useState('/cron_event')
  const [maxFires, setMaxFires] = useState('')
  const [formError, setFormError] = useState<string | null>(null)

  /** What this expression actually means, before it is registered: the plain
      reading, then the next few literal times. A six-field slip usually looks
      right and only reveals itself at the first fire. */
  const expressionError = validateCron(expression)
  const cadence = expressionError ? null : describeCron(expression)
  const upcoming = useMemo(() => {
    if (expressionError) return []
    const times: string[] = []
    let cursor = new Date()
    for (let index = 0; index < 3; index += 1) {
      const next = nextCronRun(expression, cursor)
      if (!next) break
      times.push(`${next.toISOString().slice(0, 16).replace('T', ' ')} UTC`)
      cursor = next
    }
    return times
  }, [expression, expressionError])

  // Grouped by owning worker, because the catalog is every function on the bus.
  const functionGroups = useMemo(() => {
    const byWorker = new Map<string, { value: string; label: string; description?: string }[]>()
    for (const fn of functions) {
      const worker = fn.functionId.split('::')[0] ?? 'other'
      const options = byWorker.get(worker) ?? []
      options.push({
        value: fn.functionId,
        label: fn.functionId,
        description: fn.description,
      })
      byWorker.set(worker, options)
    }
    return [...byWorker.entries()]
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([label, options]) => ({ label, options }))
  }, [functions])

  const submit = () => {
    if (!label.trim()) {
      setFormError('Describe what the task should do.')
      return
    }
    const cronError = validateCron(expression)
    if (cronError) {
      setFormError(cronError)
      return
    }
    if (delivery === 'call' && !functionId) {
      setFormError('Choose the function that should receive each cron event.')
      return
    }
    let payload: unknown
    if (delivery === 'call') {
      try {
        payload = JSON.parse(payloadText)
      } catch {
        setFormError('Fixed payload must be valid JSON.')
        return
      }
    }
    const max = maxFires.trim() ? Number(maxFires) : undefined
    if (max !== undefined && (!Number.isInteger(max) || max < 1)) {
      setFormError('Maximum runs must be a positive whole number.')
      return
    }
    setFormError(null)
    onSubmit({
      label: label.trim(),
      expression: expression.trim(),
      delivery,
      functionId,
      payload,
      eventInto: eventInto.trim(),
      maxFires: max,
    })
  }

  return (
    <div className="cron-ui-inspector-inner cron-ui-form-panel">
      <header className="cron-ui-detail-head">
        <h2 className="cron-ui-detail-title">New schedule</h2>
      </header>
      <p className="cron-ui-inspector-copy">
        The active agent registers the task so Harness can persist ownership, lifecycle, and fire count.
      </p>

      <Field label="Task" htmlFor="cron-task-label">
        <TextArea
          id="cron-task-label"
          value={label}
          onChange={setLabel}
          placeholder="Prepare a daily brief of open work"
        />
      </Field>

      <Field label="Schedule">
        <Select
          value={preset}
          options={PRESETS.map((item) => ({ value: item.value, label: item.label }))}
          onChange={(value) => {
            setPreset(value)
            const selected = PRESETS.find((item) => item.value === value)
            if (selected?.expression) setExpression(selected.expression)
          }}
          aria-label="Schedule preset"
        />
      </Field>

      <Field
        label="Cron expression"
        htmlFor="cron-task-expression"
        hint={
          expressionError ?? (
            <>
              second minute hour day month weekday, optional year. Times are UTC.
              {upcoming.length > 0 ? (
                <>
                  {' '}
                  {cadence ? `${cadence}. ` : ''}
                  Next: {upcoming.join(', ')}.
                </>
              ) : null}
            </>
          )
        }
      >
        <Input
          id="cron-task-expression"
          value={expression}
          onChange={(value) => {
            setExpression(value)
            setPreset('custom')
          }}
          placeholder="0 0 9 * * *"
        />
      </Field>

      <Field
        label="Delivery"
        hint="A conversation wake lets an agent perform the routine. Function calls run mechanically without starting an agent."
      >
        <Select
          value={delivery}
          options={[
            { value: 'notify', label: 'Wake a conversation' },
            { value: 'call', label: 'Call a function' },
          ]}
          onChange={(value) => setDelivery(value)}
          aria-label="Task delivery"
        />
      </Field>

      {delivery === 'call' ? (
        <>
          <Field label="Target function">
            <Selector
              value={functionId}
              groups={functionGroups}
              onChange={setFunctionId}
              placeholder="Choose a function"
              searchPlaceholder="Search functions"
              emptyMessage="No function matches"
              aria-label="Target function"
            />
          </Field>
          <Field
            label="Event JSON pointer"
            htmlFor="cron-task-event-path"
            hint="Where the fire event is written inside the payload."
          >
            <Input id="cron-task-event-path" value={eventInto} onChange={setEventInto} placeholder="/cron_event" />
          </Field>
          <Field label="Fixed payload" className="cron-ui-editor-field">
            <CodeEditor
              value={payloadText}
              onChange={setPayloadText}
              language="json"
              className="cron-ui-code-editor"
              aria-label="Fixed target payload"
            />
          </Field>
        </>
      ) : null}

      <Field
        label="Maximum runs"
        htmlFor="cron-task-max-runs"
        hint="Leave empty for a recurring task that runs until removed."
      >
        <Input
          id="cron-task-max-runs"
          type="number"
          min={1}
          step={1}
          value={maxFires}
          onChange={setMaxFires}
          placeholder="unbounded"
        />
      </Field>

      {formError ? <StatusPanel variant="alert" headline={formError} /> : null}

      <div className="cron-ui-form-actions">
        <Button variant="ghost" size="sm" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" size="sm" disabled={sending} onClick={submit}>
          {sending ? 'Creating…' : 'Create schedule'}
        </Button>
      </div>
    </div>
  )
}
