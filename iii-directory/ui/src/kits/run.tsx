/**
 * Running a plan: the apply call, live step progress from
 * `directory::kits::on-change`, retry after a failure (the apply is
 * idempotent), and the done summary with its shortcuts.
 */

import { Button, type Host, StatusPanel } from '@iii-dev/console-ui'
import { Check, RotateCw } from 'lucide-react'
import { useCallback, useState } from 'react'
import { errorText, kitsApi, useKitsChange } from './api'
import { initialSteps } from './model'
import { StepList } from './parts'
import type { ApplyReport, ApplyStep, Plan } from './types'

export type RunPhase =
  | { kind: 'idle' }
  | { kind: 'running'; steps: ApplyStep[] }
  | { kind: 'failed'; steps: ApplyStep[]; error: string }
  | { kind: 'done'; report: ApplyReport; message: string }

export function useApplyRun(host: Host, plan: Plan, onReplanned: (plan: Plan, message: string) => void) {
  const [phase, setPhase] = useState<RunPhase>({ kind: 'idle' })
  useKitsChange(host, `run-${plan.plan_id}`, (change) => {
    if (change.op !== 'progress' || change.plan_id !== plan.plan_id || !change.steps) return
    const steps = change.steps
    setPhase((p) => (p.kind === 'running' || p.kind === 'failed' ? { ...p, steps } : p))
  })
  const run = useCallback(
    async (decisions: Record<string, unknown>, removeWorkers: string[] = []) => {
      setPhase({ kind: 'running', steps: initialSteps(plan) })
      try {
        const out = await kitsApi(host).apply(plan.plan_id, decisions, removeWorkers)
        if (out.status === 'applied' && out.report) {
          setPhase({ kind: 'done', report: out.report, message: out.message })
        } else if (out.plan) {
          setPhase({ kind: 'idle' })
          onReplanned(out.plan, out.message)
        } else {
          setPhase({ kind: 'failed', steps: initialSteps(plan), error: out.message })
        }
      } catch (error) {
        setPhase((p) => ({
          kind: 'failed',
          steps: p.kind === 'running' ? p.steps : initialSteps(plan),
          error: errorText(error),
        }))
      }
    },
    [host, plan, onReplanned],
  )
  return { phase, run, reset: () => setPhase({ kind: 'idle' }) }
}

function verbFor(plan: Plan): string {
  switch (plan.kind) {
    case 'install':
      return `Installing ${plan.kit} ${plan.to ?? ''}`
    case 'update':
      return `Updating ${plan.kit} ${plan.from ?? ''} → ${plan.to ?? ''}`
    case 'remove':
      return `Removing ${plan.kit}`
  }
}

/** The running / failed view: the three steps, then the error and a retry. */
export function RunProgress({
  plan,
  phase,
  onRetry,
  onBack,
}: {
  plan: Plan
  phase: Extract<RunPhase, { kind: 'running' | 'failed' }>
  onRetry: () => void
  onBack: () => void
}) {
  return (
    <section className="dir-ui-kit-run" aria-live="polite">
      <h3 className="dir-ui-kit-run-title">
        {phase.kind === 'running' ? `${verbFor(plan)}…` : `${verbFor(plan)} stopped`}
      </h3>
      <StepList steps={phase.steps} />
      {phase.kind === 'failed' ? (
        <div className="dir-ui-kit-run-error">
          <StatusPanel
            variant="alert"
            headline="The apply stopped before finishing."
            detail={phase.error}
            action={
              <Button variant="primary" size="sm" onClick={onRetry}>
                <RotateCw aria-hidden />
                Try again
              </Button>
            }
          />
          <p className="dir-ui-kit-fine">
            Trying again is safe: files already written are recognised and kits.lock is only written once everything
            else succeeded.
          </p>
          <Button variant="ghost" size="sm" onClick={onBack}>
            Back to the review
          </Button>
        </div>
      ) : null}
    </section>
  )
}

/** Done: what changed, and where to go next. */
export function RunDone({
  report,
  message,
  onOpenProfile,
  onOpenKit,
  onBack,
}: {
  report: ApplyReport
  message: string
  onOpenProfile?: (id: string) => void
  onOpenKit?: () => void
  onBack: () => void
}) {
  const rows: [string, string[]][] = [
    ['written', report.files_written],
    ['merged', report.files_merged],
    ['removed', report.files_removed],
    ['kept as they were', report.files_kept],
    ['workers added', report.workers_added],
    ['workers updated', report.workers_updated],
    ['workers removed', report.workers_removed],
  ]
  return (
    <section className="dir-ui-kit-run" aria-live="polite">
      <div className="dir-ui-kit-done-head">
        <span className="dir-ui-kit-done-icon">
          <Check aria-hidden />
        </span>
        <h3 className="dir-ui-kit-run-title">{message}</h3>
      </div>
      <StepList steps={report.steps} />
      <dl className="dir-ui-kit-done-list">
        {rows
          .filter(([, items]) => items.length > 0)
          .map(([label, items]) => (
            <div key={label} className="dir-ui-kit-done-row">
              <dt>
                {items.length} {label}
              </dt>
              <dd>
                {items.map((item) => (
                  <code key={item}>{item}</code>
                ))}
              </dd>
            </div>
          ))}
      </dl>
      <div className="dir-ui-kit-done-actions">
        {onOpenProfile
          ? report.agents.slice(0, 4).map((id) => (
              <Button key={id} variant="ghost" size="sm" onClick={() => onOpenProfile(id)}>
                Open {id}
              </Button>
            ))
          : null}
        {onOpenKit && report.kind !== 'remove' ? (
          <Button variant="ghost" size="sm" onClick={onOpenKit}>
            View {report.kit}
          </Button>
        ) : null}
        <Button variant="primary" size="sm" onClick={onBack}>
          Back to kits
        </Button>
      </div>
      <p className="dir-ui-kit-fine">Recorded in {report.lock_path} — commit it with the project.</p>
    </section>
  )
}
