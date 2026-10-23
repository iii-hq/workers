/* What the New worker dialog and the coder::scaffold-worker chat card show
   about a new worker: its steps (created, installed, running) and, once it
   runs, the functions it registered. */

import { IconButton, StatusDot } from '@iii-dev/console-ui'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { Check, Copy, MessageSquarePlus, Minus, X } from 'lucide-react'
import { type ReactNode, useId } from 'react'
import type { FunctionEntry, StepState } from './new-worker'

const STEP_TONE = { live: 'ok', active: 'accent', pending: 'ink' } as const

export function StepMark({ state }: { state: StepState }) {
  if (state === 'done') return <Check aria-hidden />
  if (state === 'failed') return <X aria-hidden />
  if (state === 'skipped') return <Minus aria-hidden />
  return <StatusDot tone={STEP_TONE[state]} pulse={state === 'active'} />
}

export interface StepRow {
  key: string
  label: ReactNode
  state: StepState
  /** Beside the label: a clock, a path. */
  detail?: ReactNode
  /** Under a failed step: why it failed. */
  failure?: ReactNode
  /** Under the label, in any state: what the step's outcome means. */
  note?: ReactNode
}

/** Done, in progress (with its clock), waiting or failed. */
export function StepList({ rows }: { rows: StepRow[] }) {
  return (
    <ol className="shui-new-worker-steps" aria-live="polite">
      {rows.map((row) => (
        <li key={row.key} className="shui-new-worker-step" data-state={row.state}>
          <span className="shui-new-worker-step-mark">
            <StepMark state={row.state} />
          </span>
          <span className="shui-new-worker-step-label">{row.label}</span>
          {row.detail ? <span className="shui-new-worker-step-detail">{row.detail}</span> : null}
          {row.state === 'failed' && row.failure ? (
            <div className="shui-new-worker-step-failure">{row.failure}</div>
          ) : null}
          {row.note ? <p className="shui-new-worker-step-note">{row.note}</p> : null}
        </li>
      ))}
    </ol>
  )
}

/** Text with its `backticked` spans as code: compose quotes commands that way. */
export function WithCode({ text }: { text: string }) {
  const parts = text.split('`')
  // An odd count of backticks leaves the last one unmatched: it stays text.
  const unmatched = parts.length % 2 === 0
  return (
    <>
      {parts.map((part, index) => {
        if (unmatched && index === parts.length - 1) return `\`${part}`
        return index % 2 === 1 ? (
          <code key={index} className="shui-new-worker-code">
            {part}
          </code>
        ) : (
          part
        )
      })}
    </>
  )
}

/** Why a start failed: the error, then the container's last log lines. */
export function StartFailure({ error, logs }: { error: string; logs: string[] }) {
  return (
    <>
      {error ? (
        <p className="shui-new-worker-note alert">
          <WithCode text={error} />
        </p>
      ) : null}
      {logs.length > 0 ? <pre className="shui-new-worker-logs">{logs.join('\n')}</pre> : null}
    </>
  )
}

/** What the running worker registered: copy an id, or try it in a chat draft. */
export function WorkerFunctions({ functions, onTry }: { functions: FunctionEntry[]; onTry?: (id: string) => void }) {
  const headingId = useId()
  return (
    <section className="shui-new-worker-functions" aria-labelledby={headingId}>
      <h3 id={headingId} className="shui-text-dialog-label">
        {functions.length === 1 ? 'Its function' : `Its ${functions.length} functions`}
      </h3>
      <ul className="shui-new-worker-function-list">
        {functions.map((fn) => (
          <FunctionRow key={fn.function_id} fn={fn} onTry={onTry} />
        ))}
      </ul>
    </section>
  )
}

function FunctionRow({ fn, onTry }: { fn: FunctionEntry; onTry?: (id: string) => void }) {
  const { state, copy } = useCopyFlash(fn.function_id)
  return (
    <li className="shui-new-worker-function">
      <span className="shui-new-worker-function-text">
        <span className="shui-new-worker-function-id">{fn.function_id}</span>
        {fn.description ? <span className="shui-new-worker-function-description">{fn.description}</span> : null}
      </span>
      <IconButton label={state === 'copied' ? 'Copied' : `Copy ${fn.function_id}`} variant="ghost" onClick={copy}>
        {state === 'copied' ? <Check aria-hidden /> : <Copy aria-hidden />}
      </IconButton>
      {onTry ? (
        <IconButton label={`Try ${fn.function_id} in a chat`} variant="ghost" onClick={() => onTry(fn.function_id)}>
          <MessageSquarePlus aria-hidden />
        </IconButton>
      ) : null}
    </li>
  )
}
