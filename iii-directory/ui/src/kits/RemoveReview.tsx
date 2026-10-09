/**
 * Screen F — removal. Lists the files that will be deleted (files you
 * edited come unticked, with a warning) and the kit's workers, each with an
 * unticked box and a note when another kit still uses it.
 */

import { Button, Checkbox, Eyebrow } from '@iii-dev/console-ui'
import { useState } from 'react'
import type { ReviewProps } from './InstallReview'
import { type Choices, decisionsPayload, effectiveDecision, primaryLabel } from './model'
import { FileStateChip, PlanHeader, WarningList } from './parts'
import { RunDone, RunProgress, useApplyRun } from './run'

export function RemoveReview({ host, record, onBack, onDiscard, onReplanned, notice }: ReviewProps) {
  const plan = record.plan
  const [choices, setChoices] = useState<Choices>({})
  const [removeWorkers, setRemoveWorkers] = useState<string[]>([])
  const { phase, run, reset } = useApplyRun(host, plan, onReplanned)
  const title = `Remove ${plan.kit} ${plan.from ?? ''}`
  const apply = () => run(decisionsPayload(plan, choices), removeWorkers)

  if (phase.kind === 'running' || phase.kind === 'failed') {
    return (
      <div className="dir-ui-kit-screen">
        <PlanHeader title={title} plan={plan} />
        <div className="dir-ui-kit-scroll">
          <RunProgress plan={plan} phase={phase} onRetry={apply} onBack={reset} />
        </div>
      </div>
    )
  }
  if (phase.kind === 'done') {
    return (
      <div className="dir-ui-kit-screen">
        <PlanHeader title={title} plan={plan} />
        <div className="dir-ui-kit-scroll">
          <RunDone report={phase.report} message={phase.message} onBack={onBack} />
        </div>
      </div>
    )
  }

  const agents = plan.files.filter((f) => f.kind === 'agent')
  const skills = plan.files.filter((f) => f.kind === 'skill')
  const removable = plan.workers.filter((w) => w.action === 'remove')
  const group = (label: string, files: typeof plan.files) =>
    files.length === 0 ? null : (
      <section className="dir-ui-kit-remove-group">
        <Eyebrow as="div" className="dir-ui-kit-column-head">
          {label} ({files.length})
        </Eyebrow>
        <ul className="dir-ui-kit-remove-list">
          {files.map((f) => {
            const removing = effectiveDecision(f, choices) === 'remove'
            return (
              <li key={f.path} className="dir-ui-kit-remove-row" data-edited={f.local === 'edited' || undefined}>
                <Checkbox
                  checked={removing}
                  onChange={(e) =>
                    setChoices((c) => ({ ...c, [f.path]: { decision: e.currentTarget.checked ? 'remove' : 'keep' } }))
                  }
                  label={<span className="dir-ui-kit-path">{f.path}</span>}
                />
                {f.local === 'edited' ? <FileStateChip state="edited" /> : null}
                {!removing ? <span className="dir-ui-kit-fine">stays as a local file</span> : null}
              </li>
            )
          })}
        </ul>
      </section>
    )

  return (
    <div className="dir-ui-kit-screen">
      <PlanHeader title={title} plan={plan} onBack={onBack} />
      <div className="dir-ui-kit-scroll">
        {notice ? <p className="dir-ui-kit-notice">{notice}</p> : null}
        <WarningList issues={plan.warnings} />
        <p className="dir-ui-kit-lede">
          Ticked files are deleted. The kit leaves kits.lock; files you keep become ordinary local files.
        </p>
        {group('Profiles', agents)}
        {group('Skills', skills)}
        {plan.workers.length > 0 ? (
          <section className="dir-ui-kit-remove-group">
            <Eyebrow as="div" className="dir-ui-kit-column-head">
              Workers ({plan.workers.length})
            </Eyebrow>
            <ul className="dir-ui-kit-remove-list">
              {plan.workers.map((w) => {
                const checked = removeWorkers.includes(w.name)
                const can = removable.includes(w)
                return (
                  <li key={w.name} className="dir-ui-kit-remove-row">
                    <Checkbox
                      checked={checked}
                      disabled={!can}
                      onChange={(e) =>
                        setRemoveWorkers((list) =>
                          e.currentTarget.checked ? [...list, w.name] : list.filter((n) => n !== w.name),
                        )
                      }
                      label={
                        <span>
                          <span className="dir-ui-kit-path">{w.name}</span>{' '}
                          <span className="dir-ui-kit-fine">
                            {w.range_from} · {w.installed ? `installed ${w.installed}` : 'not installed'}
                            {w.path ? ' · local path worker' : ''}
                          </span>
                        </span>
                      }
                    />
                    {w.used_by && w.used_by.length > 0 ? (
                      <span className="dir-ui-kit-row-status" data-status="shared">
                        also used by {w.used_by.join(', ')}
                      </span>
                    ) : null}
                  </li>
                )
              })}
            </ul>
            <p className="dir-ui-kit-fine">Workers stay unless you tick them.</p>
          </section>
        ) : null}
      </div>
      <footer className="dir-ui-kit-foot">
        <Button variant="ghost" size="sm" onClick={onDiscard}>
          Discard plan
        </Button>
        <span className="dir-ui-kit-foot-gap" />
        <Button variant="ghost" size="sm" onClick={onBack}>
          Cancel
        </Button>
        <Button variant="primary" size="sm" className="dir-ui-kit-danger" onClick={apply}>
          {primaryLabel(plan, choices, removeWorkers)}
        </Button>
      </footer>
    </div>
  )
}
