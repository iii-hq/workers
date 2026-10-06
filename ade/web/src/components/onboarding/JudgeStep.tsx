import { useState } from 'react'
import { Button } from '@/components/ui/Button'
import {
  JUDGE_OPTIONS,
  JUDGE_USES,
  type JudgeOption,
} from '@/lib/onboarding/catalog'
import { judgePlan, type KeyInput } from '@/lib/onboarding/plan'
import { cn } from '@/lib/utils'
import {
  ActivityLog,
  defaultKeyInput,
  KeyField,
  keyInputReady,
  PlanPreview,
  Rows,
  Section,
  StatusChip,
  StepHeader,
  StepLayout,
} from './parts'
import type { OnboardingController } from './use-onboarding'

export function JudgeStep({
  onboarding,
  onBack,
  onNext,
}: {
  onboarding: OnboardingController
  onBack: () => void
  /** `judge` is the option set up, `null` when skipped. */
  onNext: (judge: JudgeOption | null) => void
}) {
  const { snapshot, activity, running, run } = onboarding
  const installedOption = JUDGE_OPTIONS.find((option) =>
    snapshot.installed.has(option.worker),
  )
  const [changing, setChanging] = useState(false)
  const log = activity.filter((entry) => entry.group === 'judge')
  // Right after a failed attempt the workers are up but the answer was no:
  // keep the choices (and the key field) on screen to fix it.
  const lastFailed = log[log.length - 1]?.status === 'failed'
  const alreadySetUp =
    onboarding.judgeInstalled &&
    installedOption !== undefined &&
    !changing &&
    !lastFailed
  const [selected, setSelected] = useState<JudgeOption>(
    installedOption ?? JUDGE_OPTIONS[0],
  )
  const [keys, setKeys] = useState<ReadonlyMap<string, KeyInput>>(new Map())
  const [done, setDone] = useState(false)

  const detection =
    snapshot.detections?.find((entry) => entry.name === selected.envVar) ?? null
  const key = selected.envVar
    ? (keys.get(selected.id) ?? defaultKeyInput(detection))
    : undefined
  const plan = judgePlan(selected, key, snapshot.installed)
  const busy = running !== null
  const incomplete = selected.envVar !== undefined && !keyInputReady(key)

  const setUp = async () => {
    if (await run('judge', plan)) setDone(true)
  }

  return (
    <StepLayout
      footer={
        <>
          <Button variant="ghost" onClick={onBack} disabled={busy}>
            Back
          </Button>
          <span className="flex items-center gap-2">
            {done || alreadySetUp ? (
              <Button
                onClick={() => onNext(installedOption ?? selected)}
                disabled={busy}
              >
                Continue
              </Button>
            ) : (
              <>
                <Button
                  variant="ghost"
                  onClick={() => onNext(null)}
                  disabled={busy}
                >
                  Skip
                </Button>
                <Button
                  onClick={() => void setUp()}
                  disabled={busy || incomplete}
                >
                  {running === 'judge' ? 'Setting up…' : 'Set up Judge'}
                </Button>
              </>
            )}
          </span>
        </>
      }
    >
      <StepHeader
        eyebrow="Step 2 of 2 · Optional"
        title="Let Judge make the small decisions"
        lead="Agents make many tiny choices along the way. Judge answers them with a model trained only for typed decisions, so your main model spends its tokens on the work itself."
      />

      <Section title="Where the harness asks Judge">
        <Rows>
          {JUDGE_USES.map((use) => (
            <div key={use.where} className="flex flex-col gap-0.5 px-3 py-2.5">
              <span className="font-sans text-[14px] font-medium text-ink">
                {use.where}
              </span>
              <span className="text-pretty font-sans text-[13px] leading-relaxed text-ink">
                {use.what}
              </span>
            </div>
          ))}
        </Rows>
        <p className="font-sans text-[13px] leading-relaxed text-ink">
          Without Judge, function search falls back to keyword matching and
          broken tool calls go back to the main model.
        </p>
      </Section>

      {alreadySetUp ? (
        <div className="flex items-center justify-between gap-3 rounded-md bg-ok-muted px-3 py-2">
          <p className="font-sans text-[14px] text-ink">
            Judge is already running with {installedOption?.title}.
          </p>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setChanging(true)}
            disabled={busy}
          >
            Change
          </Button>
        </div>
      ) : (
        <Section title="Who answers">
          <div
            role="radiogroup"
            aria-label="Judge"
            className="flex flex-col gap-2"
          >
            {JUDGE_OPTIONS.map((option) => {
              const checked = option.id === selected.id
              return (
                <div
                  key={option.id}
                  className={cn(
                    'flex flex-col rounded-md bg-surface',
                    checked && 'bg-surface-selected',
                  )}
                >
                  <label className="flex cursor-pointer items-start gap-3 px-3 py-3">
                    <input
                      type="radio"
                      name="judge-option"
                      checked={checked}
                      disabled={busy || done}
                      onChange={() => setSelected(option)}
                      className="mt-1 size-4 shrink-0 accent-[var(--color-accent)]"
                    />
                    <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                      <span className="flex flex-wrap items-center gap-2">
                        <span className="font-sans text-[14px] font-medium text-ink">
                          {option.title}
                        </span>
                        {option.recommended ? (
                          <StatusChip tone="neutral">Recommended</StatusChip>
                        ) : null}
                      </span>
                      <span className="text-pretty font-sans text-[13px] leading-relaxed text-ink">
                        {option.summary}
                      </span>
                      <span className="font-mono text-[12px] text-ink">
                        {option.runs}
                      </span>
                    </span>
                  </label>
                  {checked && option.envVar && !done ? (
                    <KeyField
                      envVar={option.envVar}
                      detection={detection}
                      value={key}
                      onChange={(next) =>
                        setKeys((current) =>
                          new Map(current).set(option.id, next),
                        )
                      }
                      keysUrl={option.keysUrl}
                    />
                  ) : null}
                </div>
              )
            })}
          </div>
        </Section>
      )}

      {!done && !alreadySetUp && running !== 'judge' ? (
        <PlanPreview steps={plan} />
      ) : null}
      <ActivityLog
        entries={log}
        title={running === 'judge' ? 'Setting up Judge' : 'What setup did'}
      />
    </StepLayout>
  )
}
