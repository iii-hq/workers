import { ChevronDown } from 'lucide-react'
import { Fragment, type ReactNode, useId, useState } from 'react'
import { TimelineActivityTrail } from '@/components/chat/TimelineActivityTrail'
import { cn } from '@/lib/utils'
import type { FunctionCallMessage } from '@/types/chat'
import {
  type FunctionCallStatus,
  FunctionCallStatusIcon,
  isErrorOutput,
} from './FunctionCallCard'
import { OBSERVE_FUNCTION_ID, observeRowTitle } from './observe-title'

/** A run collapses from two consecutive observes up. */
export const OBSERVE_RUN_MIN = 2

export function isObserveCall(message: { functionId?: string }): boolean {
  return message.functionId === OBSERVE_FUNCTION_ID
}

/** What one observe watches, in its row's words: the function id, else the
 * intent its description names, else nothing yet. */
function observedName(call: FunctionCallMessage): string | null {
  const title = observeRowTitle(call.input, call.description?.trim())
  if (!title) return null
  return 'functionId' in title
    ? title.functionId
    : 'intent' in title
      ? title.intent
      : null
}

export interface ObserveRunSummary {
  /** Distinct observed names in first-seen order, with how many observes
   * named each. */
  entries: Array<{ name: string; count: number }>
  /** Observes whose function is not known yet (arguments still loading). */
  unnamed: number
  status: FunctionCallStatus
  failed: number
}

export function summarizeObserveRun(
  calls: readonly FunctionCallMessage[],
): ObserveRunSummary {
  const entries: ObserveRunSummary['entries'] = []
  let unnamed = 0
  let failed = 0
  let pending = false
  let running = false
  for (const call of calls) {
    const name = observedName(call)
    if (name === null) unnamed++
    else {
      const entry = entries.find((e) => e.name === name)
      if (entry) entry.count++
      else entries.push({ name, count: 1 })
    }
    if (call.pendingApproval) pending = true
    else if (call.running) running = true
    else if (!call.unloaded && isErrorOutput(call.output)) failed++
  }
  return {
    entries,
    unnamed,
    failed,
    status: pending
      ? 'pending'
      : running
        ? 'running'
        : failed
          ? 'error'
          : 'done',
  }
}

/** "Observing 6 functions", or the one function a run observes repeatedly. */
export function observeRunTitle(summary: ObserveRunSummary): string {
  const { entries, unnamed } = summary
  if (entries.length === 1 && unnamed === 0) {
    return `Observing ${entries[0].name}`
  }
  const count = entries.length + unnamed
  return `Observing ${count} ${count === 1 ? 'function' : 'functions'}`
}

/** Every observed function, repeats as `×N`, so nothing the individual
 * rows said is lost while they are folded away. */
export function observeRunNames(summary: ObserveRunSummary): string[] {
  const { entries, unnamed } = summary
  if (entries.length === 1 && unnamed === 0) {
    // The title already names the function: say how many observers.
    return [`${entries[0].count} observers`]
  }
  return [
    ...entries.map((e) => (e.count > 1 ? `${e.name} ×${e.count}` : e.name)),
    ...(unnamed > 0 ? [`${unnamed} loading`] : []),
  ]
}

export function observeRunSubtext(summary: ObserveRunSummary): string {
  const list = observeRunNames(summary).join(', ')
  return summary.failed > 0 ? `${list} · ${summary.failed} failed` : list
}

interface ObserveRunCardProps {
  calls: readonly FunctionCallMessage[]
  /** The individual observe rows, shown when the run opens. */
  children: ReactNode
  /** Open on mount (showcase surfaces, `defaultOpenCalls`). */
  defaultOpen?: boolean
  /** A member needs the reader (an approval, a landing): open and stay open. */
  forceOpen?: boolean
  /** Called when the reader opens the run, to fetch placeholder members. */
  onOpen?: () => void
}

/**
 * Consecutive `harness::observe` calls as one row: "Observing 6 functions"
 * over a line that names every observed function. The chevron opens the
 * individual rows, each still its own card with its own details.
 */
export function ObserveRunCard({
  calls,
  children,
  defaultOpen,
  forceOpen,
  onOpen,
}: ObserveRunCardProps) {
  const [open, setOpen] = useState(!!defaultOpen || !!forceOpen)
  // Render-phase latch, like the group's landing reveal: the member's DOM
  // row must exist in the same render the request resolves.
  if (forceOpen && !open) setOpen(true)
  const listId = useId()
  const summary = summarizeObserveRun(calls)
  const title = observeRunTitle(summary)
  const subtext = observeRunSubtext(summary)
  const names = observeRunNames(summary)
  const toggle = () => {
    if (!open) onOpen?.()
    setOpen((value) => !value)
  }
  return (
    <section
      className="function-call-surface chat-tool-row chat-observe-run"
      data-message-role="observe-run"
      data-observe-run-count={calls.length}
      data-expanded={open}
      data-function-status={summary.status}
    >
      <div className="chat-tool-row__head">
        <button
          type="button"
          onClick={toggle}
          aria-expanded={open}
          aria-controls={listId}
          aria-label={`${title}: ${subtext} · ${open ? 'hide' : 'show'} each observe`}
          className="chat-tool-row__main"
          data-message-action="toggle-observe-run"
        >
          <TimelineActivityTrail kind="observer-registration" />
          <div className="chat-tool-row__text">
            <div className="chat-tool-row__title">
              <span
                className={cn(
                  summary.status === 'running' && 'function-call-shimmer',
                )}
              >
                {title}
              </span>
            </div>
            <div
              className="chat-tool-row__subtitle chat-tool-row__subtitle--wrap"
              data-visible="true"
            >
              <span data-observe-run-list="">
                {names.map((name, index) => (
                  <Fragment key={name}>
                    {index > 0 ? ', ' : null}
                    <span className="chat-observe-run__name">{name}</span>
                  </Fragment>
                ))}
                {summary.failed > 0 ? ` · ${summary.failed} failed` : null}
              </span>
            </div>
          </div>
          <FunctionCallStatusIcon status={summary.status} />
        </button>
        <button
          type="button"
          aria-hidden="true"
          tabIndex={-1}
          onClick={toggle}
          className="chat-tool-row__caret"
        >
          <ChevronDown
            className="chat-tool-row__chevron size-4"
            data-open={open ? 'true' : undefined}
          />
        </button>
      </div>
      {open ? (
        <div
          id={listId}
          className="chat-tool-row__body chat-observe-run__list"
          data-observe-run-members=""
        >
          {children}
        </div>
      ) : null}
    </section>
  )
}
