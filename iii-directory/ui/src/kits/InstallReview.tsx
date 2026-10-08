/**
 * Screen C — install review. Warnings on top (never collapsible), then the
 * permissions the kit asks for, the contents in three columns with a
 * preview pane, and a primary button that says what will happen
 * ("Install and replace 2 profiles"). A blocked plan shows the block
 * instead of the warnings and no primary button.
 */

import { Button, Eyebrow, type Host } from '@iii-dev/console-ui'
import uiClasses from '@iii-dev/console-ui/ui-classes'
import { Cpu, FileText, TriangleAlert } from 'lucide-react'
import { useMemo, useState } from 'react'
import { TokenIcon } from '../page/agent-fields'
import type { PlanRecord } from './api'
import {
  type Choices,
  decisionsPayload,
  effectiveDecision,
  primaryLabel,
  skillFolders,
  skillLabel,
  workerStatusWord,
} from './model'
import {
  BlockedBlock,
  CollisionBlock,
  DiffDialog,
  PermissionsBlock,
  PlanHeader,
  ProfilePreview,
  SkillPreview,
  WarningList,
  WorkerPreview,
} from './parts'
import { RunDone, RunProgress, useApplyRun } from './run'
import type { Plan, PlanFile } from './types'

export interface ReviewProps {
  host: Host
  record: PlanRecord
  onBack: () => void
  onDiscard: () => void
  onReplanned: (plan: Plan, message: string) => void
  onOpenProfile?: (id: string) => void
  onOpenKit?: (kit: string) => void
  notice?: string | null
}

type Selection = { type: 'file'; path: string } | { type: 'worker'; name: string }

export function fileBody(record: PlanRecord, sha: string | undefined): string | undefined {
  if (!sha) return undefined
  return record.contents?.blobs[sha]
}

export function InstallReview({
  host,
  record,
  onBack,
  onDiscard,
  onReplanned,
  onOpenProfile,
  onOpenKit,
  notice,
}: ReviewProps) {
  const plan = record.plan
  const [choices, setChoices] = useState<Choices>({})
  const firstAgent = plan.files.find((f) => f.kind === 'agent') ?? plan.files[0]
  const [selected, setSelected] = useState<Selection | null>(
    firstAgent ? { type: 'file', path: firstAgent.path } : null,
  )
  const [diffFor, setDiffFor] = useState<PlanFile | null>(null)
  const { phase, run, reset } = useApplyRun(host, plan, onReplanned)
  const blocked = plan.blocking.length > 0
  const agents = plan.files.filter((f) => f.kind === 'agent')
  const folders = useMemo(() => skillFolders(plan.files), [plan.files])
  const skillCount = plan.files.length - agents.length
  const workers = plan.workers.filter((w) => w.range)

  const choose = (file: PlanFile, decision: PlanFile['options'][number]) =>
    setChoices((c) => ({ ...c, [file.path]: { decision } }))
  const apply = () => run(decisionsPayload(plan, choices))
  const title = `Install ${plan.kit} ${plan.to ?? ''}`

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
          <RunDone
            report={phase.report}
            message={phase.message}
            onOpenProfile={onOpenProfile}
            onOpenKit={onOpenKit ? () => onOpenKit(plan.kit) : undefined}
            onBack={onBack}
          />
        </div>
      </div>
    )
  }

  const selectedFile = selected?.type === 'file' ? plan.files.find((f) => f.path === selected.path) : undefined
  const selectedWorker = selected?.type === 'worker' ? plan.workers.find((w) => w.name === selected.name) : undefined

  return (
    <div className="dir-ui-kit-screen">
      <PlanHeader title={title} plan={plan} onBack={onBack} />
      <div className="dir-ui-kit-scroll">
        {notice ? <p className="dir-ui-kit-notice">{notice}</p> : null}
        {blocked ? (
          <BlockedBlock issues={plan.blocking} />
        ) : (
          <>
            <CollisionBlock plan={plan} choices={choices} onChoose={choose} onViewDiff={setDiffFor} />
            <WarningList issues={plan.warnings} />
          </>
        )}
        <PermissionsBlock plan={plan} />

        <section className="dir-ui-kit-columns" aria-label="Kit contents">
          <div className="dir-ui-kit-column">
            <Eyebrow as="div" className="dir-ui-kit-column-head">
              Profiles ({agents.length})
            </Eyebrow>
            <div className={uiClasses.list}>
              {agents.map((f) => (
                <ContentRow
                  key={f.path}
                  selected={selected?.type === 'file' && selected.path === f.path}
                  onClick={() => setSelected({ type: 'file', path: f.path })}
                  icon={
                    f.agent?.logo ? (
                      <span className="dir-ui-kit-logo">{f.agent.logo}</span>
                    ) : (
                      <TokenIcon token={f.agent?.icon || 'agent'} />
                    )
                  }
                  label={f.id}
                  warn={!!f.collision}
                  trailing={
                    f.collision && effectiveDecision(f, choices) === 'keep' ? 'kept' : (f.agent?.model ?? undefined)
                  }
                />
              ))}
            </div>
          </div>
          <div className="dir-ui-kit-column">
            <Eyebrow as="div" className="dir-ui-kit-column-head">
              Skills ({skillCount})
            </Eyebrow>
            <div className={uiClasses.list}>
              {folders.map((group) => (
                <div key={group.folder || 'root'} className="dir-ui-kit-folder">
                  {group.folder ? (
                    <div className="dir-ui-kit-folder-head">
                      {group.folder}/ <span className="dir-ui-kit-fine">({group.files.length})</span>
                    </div>
                  ) : null}
                  {group.files.map((f) => (
                    <ContentRow
                      key={f.path}
                      selected={selected?.type === 'file' && selected.path === f.path}
                      onClick={() => setSelected({ type: 'file', path: f.path })}
                      icon={<FileText />}
                      label={group.folder ? skillLabel(f).slice(group.folder.length + 1) : skillLabel(f)}
                      warn={!!f.collision}
                    />
                  ))}
                </div>
              ))}
            </div>
          </div>
          <div className="dir-ui-kit-column">
            <Eyebrow as="div" className="dir-ui-kit-column-head">
              Workers ({workers.length})
            </Eyebrow>
            <div className={uiClasses.list}>
              {workers.length === 0 ? <p className="dir-ui-kit-fine dir-ui-kit-pad">No workers.</p> : null}
              {workers.map((w) => (
                <ContentRow
                  key={w.name}
                  selected={selected?.type === 'worker' && selected.name === w.name}
                  onClick={() => setSelected({ type: 'worker', name: w.name })}
                  icon={<Cpu />}
                  label={w.name}
                  status={workerStatusWord(w)}
                  trailing={w.action === 'none' ? (w.installed ?? undefined) : (w.to ?? undefined)}
                />
              ))}
            </div>
          </div>
        </section>

        <section className="dir-ui-kit-pane" aria-label="Preview">
          {selectedFile ? (
            selectedFile.kind === 'agent' ? (
              <ProfilePreview
                kit={plan.kit}
                agent={selectedFile.agent}
                content={fileBody(record, selectedFile.theirs)}
              />
            ) : (
              <SkillPreview
                path={selectedFile.path}
                skill={selectedFile.skill}
                content={fileBody(record, selectedFile.theirs)}
              />
            )
          ) : selectedWorker ? (
            <WorkerPreview worker={selectedWorker} />
          ) : (
            <p className="dir-ui-kit-fine dir-ui-kit-pad">Select a profile, skill or worker to preview it.</p>
          )}
        </section>
      </div>
      <footer className="dir-ui-kit-foot">
        <Button variant="ghost" size="sm" onClick={onDiscard}>
          Discard plan
        </Button>
        <span className="dir-ui-kit-foot-gap" />
        <span className="dir-ui-kit-fine">expires {new Date(plan.expires_at).toLocaleString()}</span>
        <Button variant="ghost" size="sm" onClick={onBack}>
          Not now
        </Button>
        {blocked ? null : (
          <Button variant="primary" size="sm" onClick={apply}>
            {primaryLabel(plan, choices)}
          </Button>
        )}
      </footer>
      <DiffDialog
        open={diffFor !== null}
        onOpenChange={(open) => (open ? null : setDiffFor(null))}
        title={diffFor ? diffFor.path : ''}
        description={
          diffFor?.collision ? `Left: the current file (${diffFor.note ?? ''}). Right: the kit's version.` : undefined
        }
        left={diffFor ? { name: `current/${diffFor.path}`, contents: record.contents?.ours[diffFor.path] ?? '' } : null}
        right={diffFor ? { name: `kit/${diffFor.path}`, contents: fileBody(record, diffFor.theirs) ?? '' } : null}
      />
    </div>
  )
}

export function ContentRow({
  selected,
  onClick,
  icon,
  label,
  warn,
  status,
  trailing,
}: {
  selected: boolean
  onClick: () => void
  icon: React.ReactNode
  label: string
  warn?: boolean
  status?: string
  trailing?: string
}) {
  return (
    <button
      type="button"
      className={`${uiClasses.listItem} dir-ui-kit-row${selected ? ' active' : ''}`}
      aria-current={selected ? 'true' : undefined}
      onClick={onClick}
    >
      <span className={`${uiClasses.listItemIcon} dir-ui-kit-row-icon`} aria-hidden>
        {icon}
      </span>
      <span className={uiClasses.listItemContent}>
        <span className={`${uiClasses.listItemTitle} dir-ui-kit-row-label`}>{label}</span>
      </span>
      {warn ? (
        <span className="dir-ui-kit-row-warn" title="replaces an existing file">
          <TriangleAlert aria-label="replaces an existing file" />
        </span>
      ) : null}
      {status ? (
        <span className="dir-ui-kit-row-status" data-status={status}>
          {status}
        </span>
      ) : null}
      {trailing ? <span className={`${uiClasses.listItemMeta} dir-ui-kit-row-meta`}>{trailing}</span> : null}
    </button>
  )
}
