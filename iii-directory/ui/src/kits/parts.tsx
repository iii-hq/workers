/**
 * Building blocks shared by the Kits screens: the kit header, the
 * non-collapsible collision block, the blocked panel, the permissions
 * block, previews that render a profile's frontmatter as fields, the diff
 * dialog and the apply step list.
 */

import {
  Badge,
  Button,
  Chip,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  Eyebrow,
  FileDiff,
  MarkdownPreview,
  SegmentedControl,
  Skeleton,
  StatusPanel,
} from '@iii-dev/console-ui'
import uiClasses from '@iii-dev/console-ui/ui-classes'
import {
  ArrowLeft,
  BadgeCheck,
  Ban,
  Check,
  ChevronDown,
  ChevronRight,
  Circle,
  CircleX,
  Cpu,
  ExternalLink,
  FileDiff as FileDiffIcon,
  LoaderCircle,
  Minus,
  Puzzle,
  SquareFunction,
  TriangleAlert,
} from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { TokenIcon } from '../page/agent-fields'
import { frontmatterBody } from '../page/frontmatter'
import {
  type Choices,
  collisionHeadline,
  collisionTone,
  decisionLabel,
  effectiveDecision,
  functionsByWorker,
  functionsSummary,
  isKitSkill,
  ownerLabel,
  profileFields,
  workerLine,
} from './model'
import type {
  AgentEntry,
  ApplyStep,
  Decision,
  Issue,
  KitFunction,
  Plan,
  PlanFile,
  PlanWorker,
  SkillEntry,
} from './types'

/** The kit type mark: a puzzle piece + the word, always first in a header. */
export function KitMark() {
  return (
    <span className="dir-ui-kit-mark" title="Kit">
      <Puzzle aria-hidden />
      <Eyebrow as="span">Kit</Eyebrow>
    </span>
  )
}

export function BackButton({ onClick, label = 'Kits' }: { onClick: () => void; label?: string }) {
  return (
    <Button variant="ghost" size="sm" className="dir-ui-kit-back" onClick={onClick}>
      <ArrowLeft aria-hidden />
      <span>{label}</span>
    </Button>
  )
}

export function ExternalLinkButton({ href, children }: { href: string; children: ReactNode }) {
  return (
    <a className="dir-ui-kit-link" href={href} target="_blank" rel="noreferrer">
      {children}
      <ExternalLink aria-hidden />
    </a>
  )
}

/** Header of every plan screen: what is about to happen, by whom. */
export function PlanHeader({
  title,
  plan,
  onBack,
  badges,
}: {
  title: ReactNode
  plan: Plan
  onBack?: () => void
  badges?: ReactNode
}) {
  const author = plan.author
  return (
    <header className="dir-ui-kit-head">
      <div className="dir-ui-kit-head-top">
        {onBack ? <BackButton onClick={onBack} /> : null}
        <h2 className="dir-ui-kit-title">{title}</h2>
        {badges}
        <span className="dir-ui-kit-head-mark">
          <KitMark />
        </span>
      </div>
      <div className="dir-ui-kit-meta">
        {author ? (
          <span className="dir-ui-kit-author">
            by {author.name || author.handle}
            {author.verified ? <BadgeCheck aria-label="verified author" className="dir-ui-kit-verified" /> : null}
          </span>
        ) : null}
        {plan.license ? <span>{plan.license}</span> : null}
        <ExternalLinkButton href={plan.registry_url}>Open in registry</ExternalLinkButton>
      </div>
      {plan.description ? <p className="dir-ui-kit-desc">{plan.description}</p> : null}
    </header>
  )
}

/** A two-choice radio for a file decision. */
export function DecisionRadio({
  file,
  choices,
  onChange,
  label,
}: {
  file: PlanFile
  choices: Choices
  onChange: (decision: Decision) => void
  label?: string
}) {
  const value = effectiveDecision(file, choices) ?? file.options[0]
  return (
    <SegmentedControl<Decision>
      variant="radio"
      value={value}
      onChange={onChange}
      options={file.options.map((d) => ({ value: d, label: decisionLabel(d, file), icon: false }))}
      className="dir-ui-kit-radio"
      aria-label={label ?? `What to do with ${file.path}`}
    />
  )
}

/**
 * The collision block: always on top, never collapsible. One row per
 * existing file the kit would replace, its owner, a diff, and the choice.
 */
export function CollisionBlock({
  plan,
  choices,
  onChoose,
  onViewDiff,
}: {
  plan: Plan
  choices: Choices
  onChoose: (file: PlanFile, decision: Decision) => void
  onViewDiff: (file: PlanFile) => void
}) {
  const collided = plan.files.filter((f) => f.collision)
  if (collided.length === 0) return null
  const strongest = collided.some((f) => f.collision?.owner === 'local') ? 'alert' : 'warn'
  return (
    <section className="dir-ui-kit-block" data-tone={strongest} aria-label="Files that will be replaced">
      <header className="dir-ui-kit-block-head">
        <TriangleAlert aria-hidden />
        <span>{collisionHeadline(plan)}</span>
      </header>
      <p className="dir-ui-kit-block-lede">
        Kit profiles take precedence over the ones workers ship. Keep the current file to leave it as it is — the kit
        records your choice and later updates respect it.
      </p>
      <ul className="dir-ui-kit-collisions">
        {collided.map((file) => {
          const c = file.collision
          if (!c) return null
          const canDiff = c.owner !== 'builtin'
          return (
            <li key={file.path} className="dir-ui-kit-collision" data-tone={collisionTone(c)}>
              <div className="dir-ui-kit-collision-main">
                <span className="dir-ui-kit-path">{file.path}</span>
                <span className="dir-ui-kit-owner" data-owner={c.owner}>
                  {ownerLabel(c)}
                </span>
              </div>
              <div className="dir-ui-kit-collision-actions">
                {canDiff ? (
                  <Button variant="ghost" size="sm" onClick={() => onViewDiff(file)}>
                    <FileDiffIcon aria-hidden />
                    View diff
                  </Button>
                ) : null}
                <DecisionRadio file={file} choices={choices} onChange={(d) => onChoose(file, d)} />
              </div>
            </li>
          )
        })}
      </ul>
    </section>
  )
}

/** Shown INSTEAD of the warnings when the plan cannot be applied. */
export function BlockedBlock({ issues }: { issues: Issue[] }) {
  if (issues.length === 0) return null
  return (
    <section className="dir-ui-kit-block" data-tone="alert" aria-label="Blocked">
      <header className="dir-ui-kit-block-head">
        <Ban aria-hidden />
        <span>{issues.length === 1 ? 'This plan cannot be applied' : `${issues.length} issues block this plan`}</span>
      </header>
      <ul className="dir-ui-kit-issues">
        {issues.map((issue) => (
          <li key={issue.code + (issue.path ?? '')}>
            <IssueText issue={issue} />
          </li>
        ))}
      </ul>
    </section>
  )
}

/** Issue message with `inline code` spans rendered as code. */
export function IssueText({ issue }: { issue: Issue }) {
  const parts = issue.message.split('`')
  return (
    <span className="dir-ui-kit-issue">
      {parts.map((part, i) => (i % 2 === 1 ? <code key={i}>{part}</code> : <span key={i}>{part}</span>))}
    </span>
  )
}

/** Non-collision warnings (merges, major jumps, removals of edited files). */
export function WarningList({ issues }: { issues: Issue[] }) {
  const rest = issues.filter(
    (w) => !w.code.startsWith('agent_') && w.code !== 'builtin_override' && w.code !== 'file_overwrite_local',
  )
  if (rest.length === 0) return null
  return (
    <section className="dir-ui-kit-block" data-tone="warn" aria-label="Warnings">
      <header className="dir-ui-kit-block-head">
        <TriangleAlert aria-hidden />
        <span>{rest.length === 1 ? '1 thing to know' : `${rest.length} things to know`}</span>
      </header>
      <ul className="dir-ui-kit-issues">
        {rest.map((issue) => (
          <li key={issue.code + (issue.path ?? '')}>
            <IssueText issue={issue} />
          </li>
        ))}
      </ul>
    </section>
  )
}

function workerIcon(w: PlanWorker) {
  if (w.action === 'add')
    return (
      <span className="dir-ui-kit-sign" data-sign="add">
        +
      </span>
    )
  if (w.action === 'remove')
    return (
      <span className="dir-ui-kit-sign" data-sign="remove">
        −
      </span>
    )
  return (
    <span className="dir-ui-kit-sign" data-sign="change">
      ~
    </span>
  )
}

/**
 * What the kit is allowed to do on this machine: workers (binaries) it adds
 * or moves, functions its profiles preload, models that change.
 */
export function PermissionsBlock({ plan, title = 'Permissions' }: { plan: Plan; title?: string }) {
  const [open, setOpen] = useState(false)
  const moving = plan.workers.filter((w) => w.action === 'add' || w.action === 'update' || w.action === 'redeclare')
  const functionsAdded = plan.capabilities.functions_added ?? []
  const functions =
    plan.kind === 'install'
      ? plan.functions
      : plan.functions.filter((f) => functionsAdded.some((a) => a.function === f.id))
  // On install every model is new — the profile previews show them; only
  // an update's model changes are a permission change.
  const models = plan.kind === 'update' ? (plan.capabilities.models_changed ?? []).filter((m) => m.from || m.to) : []
  return (
    <section className="dir-ui-kit-block" data-tone="neutral" aria-label={title}>
      <header className="dir-ui-kit-block-head">
        <Cpu aria-hidden />
        <span>{title}</span>
      </header>
      <ul className="dir-ui-kit-perms">
        {moving.length === 0 ? (
          <li className="dir-ui-kit-perm-quiet">
            No worker is added or changed
            {plan.workers.some((w) => w.range && w.installed)
              ? ` — ${plan.workers
                  .filter((w) => w.range && w.installed)
                  .map((w) => `${w.name} ${w.installed} already satisfies ${w.range}`)
                  .join(', ')}`
              : ''}
            .
          </li>
        ) : (
          moving.map((w) => (
            <li key={w.name} className="dir-ui-kit-perm">
              {workerIcon(w)}
              <span className="dir-ui-kit-perm-text">
                <span>{workerLine(w)}</span>
                {w.action === 'add' ? (
                  <span className="dir-ui-kit-perm-sub">
                    {w.type === 'binary' || !w.type ? 'a binary that will run on this machine' : `${w.type} worker`}
                    {w.description ? ` — ${w.description}` : ''}
                  </span>
                ) : null}
              </span>
              <ExternalLinkButton href={w.registry_url}>registry</ExternalLinkButton>
            </li>
          ))
        )}
        {models.map((m) => (
          <li key={`model:${m.agent}`} className="dir-ui-kit-perm">
            <span className="dir-ui-kit-sign" data-sign="change">
              ~
            </span>
            <span className="dir-ui-kit-perm-text">
              {m.from ? (
                <span>
                  {m.agent} runs on <code>{m.to ?? 'the default model'}</code> instead of <code>{m.from}</code>
                </span>
              ) : (
                <span>
                  {m.agent} runs on <code>{m.to}</code>
                </span>
              )}
            </span>
          </li>
        ))}
        {functions.length > 0 ? (
          <li className="dir-ui-kit-perm dir-ui-kit-perm-fns">
            <span className="dir-ui-kit-sign" data-sign="info">
              <SquareFunction aria-hidden />
            </span>
            <span className="dir-ui-kit-perm-text">
              <span>
                {plan.kind === 'install'
                  ? functionsSummary(functions)
                  : `The profiles preload ${functionsAdded.length} new function${functionsAdded.length === 1 ? '' : 's'}`}
              </span>
            </span>
            <Button variant="ghost" size="sm" onClick={() => setOpen((v) => !v)} aria-expanded={open}>
              {open ? <ChevronDown aria-hidden /> : <ChevronRight aria-hidden />}
              {open ? 'Hide list' : 'Show list'}
            </Button>
          </li>
        ) : null}
      </ul>
      {open ? <FunctionTable functions={functions} /> : null}
    </section>
  )
}

export function FunctionTable({ functions }: { functions: KitFunction[] }) {
  return (
    <div className="dir-ui-kit-fns">
      {functionsByWorker(functions).map((group) => (
        <div key={group.worker || 'unknown'} className="dir-ui-kit-fn-group">
          <Eyebrow as="div" className="dir-ui-kit-fn-head">
            {group.worker ? `worker ${group.worker}` : 'not provided by a declared worker'}
          </Eyebrow>
          <ul>
            {group.functions.map((f) => (
              <li key={f.id} className="dir-ui-kit-fn">
                <code>{f.id}</code>
                <span className="dir-ui-kit-fn-used">{(f.used_by ?? []).join(', ')}</span>
                {f.status === 'unknown' ? (
                  <Badge variant="warn" title="No declared worker lists this function">
                    unknown
                  </Badge>
                ) : null}
              </li>
            ))}
          </ul>
        </div>
      ))}
    </div>
  )
}

/** One frontmatter field as a labelled value. */
function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="dir-ui-kit-field">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  )
}

/**
 * A profile as the review shows it: frontmatter as fields (logo, model,
 * extends, preloaded skills and functions), then the system prompt.
 */
export function ProfilePreview({
  kit,
  content,
  agent,
  sourceToggle = true,
}: {
  kit: string
  content: string | undefined
  agent?: AgentEntry
  sourceToggle?: boolean
}) {
  const [mode, setMode] = useState<'preview' | 'source'>('preview')
  if (content === undefined) return <PreviewSkeleton />
  const fm = profileFields(content)
  const name = fm.name || agent?.name || agent?.id || ''
  return (
    <div className="dir-ui-kit-preview">
      <div className="dir-ui-kit-profile-id">
        <span className="dir-ui-nav-ico" data-color={fm.color || undefined} aria-hidden>
          {fm.logo ? <span className="dir-ui-kit-logo">{fm.logo}</span> : <TokenIcon token={fm.icon || 'agent'} />}
        </span>
        <span className="dir-ui-kit-profile-names">
          <span className="dir-ui-kit-profile-name">{name}</span>
          <span className="dir-ui-kit-path">{agent?.path ?? ''}</span>
        </span>
        {sourceToggle ? (
          <SegmentedControl<'preview' | 'source'>
            value={mode}
            onChange={setMode}
            options={[
              { value: 'preview', label: 'Preview', icon: false },
              { value: 'source', label: 'Source', icon: false },
            ]}
            className="dir-ui-kit-mode"
            aria-label="Preview or source"
          />
        ) : null}
      </div>
      {mode === 'source' ? (
        <pre className="dir-ui-kit-source">{content}</pre>
      ) : (
        <>
          {fm.description ? <p className="dir-ui-kit-desc">{fm.description}</p> : null}
          <dl className="dir-ui-kit-fields">
            {fm.model ? (
              <Field label="model">
                <code>{fm.model}</code>
                {fm.reasoning_effort ? <span className="dir-ui-kit-fine"> · effort {fm.reasoning_effort}</span> : null}
              </Field>
            ) : null}
            {fm.extends ? (
              <Field label="extends">
                <code>{fm.extends}</code>
              </Field>
            ) : null}
            {fm.icon || fm.color ? (
              <Field label="look">
                <span className="dir-ui-kit-swatch" data-color={fm.color || undefined}>
                  <TokenIcon token={fm.icon || 'agent'} />
                </span>
                <span className="dir-ui-kit-fine">{[fm.icon, fm.color].filter(Boolean).join(' · ')}</span>
              </Field>
            ) : null}
            <Field label={`skills (${fm.skills.length})`}>
              {fm.skills.length === 0 ? (
                <span className="dir-ui-kit-fine">every skill</span>
              ) : (
                <span className="dir-ui-kit-chips">
                  {fm.skills.map((s) => (
                    <Chip
                      key={s}
                      tone={isKitSkill(kit, s) ? 'accent' : 'neutral'}
                      title={isKitSkill(kit, s) ? 'in this kit' : 'from a worker'}
                    >
                      {s}
                    </Chip>
                  ))}
                </span>
              )}
            </Field>
            <Field label={`functions (${fm.functions.length})`}>
              {fm.functions.length === 0 ? (
                <span className="dir-ui-kit-fine">none preloaded</span>
              ) : (
                <span className="dir-ui-kit-chips">
                  {fm.functions.map((f) => (
                    <code key={f} className="dir-ui-kit-fn-chip">
                      {f}
                    </code>
                  ))}
                </span>
              )}
            </Field>
          </dl>
          <Eyebrow as="div" className="dir-ui-kit-prompt-head">
            system prompt
          </Eyebrow>
          {fm.body.trim() ? (
            <MarkdownPreview markdown={fm.body} className="dir-ui-preview dir-ui-kit-md" />
          ) : (
            <p className="dir-ui-kit-fine">No prompt of its own{fm.extends ? ` — it serves ${fm.extends}'s` : ''}.</p>
          )}
        </>
      )}
    </div>
  )
}

export function SkillPreview({
  content,
  skill,
  path,
}: {
  content: string | undefined
  skill?: SkillEntry
  path: string
}) {
  if (content === undefined) return <PreviewSkeleton />
  return (
    <div className="dir-ui-kit-preview">
      <div className="dir-ui-kit-profile-id">
        <span className="dir-ui-kit-profile-names">
          <span className="dir-ui-kit-profile-name">{skill?.title || skill?.id || path}</span>
          <span className="dir-ui-kit-path">{skill?.id ?? path}</span>
        </span>
      </div>
      {skill?.description ? <p className="dir-ui-kit-desc">{skill.description}</p> : null}
      {skill?.used_by && skill.used_by.length > 0 ? (
        <dl className="dir-ui-kit-fields">
          <Field label="preloaded by">
            <span className="dir-ui-kit-chips">
              {skill.used_by.map((a) => (
                <Chip key={a}>{a}</Chip>
              ))}
            </span>
          </Field>
        </dl>
      ) : null}
      <MarkdownPreview markdown={frontmatterBody(content)} className="dir-ui-preview dir-ui-kit-md" />
    </div>
  )
}

export function WorkerPreview({ worker }: { worker: PlanWorker }) {
  return (
    <div className="dir-ui-kit-preview">
      <div className="dir-ui-kit-profile-id">
        <span className="dir-ui-nav-ico" aria-hidden>
          <Cpu />
        </span>
        <span className="dir-ui-kit-profile-names">
          <span className="dir-ui-kit-profile-name">{worker.name}</span>
          <span className="dir-ui-kit-path">{worker.type ?? 'worker'}</span>
        </span>
        <ExternalLinkButton href={worker.registry_url}>Open in registry</ExternalLinkButton>
      </div>
      {worker.description ? <p className="dir-ui-kit-desc">{worker.description}</p> : null}
      <dl className="dir-ui-kit-fields">
        <Field label="range">
          {worker.range_from && worker.range_from !== worker.range ? (
            <>
              <code>{worker.range_from}</code> → <code>{worker.range ?? '—'}</code>
            </>
          ) : (
            <code>{worker.range ?? worker.range_from ?? '—'}</code>
          )}
        </Field>
        <Field label="installed">
          {worker.installed ? <code>{worker.installed}</code> : <span className="dir-ui-kit-fine">not installed</span>}
          {worker.declared && worker.declared !== worker.installed ? (
            <span className="dir-ui-kit-fine"> · declared {worker.declared}</span>
          ) : null}
        </Field>
        {worker.to ? (
          <Field label="resolves to">
            <code>{worker.to}</code>
          </Field>
        ) : null}
        <Field label="change">{workerLine(worker)}</Field>
        {worker.used_by && worker.used_by.length > 0 ? (
          <Field label="also used by">{worker.used_by.join(', ')}</Field>
        ) : null}
      </dl>
    </div>
  )
}

export function PreviewSkeleton() {
  return (
    <div className="dir-ui-kit-preview" aria-hidden>
      <div className="dir-ui-loading">
        <Skeleton style={{ width: '42%' }} />
        <Skeleton style={{ width: '70%' }} />
        <Skeleton style={{ width: '58%' }} />
      </div>
    </div>
  )
}

/** Split/unified file diff in a dialog (collisions, "view my changes"). */
export function DiffDialog({
  open,
  onOpenChange,
  title,
  description,
  left,
  right,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: string
  left: { name: string; contents: string } | null
  right: { name: string; contents: string } | null
}) {
  const [style, setStyle] = useState<'split' | 'unified'>('split')
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="dir-ui-kit-dialog"
        style={{ width: 'min(1100px, 94vw)', maxWidth: 'min(1100px, 94vw)' }}
      >
        <div
          className="dir-ui-kit-dialog-head"
          style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12, paddingRight: 32 }}
        >
          <DialogTitle>{title}</DialogTitle>
          <SegmentedControl<'split' | 'unified'>
            value={style}
            onChange={setStyle}
            options={[
              { value: 'split', label: 'Split', icon: false },
              { value: 'unified', label: 'Unified', icon: false },
            ]}
            aria-label="Diff layout"
          />
        </div>
        {description ? <DialogDescription>{description}</DialogDescription> : null}
        <div className="dir-ui-kit-dialog-body" style={{ maxHeight: '70vh', overflow: 'auto' }}>
          {left && right ? <FileDiff oldFile={left} newFile={right} diffStyle={style} /> : <PreviewSkeleton />}
        </div>
      </DialogContent>
    </Dialog>
  )
}

function stepIcon(step: ApplyStep) {
  switch (step.state) {
    case 'done':
      return <Check aria-hidden />
    case 'running':
      return <LoaderCircle aria-hidden className={uiClasses.spin} />
    case 'failed':
      return <CircleX aria-hidden />
    case 'skipped':
      return <Minus aria-hidden />
    default:
      return <Circle aria-hidden />
  }
}

/** Workers → files → kits.lock, each with its state. */
export function StepList({ steps }: { steps: ApplyStep[] }) {
  return (
    <ol className="dir-ui-kit-steps">
      {steps.map((step, i) => (
        <li key={step.id} className="dir-ui-kit-step" data-state={step.state}>
          <span className="dir-ui-kit-step-icon">{stepIcon(step)}</span>
          <span className="dir-ui-kit-step-text">
            <span className="dir-ui-kit-step-label">
              {i + 1}. {step.label}
            </span>
            {step.detail ? <span className="dir-ui-kit-step-detail">{step.detail}</span> : null}
          </span>
        </li>
      ))}
    </ol>
  )
}

export function StatusLine({ tone, children }: { tone: 'warn' | 'alert' | 'ok' | 'info'; children: ReactNode }) {
  return (
    <StatusPanel
      variant={tone === 'ok' ? 'success' : tone === 'info' ? 'info' : tone}
      headline={children}
      className="dir-ui-kit-status"
    />
  )
}

/** State chip for an installed file. */
export function FileStateChip({ state }: { state: 'intact' | 'edited' | 'missing' | 'skipped' }) {
  const tone =
    state === 'intact' ? 'neutral' : state === 'edited' ? 'warning' : state === 'missing' ? 'danger' : 'neutral'
  const label = state === 'skipped' ? 'kept yours' : state
  return (
    <Chip tone={tone} className="dir-ui-kit-state" data-state={state}>
      {label}
    </Chip>
  )
}
