/**
 * Screen D — update review, laid out like a pull request's "Files changed":
 * a sidebar of changes grouped as new profiles, changed profiles, workers,
 * ranges, skills (new / changed / removed) and permissions, each with its
 * state and an optional "reviewed" tick; the selected change on the right.
 * `Update` stays disabled while a conflict has no decision.
 */

import { Badge, Button, Checkbox, Eyebrow, FileDiff, MarkdownPreview, SegmentedControl } from '@iii-dev/console-ui'
import { Check, ChevronDown, ChevronRight } from 'lucide-react'
import { useMemo, useState } from 'react'
import { ago } from '../lib/format'
import type { PlanRecord } from './api'
import { ConflictView } from './ConflictView'
import { fileBody, type ReviewProps } from './InstallReview'
import {
  bumpWord,
  type ChangeItem,
  type Choices,
  changeCount,
  decisionsPayload,
  effectiveDecision,
  ownerLabel,
  permissionTally,
  primaryLabel,
  skillLabel,
  unresolvedFiles,
  updateGroups,
} from './model'
import {
  BlockedBlock,
  DecisionRadio,
  PermissionsBlock,
  PlanHeader,
  ProfilePreview,
  SkillPreview,
  WarningList,
  WorkerPreview,
} from './parts'
import { RunDone, RunProgress, useApplyRun } from './run'
import type { Decision, Plan, PlanFile, PlanWorker } from './types'

export function UpdateReview({
  host,
  record,
  onBack,
  onDiscard,
  onReplanned,
  onOpenProfile,
  onOpenKit,
  onIgnore,
  notice,
}: ReviewProps & { onIgnore: () => void }) {
  const plan = record.plan
  const [choices, setChoices] = useState<Choices>({})
  const [removeWorkers, setRemoveWorkers] = useState<string[]>([])
  const groups = useMemo(() => updateGroups(plan), [plan])
  const firstConflict = plan.files.find((f) => f.default === null)
  const [selected, setSelected] = useState<string>(firstConflict?.path ?? groups[0]?.items[0]?.key ?? 'permissions')
  const [reviewed, setReviewed] = useState<Set<string>>(new Set())
  const [notesOpen, setNotesOpen] = useState(true)
  const { phase, run, reset } = useApplyRun(host, plan, onReplanned)
  const pending = unresolvedFiles(plan, choices)
  const blocked = plan.blocking.length > 0
  const title = `Update ${plan.kit} ${plan.from ?? ''} → ${plan.to ?? ''}`
  const bump = bumpWord(plan)
  const badges = (
    <>
      {bump ? (
        <Badge variant={plan.major ? 'alert' : 'default'} className="dir-ui-kit-bump">
          {bump}
        </Badge>
      ) : null}
      {plan.published_at ? <span className="dir-ui-kit-fine">published {ago(plan.published_at)}</span> : null}
    </>
  )
  const apply = () => run(decisionsPayload(plan, choices), removeWorkers)
  const choose = (file: PlanFile, decision: Decision, content?: string) =>
    setChoices((c) => ({ ...c, [file.path]: content === undefined ? { decision } : { decision, content } }))

  if (phase.kind === 'running' || phase.kind === 'failed') {
    return (
      <div className="dir-ui-kit-screen">
        <PlanHeader title={title} plan={plan} badges={badges} />
        <div className="dir-ui-kit-scroll">
          <RunProgress plan={plan} phase={phase} onRetry={apply} onBack={reset} />
        </div>
      </div>
    )
  }
  if (phase.kind === 'done') {
    return (
      <div className="dir-ui-kit-screen">
        <PlanHeader title={title} plan={plan} badges={badges} />
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

  const item = groups.flatMap((g) => g.items).find((i) => i.key === selected)
  const toggleReviewed = (key: string) =>
    setReviewed((r) => {
      const next = new Set(r)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })

  return (
    <div className="dir-ui-kit-screen">
      <PlanHeader title={title} plan={plan} onBack={onBack} badges={badges} />
      <div className="dir-ui-kit-scroll dir-ui-kit-scroll-tight">
        {notice ? <p className="dir-ui-kit-notice">{notice}</p> : null}
        {plan.notes ? (
          <section className="dir-ui-kit-notes">
            <button
              type="button"
              className="dir-ui-kit-notes-toggle"
              onClick={() => setNotesOpen((v) => !v)}
              aria-expanded={notesOpen}
            >
              {notesOpen ? <ChevronDown aria-hidden /> : <ChevronRight aria-hidden />}
              Release notes · {plan.to}
            </button>
            {notesOpen ? (
              <div className="dir-ui-kit-notes-body">
                <MarkdownPreview markdown={plan.notes} className="dir-ui-preview" />
              </div>
            ) : null}
          </section>
        ) : null}
        {/* File-level warnings (merges, conflicts) are the change list's own
            states; only plan-level ones (a major jump) stand above it. */}
        {blocked ? (
          <BlockedBlock issues={plan.blocking} />
        ) : (
          <WarningList issues={plan.warnings.filter((w) => !w.path)} />
        )}

        <div className="dir-ui-kit-pr">
          <nav className="dir-ui-kit-pr-side" aria-label="Changes">
            <Eyebrow as="div" className="dir-ui-kit-pr-count">
              Changes ({changeCount(plan)})
            </Eyebrow>
            {groups.map((group) => (
              <div key={group.id} className="dir-ui-kit-pr-group">
                <div className="dir-ui-kit-pr-group-head">{group.label}</div>
                {group.items.map((it) => (
                  <ChangeRow
                    key={it.key}
                    item={it}
                    plan={plan}
                    choices={choices}
                    selected={selected === it.key}
                    reviewed={reviewed.has(it.key)}
                    onSelect={() => setSelected(it.key)}
                    onToggleReviewed={() => toggleReviewed(it.key)}
                  />
                ))}
              </div>
            ))}
            {plan.counts.unchanged > 0 ? (
              <p className="dir-ui-kit-fine dir-ui-kit-pad">
                {plan.counts.unchanged} file{plan.counts.unchanged === 1 ? '' : 's'} unchanged
              </p>
            ) : null}
          </nav>
          <div className="dir-ui-kit-pr-main">
            {item ? (
              <ChangeDetail
                item={item}
                plan={plan}
                record={record}
                choices={choices}
                onChoose={choose}
                removeWorkers={removeWorkers}
                setRemoveWorkers={setRemoveWorkers}
              />
            ) : null}
          </div>
        </div>
      </div>
      <footer className="dir-ui-kit-foot">
        {plan.to ? (
          <Button variant="ghost" size="sm" onClick={onIgnore} title="Skip this version in update checks">
            Ignore {plan.to}
          </Button>
        ) : null}
        <Button variant="ghost" size="sm" onClick={onBack}>
          Later
        </Button>
        <Button variant="ghost" size="sm" onClick={onDiscard}>
          Discard plan
        </Button>
        <span className="dir-ui-kit-foot-gap" />
        {blocked ? null : (
          <Button
            variant="primary"
            size="sm"
            disabled={pending.length > 0}
            onClick={apply}
            title={pending.length > 0 ? `Resolve: ${pending.map((f) => f.path).join(', ')}` : undefined}
          >
            {primaryLabel(plan, choices)}
          </Button>
        )}
      </footer>
    </div>
  )
}

function sign(item: ChangeItem): { glyph: string; kind: string } {
  if (item.type === 'permissions') return { glyph: '!', kind: 'info' }
  if (item.type === 'worker') {
    const a = item.worker.action
    return a === 'add'
      ? { glyph: '+', kind: 'add' }
      : a === 'remove'
        ? { glyph: '−', kind: 'remove' }
        : { glyph: '~', kind: 'change' }
  }
  const c = item.file.change
  return c === 'added'
    ? { glyph: '+', kind: 'add' }
    : c === 'removed'
      ? { glyph: '−', kind: 'remove' }
      : { glyph: '~', kind: 'change' }
}

function stateFor(item: ChangeItem, plan: Plan, choices: Choices): { label: string; tone: string } {
  if (item.type === 'permissions') return { label: permissionTally(plan), tone: 'quiet' }
  if (item.type === 'worker') {
    const w = item.worker
    if (w.action === 'add') return { label: `${w.range} → ${w.to ?? '?'}`, tone: 'quiet' }
    if (w.action === 'remove') return { label: 'dropped', tone: 'quiet' }
    return { label: `${w.range_from ?? w.declared ?? '?'} → ${w.range ?? '?'}`, tone: 'quiet' }
  }
  const f = item.file
  const decision = effectiveDecision(f, choices)
  if (f.merge === 'conflicts') {
    if (decision === null || unresolvedFiles(plan, choices).some((u) => u.path === f.path)) {
      return { label: 'conflict', tone: 'alert' }
    }
    return { label: 'resolved', tone: 'ok' }
  }
  if (f.local === 'edited' && f.change === 'modified') return { label: 'merges', tone: 'warn' }
  if (f.collision) return { label: 'replaces', tone: 'warn' }
  if (f.change === 'kept') return { label: decision === 'overwrite' ? 'take kit' : 'kept yours', tone: 'quiet' }
  if (f.change === 'removed') return { label: decision === 'keep' ? 'keep local' : 'removed', tone: 'quiet' }
  if (f.local === 'missing') return { label: 'deleted here', tone: 'warn' }
  return { label: f.change === 'added' ? 'new' : 'changed', tone: 'quiet' }
}

function itemLabel(item: ChangeItem): string {
  if (item.type === 'permissions') return 'grants'
  if (item.type === 'worker') return item.worker.name
  return item.file.kind === 'agent' ? item.file.id : skillLabel(item.file)
}

function ChangeRow({
  item,
  plan,
  choices,
  selected,
  reviewed,
  onSelect,
  onToggleReviewed,
}: {
  item: ChangeItem
  plan: Plan
  choices: Choices
  selected: boolean
  reviewed: boolean
  onSelect: () => void
  onToggleReviewed: () => void
}) {
  const s = sign(item)
  const state = stateFor(item, plan, choices)
  return (
    <div className={`dir-ui-kit-pr-row${selected ? ' active' : ''}${reviewed ? ' reviewed' : ''}`}>
      <button
        type="button"
        className="dir-ui-kit-pr-row-main"
        onClick={onSelect}
        aria-current={selected ? 'true' : undefined}
      >
        <span className="dir-ui-kit-sign" data-sign={s.kind}>
          {s.glyph}
        </span>
        <span className="dir-ui-kit-pr-row-label">{itemLabel(item)}</span>
        <span className="dir-ui-kit-pr-row-state" data-tone={state.tone}>
          {state.label}
        </span>
      </button>
      <button
        type="button"
        className="dir-ui-kit-pr-tick"
        aria-pressed={reviewed}
        aria-label={reviewed ? `Mark ${itemLabel(item)} as not reviewed` : `Mark ${itemLabel(item)} as reviewed`}
        title={reviewed ? 'Reviewed' : 'Mark as reviewed'}
        onClick={onToggleReviewed}
      >
        <Check aria-hidden />
      </button>
    </div>
  )
}

function FrontmatterSummary({ file }: { file: PlanFile }) {
  const changes = file.agent_changes
  const fields = Object.entries(changes?.fields ?? {})
  const lists: [string, string[] | undefined, '+' | '−'][] = [
    ['functions', changes?.functions_added, '+'],
    ['functions', changes?.functions_removed, '−'],
    ['skills', changes?.skills_added, '+'],
    ['skills', changes?.skills_removed, '−'],
  ]
  const empty = fields.length === 0 && lists.every(([, l]) => !l || l.length === 0)
  return (
    <section className="dir-ui-kit-fm">
      <Eyebrow as="div" className="dir-ui-kit-fm-head">
        frontmatter
      </Eyebrow>
      {empty ? (
        <p className="dir-ui-kit-fine">
          No frontmatter changes{file.body_changed ? ' — only the prompt changed.' : '.'}
        </p>
      ) : (
        <dl className="dir-ui-kit-fields">
          {fields.map(([key, [from, to]]) => (
            <div key={key} className="dir-ui-kit-field">
              <dt>{key}</dt>
              <dd>
                <code className="dir-ui-kit-old">{from ?? '—'}</code> → <code>{to ?? '—'}</code>
              </dd>
            </div>
          ))}
          {lists
            .filter(([, list]) => list && list.length > 0)
            .map(([key, list, op]) => (
              <div key={key + op} className="dir-ui-kit-field">
                <dt>{key}</dt>
                <dd className="dir-ui-kit-chips">
                  {(list ?? []).map((v) => (
                    <code key={v} className="dir-ui-kit-fn-chip" data-op={op === '+' ? 'add' : 'remove'}>
                      {op} {v}
                    </code>
                  ))}
                </dd>
              </div>
            ))}
        </dl>
      )}
    </section>
  )
}

function ChangeDetail({
  item,
  plan,
  record,
  choices,
  onChoose,
  removeWorkers,
  setRemoveWorkers,
}: {
  item: ChangeItem
  plan: Plan
  record: PlanRecord
  choices: Choices
  onChoose: (file: PlanFile, decision: Decision, content?: string) => void
  removeWorkers: string[]
  setRemoveWorkers: (next: string[]) => void
}) {
  const [style, setStyle] = useState<'split' | 'unified'>('split')
  if (item.type === 'permissions') {
    return <PermissionsBlock plan={plan} title="What this update grants" />
  }
  if (item.type === 'worker') {
    return <WorkerDetail worker={item.worker} removeWorkers={removeWorkers} setRemoveWorkers={setRemoveWorkers} />
  }
  const file = item.file
  const name = file.path.split('/').pop() ?? file.path
  const head = (
    <div className="dir-ui-kit-detail-head">
      <span className="dir-ui-kit-path">{file.path}</span>
      {file.change === 'modified' && file.local !== 'edited' ? (
        <SegmentedControl<'split' | 'unified'>
          value={style}
          onChange={setStyle}
          options={[
            { value: 'split', label: 'Split', icon: false },
            { value: 'unified', label: 'Unified', icon: false },
          ]}
          aria-label="Diff layout"
        />
      ) : null}
    </div>
  )

  // Edited here and changed in the kit: the conflict view.
  if (file.change === 'modified' && file.local === 'edited') {
    return (
      <div className="dir-ui-kit-detail">
        {head}
        {file.kind === 'agent' ? <FrontmatterSummary file={file} /> : null}
        <ConflictView
          plan={plan}
          record={record}
          file={file}
          choices={choices}
          onChoose={(decision, content) => onChoose(file, decision, content)}
        />
      </div>
    )
  }
  if (file.change === 'modified') {
    return (
      <div className="dir-ui-kit-detail">
        {head}
        {file.local === 'missing' ? (
          <div className="dir-ui-kit-choice">
            <span>{file.note}</span>
            <DecisionRadio file={file} choices={choices} onChange={(d) => onChoose(file, d)} />
          </div>
        ) : null}
        {file.kind === 'agent' ? <FrontmatterSummary file={file} /> : null}
        <FileDiff
          oldFile={{ name: `${plan.from}/${name}`, contents: fileBody(record, file.base) ?? '' }}
          newFile={{ name: `${plan.to}/${name}`, contents: fileBody(record, file.theirs) ?? '' }}
          diffStyle={style}
        />
      </div>
    )
  }
  if (file.change === 'kept') {
    return (
      <div className="dir-ui-kit-detail">
        {head}
        <div className="dir-ui-kit-choice">
          <span>When you installed the kit you kept the existing file. The kit's version changed.</span>
          <DecisionRadio file={file} choices={choices} onChange={(d) => onChoose(file, d)} />
        </div>
        <FileDiff
          oldFile={{ name: `current/${name}`, contents: record.contents?.ours[file.path] ?? '' }}
          newFile={{ name: `${plan.to}/${name}`, contents: fileBody(record, file.theirs) ?? '' }}
          diffStyle="split"
        />
      </div>
    )
  }
  if (file.change === 'removed') {
    const base = fileBody(record, file.base)
    return (
      <div className="dir-ui-kit-detail">
        {head}
        <div className="dir-ui-kit-choice">
          <span>{file.note}</span>
          <DecisionRadio file={file} choices={choices} onChange={(d) => onChoose(file, d)} />
        </div>
        {file.kind === 'agent' ? (
          <ProfilePreview kit={plan.kit} agent={file.agent} content={record.contents?.ours[file.path] ?? base} />
        ) : (
          <SkillPreview path={file.path} skill={file.skill} content={record.contents?.ours[file.path] ?? base} />
        )}
      </div>
    )
  }
  // Added.
  return (
    <div className="dir-ui-kit-detail">
      {head}
      {file.collision ? (
        <div className="dir-ui-kit-choice" data-tone="warn">
          <span>This path already exists: it {ownerLabel(file.collision)}.</span>
          <DecisionRadio file={file} choices={choices} onChange={(d) => onChoose(file, d)} />
        </div>
      ) : null}
      {file.kind === 'agent' ? (
        <ProfilePreview kit={plan.kit} agent={file.agent} content={fileBody(record, file.theirs)} />
      ) : (
        <SkillPreview path={file.path} skill={file.skill} content={fileBody(record, file.theirs)} />
      )}
    </div>
  )
}

function WorkerDetail({
  worker,
  removeWorkers,
  setRemoveWorkers,
}: {
  worker: PlanWorker
  removeWorkers: string[]
  setRemoveWorkers: (next: string[]) => void
}) {
  const checked = removeWorkers.includes(worker.name)
  return (
    <div className="dir-ui-kit-detail">
      <WorkerPreview worker={worker} />
      {worker.action === 'remove' ? (
        <div className="dir-ui-kit-choice">
          <Checkbox
            checked={checked}
            onChange={(e) =>
              setRemoveWorkers(
                e.currentTarget.checked
                  ? [...removeWorkers, worker.name]
                  : removeWorkers.filter((n) => n !== worker.name),
              )
            }
            label={`Also remove ${worker.name} from this project`}
          />
          {worker.used_by && worker.used_by.length > 0 ? (
            <span className="dir-ui-kit-fine">Also declared by {worker.used_by.join(', ')}.</span>
          ) : (
            <span className="dir-ui-kit-fine">Workers are never removed unless you tick this.</span>
          )}
        </div>
      ) : null}
    </div>
  )
}
