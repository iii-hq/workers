import {
  Badge,
  Button,
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
  Input,
  List,
  ListItem,
  SearchField,
  SegmentedControl,
  Skeleton,
  StatusPanel,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useDebounce } from '@iii-dev/console-ui/hooks'
import { type FormEvent, useEffect, useId, useState } from 'react'
import type { EvalApi } from '../api'
import { plural, readiness, shortId, span } from '../model'
import type { CycleInput, CycleRecord, ExecutionOption, LoopConfig } from '../types'
import { ago, ToneIcon, usd } from './bits'
import type { Nav } from './index'

type From = 'execution' | 'objective'

const FROM = [
  { value: 'execution' as const, label: 'A finished E2E execution', icon: false as const },
  { value: 'objective' as const, label: 'An objective', icon: false as const },
]

const WILL = [
  'This cycle will only map coverage: the loop is not configured',
  'This cycle will measure and stop: Improve is off',
  'This cycle will measure and write a change, but cannot prove it: Prove is off',
  'This cycle will measure, improve and prove, then stop at the decision',
  'This cycle will measure, improve, prove and, if accepted, ship',
]

/** The one way to start a cycle: pick an execution that lost points, or name an objective. */
export function NewCycle({
  api,
  open,
  onOpenChange,
  cycles,
  config,
  nav,
  onStarted,
}: {
  api: EvalApi
  open: boolean
  onOpenChange: (open: boolean) => void
  cycles: CycleRecord[]
  config: LoopConfig | null
  nav: Nav
  onStarted: () => void
}) {
  const [from, setFrom] = useState<From>('execution')
  const [query, setQuery] = useState('')
  const [objective, setObjective] = useState('')
  const [picked, setPicked] = useState<string | null>(null)
  const [options, setOptions] = useState<ExecutionOption[] | null>(null)
  const [readError, setReadError] = useState<string | null>(null)
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const settled = useDebounce(query.trim(), 250)
  const objectiveId = useId()
  const level = readiness(config)

  useEffect(() => {
    if (!open || from !== 'execution') return
    let live = true
    setReadError(null)
    api.executions(settled).then(
      (next) => live && setOptions(next),
      (failure) => live && setReadError(errorMessage(failure)),
    )
    return () => {
      live = false
    }
  }, [api, open, from, settled])

  // A cycle started from it before the claim index knew: the cycle list says so too.
  const cycleOf = (option: ExecutionOption) =>
    option.cycle_id ??
    cycles.find((cycle) => 'execution_id' in cycle.input && cycle.input.execution_id === option.execution_id)?.cycle_id
  const pasted = settled && !/\s/.test(settled) && !options?.some((option) => option.execution_id === settled)
  const input: CycleInput | null =
    from === 'objective'
      ? objective.trim()
        ? { objective: objective.trim() }
        : null
      : picked
        ? { execution_id: picked }
        : null

  async function submit(event: FormEvent) {
    event.preventDefault()
    if (!input || pending) return
    setPending(true)
    setError(null)
    try {
      const started = await api.start(input)
      onStarted()
      nav.announce('Cycle started')
      nav.cycle(started.cycle_id)
      onOpenChange(false)
      setPicked(null)
      setObjective('')
      setQuery('')
    } catch (failure) {
      setError(errorMessage(failure))
    } finally {
      setPending(false)
    }
  }

  const budget = config?.budget

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="eval-ui-dialog">
        <form className="eval-ui-dialog-form" onSubmit={(event) => void submit(event)}>
          <DialogTitle>New cycle</DialogTitle>
          <DialogDescription>
            A cycle measures today's Harness on the E2E, tries one change, and keeps it only if the runs prove it.
          </DialogDescription>
          <SegmentedControl<From>
            className="eval-ui-start"
            aria-label="Start from"
            variant="radio"
            value={from}
            onChange={(next) => {
              setFrom(next)
              setError(null)
            }}
            options={FROM}
          />

          {from === 'execution' ? (
            <div className="eval-ui-field">
              <p className="eval-ui-strong">Which execution should improve?</p>
              <p className="eval-ui-faint">
                Pick one that lost points. The loop restores the criterion it lost the most, without losing any other.
              </p>
              <SearchField
                aria-label="Search finished executions"
                placeholder="Search finished executions or paste an id"
                value={query}
                onChange={setQuery}
                className="eval-ui-mono-input"
              />
              {readError ? (
                <StatusPanel
                  variant="alert"
                  headline="Couldn't list the executions"
                  detail={`${readError}. An execution id pasted above still starts a cycle.`}
                />
              ) : !options ? (
                <div className="eval-ui-skeleton" role="status" aria-busy="true" aria-label="Loading executions">
                  <Skeleton className="eval-ui-sk-row" />
                  <Skeleton className="eval-ui-sk-row" />
                </div>
              ) : null}
              {options || pasted ? (
                <List className="eval-ui-options" aria-label="Recent finished executions">
                  {pasted ? (
                    <ListItem
                      selected={picked === settled}
                      aria-pressed={picked === settled}
                      onClick={() => setPicked(settled)}
                      label={
                        <>
                          Use <span className="eval-ui-mono">{settled}</span>
                        </>
                      }
                      description="Not in the recent list; the loop reads it when the cycle starts."
                    />
                  ) : null}
                  {options?.map((option) => (
                    <ExecutionRow
                      key={option.execution_id}
                      option={option}
                      cycleId={cycleOf(option)}
                      picked={picked === option.execution_id}
                      onPick={() => setPicked(option.execution_id)}
                      onOpenCycle={(cycleId) => {
                        onOpenChange(false)
                        nav.cycle(cycleId)
                      }}
                    />
                  ))}
                  {options?.length === 0 && !pasted ? (
                    <p className="eval-ui-faint">No finished execution{settled ? ' matches' : ' yet'}.</p>
                  ) : null}
                </List>
              ) : null}
            </div>
          ) : (
            <div className="eval-ui-field">
              <label className="eval-ui-strong" htmlFor={objectiveId}>
                What should improve?
              </label>
              <Input
                id={objectiveId}
                value={objective}
                onChange={setObjective}
                placeholder="Recover the charter facts after a long context"
                autoComplete="off"
              />
              <p className="eval-ui-faint">
                The loop measures it on the tests that cover it and freezes the criterion before any run.
              </p>
            </div>
          )}

          <StatusPanel
            variant="info"
            icon={<ToneIcon tone="neutral" />}
            headline={WILL[level]}
            detail={
              budget
                ? `It may spend up to ${usd(budget.max_spend_usd)}, ${span(budget.max_duration_ms)} and ${plural(budget.max_calls, 'call')}; if no E2E test covers the goal, it stops before spending anything.`
                : 'Nothing is dispatched until Loop settings are saved.'
            }
          />
          {error ? (
            <p className="eval-ui-form-error" role="alert">
              {error}
            </p>
          ) : null}
          <p className="eval-ui-faint">
            Starting from an objective measures it on the tests that cover it. To change the Harness, an objective needs
            a criterion the E2E scores itself; token, cost and call counts come from the Harness and cannot qualify a
            change.
          </p>
          <div className="eval-ui-actions">
            <DialogClose asChild>
              <Button type="button" variant="ghost">
                Cancel
              </Button>
            </DialogClose>
            <Button type="submit" variant="primary" disabled={!input || pending}>
              {pending ? 'Starting…' : 'Start cycle'}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}

function ExecutionRow({
  option,
  cycleId,
  picked,
  onPick,
  onOpenCycle,
}: {
  option: ExecutionOption
  cycleId?: string
  picked: boolean
  onPick: () => void
  onOpenCycle: (cycleId: string) => void
}) {
  const lost = option.scenarios.filter((scenario) => scenario.lost)
  const scores = option.scenarios.flatMap((scenario) =>
    scenario.mean_score === undefined ? [] : [scenario.mean_score],
  )
  const score = scores.length ? Math.round(scores.reduce((sum, value) => sum + value, 0) / scores.length) : null
  const nothing = !cycleId && option.scenarios.length > 0 && lost.length === 0
  const description = cycleId
    ? 'Already in a cycle: open it'
    : lost.length
      ? // `criterion X of S was awarded 50% on average` reads `Lost X (50% on average)`.
        `Lost ${lost.map((scenario) => scenario.lost?.replace(/^criterion (\S+) of \S+ was awarded (.+)$/, '$1 ($2)')).join('; ')}`
      : option.scenarios.length
        ? 'Nothing lost: there is nothing for a cycle to improve'
        : 'Its reports could not be read; the cycle reads them when it starts'
  return (
    <ListItem
      selected={picked}
      aria-pressed={cycleId ? undefined : picked}
      disabled={nothing}
      onClick={() => (cycleId ? onOpenCycle(cycleId) : onPick())}
      label={
        <span className="eval-ui-clip">
          <span className="eval-ui-mono" title={option.execution_id}>
            {shortId(option.execution_id)}
          </span>
          {option.scenarios.length ? (
            <span className="eval-ui-faint">
              {' '}
              · {option.scenarios.map((scenario) => scenario.scenario_id).join(', ')}
            </span>
          ) : null}
        </span>
      }
      description={<span className="eval-ui-clip">{description}</span>}
      trailing={
        <span className="eval-ui-row-end">
          {score === null ? <span /> : <Badge className="eval-ui-num">{score}/100</Badge>}
          {option.started_at ? <span className="eval-ui-num eval-ui-ghost">{ago(option.started_at)}</span> : null}
        </span>
      }
    />
  )
}
