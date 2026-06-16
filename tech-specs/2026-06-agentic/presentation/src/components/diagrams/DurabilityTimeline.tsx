import { PlayerControls } from '@/components/PlayerControls'
import { FnChip } from '@/components/schematic/FnChip'
import { useStepper } from '@/hooks/useStepper'
import { cn } from '@/lib/utils'

type StageTone = 'ink' | 'alert' | 'warn' | 'accent'

interface Stage {
  id: string
  label: string
  sub: string
  tone: StageTone
  title: string
  desc: string
  record: { status: string; step: string; calls: string; note: string }
}

const STAGES: Stage[] = [
  {
    id: 'send',
    label: 'harness::send',
    sub: 'turn t_001 seeded',
    tone: 'ink',
    title: 'a turn becomes a record, not a process',
    desc: 'send persists the message, seeds the turn record with an atomic check-and-set, enqueues step 1, and returns. from here on, the turn lives as durable state plus queue entries — never as a thread someone has to keep alive.',
    record: { status: 'running', step: '1 (enqueued)', calls: '{}', note: 'options frozen on the record for the whole turn' },
  },
  {
    id: 'gen',
    label: 'step 1 · generate',
    sub: 'e_t001_1_assistant',
    tone: 'ink',
    title: 'streaming into a deterministic entry',
    desc: 'the generate step appends the assistant entry under a deterministic id derived from turn + step, then streams deltas into it. the id is the durability trick: any replay of this step writes into the same entry.',
    record: { status: 'running', step: '1', calls: '{}', note: 'stream_request_id recorded — a stop can abort mid-flight' },
  },
  {
    id: 'crash',
    label: 'worker crashes',
    sub: 'mid-stream',
    tone: 'alert',
    title: 'the process dies. the turn does not.',
    desc: 'the queue delivery times out and redelivers; the turn record, the entries, and the per-call checkpoints are all still there. there is no in-memory loop to lose — that is the whole point.',
    record: { status: 'running', step: '1 (redelivering)', calls: '{}', note: 'partial assistant text already persisted + already rendered live' },
  },
  {
    id: 'resume',
    label: 'step 1 · redelivered',
    sub: 'same entry — no duplicate',
    tone: 'accent',
    title: 'resume, not restart',
    desc: 'the redelivered step finds the deterministic assistant entry already exists and streams into it instead of appending a second one. a crash never yields two answers — consumers just see the stream continue.',
    record: { status: 'running', step: '1', calls: '{}', note: 'stale-step guard drops anything older than the record\u2019s step' },
  },
  {
    id: 'dispatch',
    label: 'step 2 · dispatch',
    sub: 'checkpoint per call',
    tone: 'ink',
    title: 'side effects are at-most-once',
    desc: 'each function call checkpoints dispatched before it runs and done after its result lands. a call caught mid-flight by a crash is never silently re-invoked — the model gets an explicit "result unknown" and decides whether to retry.',
    record: { status: 'awaiting_functions', step: '2', calls: '{ call_1: "done" }', note: 'function_result ids derive from the call id — replays are no-ops' },
  },
  {
    id: 'hold',
    label: 'turn parks',
    sub: 'held — hours pass',
    tone: 'warn',
    title: 'durable execution: waiting costs nothing',
    desc: 'a held call (an approval, a long-running child) parks the turn: the step ends, no queue entry stays open, nothing blocks. the turn sleeps as state — for minutes or for a week — while the rest of the system runs at full speed.',
    record: { status: 'awaiting_functions', step: '2 (parked)', calls: '{ call_2: "pending" }', note: 'pending timeout sweeps guarantee nothing parks forever' },
  },
  {
    id: 'resolve',
    label: 'function::resolve',
    sub: 'decision lands',
    tone: 'accent',
    title: 'one call wakes the turn',
    desc: 'whoever owns the decision — a human, a finished child, a timeout sweep — resolves the pending call. the harness appends the result under its deterministic id and re-enqueues the turn exactly where it parked.',
    record: { status: 'running', step: '3 (enqueued)', calls: '{ call_2: "done" }', note: 'duplicate resolves hit the same entry id — idempotent' },
  },
  {
    id: 'done',
    label: 'completed',
    sub: 'result + event',
    tone: 'ink',
    title: 'the turn ends with a typed result',
    desc: 'the turn record carries the output-contract result, the session flips to done, and turn_completed fires for everything that reacts to outcomes. total process restarts survived: one. messages lost: zero.',
    record: { status: 'completed', step: '3 (final)', calls: 'all done', note: 'result validated against the contract\u2019s schema when supplied' },
  },
]

const toneClasses: Record<StageTone, { box: string; active: string; label: string }> = {
  ink: { box: 'border-rule', active: 'border-ink bg-panel', label: 'text-ink' },
  alert: { box: 'border-alert/50', active: 'border-alert bg-panel', label: 'text-alert' },
  warn: { box: 'border-warn/50', active: 'border-warn bg-panel', label: 'text-warn' },
  accent: { box: 'border-rule', active: 'border-accent bg-panel', label: 'text-accent' },
}

export function DurabilityTimeline({ className }: { className?: string }) {
  const stepper = useStepper(STAGES.length, 3000)
  const active = STAGES[stepper.step]

  return (
    <div className={cn('border border-rule bg-bg', className)}>
      <div className="flex items-center justify-between bg-panel px-3.5 py-2 border-b border-rule">
        <span className="font-mono text-[11px] font-medium uppercase tracking-[0.06em] text-ink-faint">
          one turn vs. a crash and a long wait
        </span>
        <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost tabular-nums">
          turn t_001
        </span>
      </div>

      {/* the track */}
      <div className="overflow-x-auto">
        <div className="flex items-stretch gap-0 px-4 py-6 min-w-[860px]">
          {STAGES.map((stage, i) => {
            const reached = i <= stepper.step
            const isActive = i === stepper.step
            const tone = toneClasses[stage.tone]
            return (
              <div key={stage.id} className="flex items-center flex-1 min-w-0">
                <button
                  type="button"
                  onClick={() => stepper.goTo(i)}
                  className={cn(
                    'border bg-bg px-2.5 py-2 w-full min-w-0 text-left transition-all duration-200 cursor-pointer',
                    reached ? tone.box : 'border-rule-2 opacity-45',
                    isActive && tone.active,
                    isActive && stage.tone === 'ink' && 'border-accent',
                  )}
                >
                  <div
                    className={cn(
                      'font-mono text-[10.5px] font-semibold leading-[1.35] truncate',
                      reached ? tone.label : 'text-ink-ghost',
                      isActive && stage.tone === 'ink' && 'text-ink',
                    )}
                  >
                    {stage.tone === 'alert' ? '⚡ ' : stage.tone === 'warn' ? '⏸ ' : ''}
                    {stage.label}
                  </div>
                  <div className="mt-0.5 font-mono text-[9px] tracking-[0.03em] text-ink-ghost truncate">
                    {stage.sub}
                  </div>
                </button>
                {i < STAGES.length - 1 ? (
                  <span
                    aria-hidden
                    className={cn(
                      'h-px w-4 shrink-0',
                      i < stepper.step ? 'bg-ink-faint' : 'bg-rule',
                      stage.id === 'hold' && 'w-6 border-t border-dashed border-rule bg-transparent',
                    )}
                  />
                ) : null}
              </div>
            )
          })}
        </div>
      </div>

      {/* the turn record — durable state evolving */}
      <div className="grid grid-cols-1 @3xl:grid-cols-[minmax(0,7fr)_minmax(0,5fr)] border-t border-rule">
        <div className="px-4 py-3.5 min-h-[108px]">
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
            <span className="font-mono text-[11px] text-ink-ghost tabular-nums">
              {String(stepper.step + 1).padStart(2, '0')}
            </span>
            <span className="font-mono text-[14px] font-semibold lowercase text-ink">
              {active.title}
            </span>
          </div>
          <p className="mt-1.5 font-mono text-[12.5px] leading-[1.65] text-ink-faint lowercase max-w-[78ch]">
            {active.desc}
          </p>
        </div>
        <div className="border-t @3xl:border-t-0 @3xl:border-l border-rule-2 bg-paper-2 px-4 py-3.5">
          <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-ink-faint mb-2">
            the turn record — state, not a process
          </div>
          <dl className="font-mono text-[11.5px] leading-[1.8]">
            <div className="flex gap-x-2">
              <dt className="text-ink-ghost w-14 shrink-0">status</dt>
              <dd className="text-ink">{active.record.status}</dd>
            </div>
            <div className="flex gap-x-2">
              <dt className="text-ink-ghost w-14 shrink-0">step</dt>
              <dd className="text-ink tabular-nums">{active.record.step}</dd>
            </div>
            <div className="flex gap-x-2">
              <dt className="text-ink-ghost w-14 shrink-0">calls</dt>
              <dd className="text-ink">{active.record.calls}</dd>
            </div>
          </dl>
          <div className="mt-2">
            <FnChip tone="faint" className="whitespace-normal">{active.record.note}</FnChip>
          </div>
        </div>
      </div>

      <PlayerControls stepper={stepper} total={STAGES.length} />
    </div>
  )
}
