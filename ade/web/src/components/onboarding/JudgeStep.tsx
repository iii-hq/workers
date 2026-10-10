import type { LucideIcon } from 'lucide-react'
import {
  Cloud,
  Cpu,
  LoaderCircle,
  MousePointerClick,
  SearchCode,
  Wrench,
} from 'lucide-react'
import type * as React from 'react'
import { useId, useState } from 'react'
import {
  JUDGE_OPTIONS,
  JUDGE_USES,
  type JudgeOption,
  type JudgeUseId,
} from '@/lib/onboarding/catalog'
import {
  type JudgeSelection,
  judgePlan,
  type KeyInput,
} from '@/lib/onboarding/plan'
import { cn } from '@/lib/utils'
import { Button, Checkbox } from './controls'
import {
  defaultKeyInput,
  EngineLog,
  KeyField,
  keyInputReady,
  Section,
  StatusChip,
  StepHeader,
  StepLayout,
} from './parts'
import type { OnboardingController } from './use-onboarding'

const USE_ICONS: Record<JudgeUseId, LucideIcon> = {
  search: SearchCode,
  repair: Wrench,
  browser: MousePointerClick,
}

/**
 * Judge, optional. Any number of strategies can answer; the hub sends a
 * request to its default — the one it is configured with when that is
 * still checked, otherwise the first checked — and a request may still
 * name another. Running strategies start checked and unchecking one
 * removes it, the same as a provider in the models step.
 *
 * The step reads top to bottom as: what Judge does (three uses, one line
 * each), then the one thing to do here — tick the strategies that should
 * answer — then the button that makes it so.
 */
export function JudgeStep({
  onboarding,
  onBack,
  onNext,
}: {
  onboarding: OnboardingController
  onBack: () => void
  /** The strategies answering, default first; empty when skipped. */
  onNext: (judges: JudgeOption[]) => void
}) {
  const { snapshot, activity, running, run } = onboarding
  const installed = (option: JudgeOption) =>
    onboarding.judgeInstalled && snapshot.installed.has(option.worker)
  const anyInstalled = JUDGE_OPTIONS.some(installed)
  const [drafts, setDrafts] = useState<ReadonlyMap<string, boolean>>(new Map())
  const [keys, setKeys] = useState<ReadonlyMap<string, KeyInput>>(new Map())
  const log = activity.filter((entry) => entry.group === 'judge')
  const busy = running !== null

  // Running strategies start checked; on a fresh setup, the recommended one.
  const checked = (option: JudgeOption) =>
    drafts.get(option.id) ??
    (anyInstalled ? installed(option) : option.recommended === true)
  const keyFor = (option: JudgeOption) => {
    if (!option.envVar || installed(option)) return undefined
    const detection =
      snapshot.detections?.find((entry) => entry.name === option.envVar) ?? null
    return keys.get(option.id) ?? defaultKeyInput(detection)
  }

  const chosen = JUDGE_OPTIONS.filter(checked)
  const current = snapshot.judgeProvider
  const primary =
    chosen.find((option) => option.id === current) ?? chosen[0] ?? null
  const ordered = primary
    ? [primary, ...chosen.filter((option) => option !== primary)]
    : []
  const selections: JudgeSelection[] = ordered.map((option) => ({
    option,
    key: keyFor(option),
  }))
  const removals = JUDGE_OPTIONS.filter(
    (option) => installed(option) && !checked(option),
  )
  const plan = judgePlan(selections, snapshot.installed, {
    removals,
    currentDefault: current,
  })
  const incomplete = selections.some(
    ({ option, key }) =>
      option.envVar !== undefined && !installed(option) && !keyInputReady(key),
  )

  const apply = async () => {
    if (await run('judge', plan)) {
      setDrafts(new Map())
      setKeys(new Map())
    }
  }

  return (
    <StepLayout
      footer={
        <>
          <Button variant="outline" onClick={onBack} disabled={busy}>
            Back
          </Button>
          <span className="flex items-center gap-2">
            {plan.length === 0 ? (
              <Button onClick={() => onNext(ordered)} disabled={busy}>
                {ordered.length > 0 ? 'Continue' : 'Skip'}
              </Button>
            ) : (
              <>
                {!anyInstalled ? (
                  <Button
                    variant="ghost"
                    onClick={() => onNext([])}
                    disabled={busy}
                  >
                    Skip
                  </Button>
                ) : null}
                <Button
                  onClick={() => void apply()}
                  disabled={busy || incomplete}
                >
                  {running === 'judge' ? (
                    <LoaderCircle
                      className="animate-spin motion-reduce:animate-none"
                      aria-hidden
                    />
                  ) : null}
                  {running === 'judge'
                    ? 'Setting up…'
                    : anyInstalled
                      ? 'Apply changes'
                      : 'Set up Judge'}
                </Button>
              </>
            )}
          </span>
        </>
      }
    >
      <StepHeader
        title="Add a judge"
        badge="Optional"
        lead="A small, fast model that takes the harness's little decisions off your main model's plate."
      />

      <section
        aria-label="What Judge does"
        className="rounded-lg border border-neutral-200 bg-neutral-50 dark:border-neutral-800 dark:bg-neutral-900"
      >
        <ul className="grid divide-y divide-neutral-200 @md:grid-cols-3 @md:divide-x @md:divide-y-0 dark:divide-neutral-800">
          {JUDGE_USES.map((use) => {
            const Icon = USE_ICONS[use.id]
            return (
              <li key={use.id} className="flex gap-2.5 px-3.5 py-3">
                <Icon
                  className="mt-0.5 size-4 shrink-0 text-ink"
                  strokeWidth={1.75}
                  aria-hidden
                />
                <span className="flex min-w-0 flex-col gap-px">
                  <span className="text-[13px] font-medium leading-5 text-ink">
                    {use.where}
                  </span>
                  <span className="text-pretty text-xs leading-4 text-neutral-600 dark:text-neutral-400">
                    {use.short}
                  </span>
                </span>
              </li>
            )
          })}
        </ul>
        <p className="border-t border-neutral-200 px-3.5 py-2 text-xs leading-4 text-neutral-500 dark:border-neutral-800 dark:text-neutral-400">
          Without Judge, function search falls back to keyword matching and a
          broken tool call goes back to the main model.
        </p>
      </section>

      <Section
        title="Choose who answers"
        hint={
          anyInstalled
            ? 'tick to add, untick to remove'
            : 'tick one or more; the first answers by default'
        }
      >
        <div className="flex flex-col gap-2">
          {JUDGE_OPTIONS.map((option) => (
            <JudgeRow
              key={option.id}
              option={option}
              checked={checked(option)}
              running={installed(option)}
              isDefault={primary === option && chosen.length > 1}
              showRecommended={!anyInstalled}
              disabled={busy}
              onToggle={(next) =>
                setDrafts((current) => new Map(current).set(option.id, next))
              }
              keyField={
                checked(option) && option.envVar && !installed(option) ? (
                  <KeyField
                    envVar={option.envVar}
                    detection={
                      snapshot.detections?.find(
                        (entry) => entry.name === option.envVar,
                      ) ?? null
                    }
                    value={keyFor(option)}
                    onChange={(next) =>
                      setKeys((current) =>
                        new Map(current).set(option.id, next),
                      )
                    }
                    keysUrl={option.keysUrl}
                    className="border-t border-neutral-200 px-3.5 py-3 dark:border-neutral-800"
                  />
                ) : null
              }
            />
          ))}
        </div>
      </Section>

      <EngineLog plan={plan} entries={log} running={running === 'judge'} />
    </StepLayout>
  )
}

/**
 * One strategy to tick: the box leads the row so the choice reads as a
 * choice, the title and its state follow, and where it runs sits under the
 * summary with a glyph for hosted or local.
 */
function JudgeRow({
  option,
  checked,
  running,
  isDefault,
  showRecommended,
  disabled,
  onToggle,
  keyField,
}: {
  option: JudgeOption
  checked: boolean
  running: boolean
  isDefault: boolean
  showRecommended: boolean
  disabled: boolean
  onToggle: (checked: boolean) => void
  keyField: React.ReactNode
}) {
  const id = useId()
  const RunsIcon = option.envVar ? Cloud : Cpu
  return (
    <div
      data-selected={checked || undefined}
      className={cn(
        'flex flex-col rounded-lg border transition-[background-color,border-color] duration-150 ease-[var(--motion-ease-standard)]',
        checked
          ? 'border-neutral-400 bg-neutral-100 dark:border-neutral-600 dark:bg-neutral-900'
          : 'border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-950',
        !disabled &&
          !checked &&
          'hover:bg-neutral-50 dark:hover:bg-neutral-900',
      )}
    >
      <div className="flex items-start gap-3 px-3.5 py-3">
        <Checkbox
          id={id}
          aria-label={`Answer with ${option.title}`}
          checked={checked}
          disabled={disabled}
          onChange={(event) => onToggle(event.currentTarget.checked)}
          className="h-5"
        />
        <label
          htmlFor={id}
          className={cn(
            'flex min-w-0 flex-1 flex-col gap-1',
            !disabled && 'cursor-pointer',
          )}
        >
          <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span className="font-sans text-[13px] font-medium leading-5 text-ink">
              {option.title}
            </span>
            {running ? (
              <StatusChip tone={checked ? 'ok' : 'warn'}>
                {checked ? 'Running' : 'Will be removed'}
              </StatusChip>
            ) : showRecommended && option.recommended ? (
              <StatusChip tone="neutral">Recommended</StatusChip>
            ) : null}
            {isDefault ? (
              <StatusChip tone="neutral">Answers by default</StatusChip>
            ) : null}
          </span>
          <span className="text-pretty font-sans text-[13px] leading-5 text-neutral-600 dark:text-neutral-400">
            {option.summary}
          </span>
          <span className="flex items-center gap-1.5 text-xs leading-4 text-neutral-500 dark:text-neutral-400">
            <RunsIcon
              className="size-4 shrink-0"
              strokeWidth={1.75}
              aria-hidden
            />
            {option.runs}
          </span>
        </label>
      </div>
      {keyField}
    </div>
  )
}
