import { Button, Input, SegmentedControl } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { type FormEvent, type RefObject, useId, useState } from 'react'
import type { EvalApi } from '../api'
import type { CycleInput } from '../types'

type Entry = 'objective' | 'execution'

const ENTRIES = [
  { value: 'objective' as const, label: 'Objective', icon: false as const },
  { value: 'execution' as const, label: 'Execution', icon: false as const },
]

/** The one start form: a free objective, or the id of a finished E2E execution. Both start the same cycle. */
export function StartCycle({
  api,
  inputRef,
  onStarted,
}: {
  api: EvalApi
  inputRef: RefObject<HTMLInputElement | null>
  onStarted: (cycleId: string) => void
}) {
  const [entry, setEntry] = useState<Entry>('objective')
  const [value, setValue] = useState('')
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const inputId = useId()
  const hintId = useId()
  const text = value.trim()

  async function submit(event: FormEvent) {
    event.preventDefault()
    if (!text || pending) return
    const input: CycleInput = entry === 'objective' ? { objective: text } : { execution_id: text }
    setPending(true)
    setError(null)
    try {
      const started = await api.start(input)
      setValue('')
      onStarted(started.cycle_id)
    } catch (failure) {
      setError(errorMessage(failure))
    } finally {
      setPending(false)
    }
  }

  return (
    <form className="eval-ui-start" aria-label="Start a cycle" onSubmit={(event) => void submit(event)}>
      <SegmentedControl
        aria-label="Cycle input"
        variant="radio"
        value={entry}
        onChange={(next) => {
          setEntry(next)
          setError(null)
        }}
        options={ENTRIES}
      />
      <label className="eval-ui-label" htmlFor={inputId}>
        {entry === 'objective' ? 'Objective' : 'E2E execution id'}
      </label>
      <Input
        id={inputId}
        ref={inputRef}
        value={value}
        onChange={setValue}
        className={entry === 'execution' ? 'eval-ui-mono-input' : undefined}
        placeholder={entry === 'objective' ? 'Reduce repeated failing calls in long tasks' : 'exec_…'}
        aria-describedby={hintId}
        autoComplete="off"
        spellCheck={entry === 'objective'}
      />
      <p id={hintId} className="eval-ui-hint">
        {entry === 'objective'
          ? 'The loop picks the scenario and freezes the criterion before any run.'
          : 'Analyzes a finished execution, passed or failed, in the same history.'}
      </p>
      <Button type="submit" variant="primary" disabled={!text || pending}>
        {pending ? 'Starting…' : entry === 'objective' ? 'Start cycle' : 'Evaluate execution'}
      </Button>
      {error ? (
        <p className="eval-ui-form-error" role="alert">
          {error}
        </p>
      ) : null}
    </form>
  )
}
