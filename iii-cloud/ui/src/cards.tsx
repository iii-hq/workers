import type { FunctionCallMessage, FunctionCallRenderer, Host } from '@iii-dev/console-ui'
import { Button, Card, MetaRow } from '@iii-dev/console-ui'
import { KeyRound, Rocket, ShieldCheck, ShieldQuestion } from 'lucide-react'
import type { ReactNode } from 'react'
import { useRecord, useStatusSnapshot } from './hub'
import {
  CALL_IDS,
  CONNECTION_LABEL,
  deployRows,
  deviceLabel,
  duration,
  envSlug,
  failedStage,
  failureWords,
  isFailedDeploy,
  needsAction,
  PAGE_ID,
  peopleToDecide,
  planLine,
  runLine,
  sha7,
  unwrapDetails,
} from './model'
import { Dot, StatusChip } from './parts'
import type { Route } from './route'
import type { Check, Connection, Deploy, SecretRequest } from './types'

type Line = { tone: 'ok' | 'alert' | 'warn' | 'live' | 'edge' | null; text: ReactNode }
type CardView = {
  icon: ReactNode
  fn: string
  meta: { label: string; value: string }[]
  chip: { tone: 'neutral' | 'success' | 'warning' | 'danger'; label: string; live?: boolean }
  lines: Line[]
  /** Opens the page at this route, beside the chat. */
  action?: { label: string; open: Route }
}

function Shell({ host, view }: { host: Host; view: CardView }) {
  return (
    <Card className="iii-cloud-fcard">
      <div className="iii-cloud-fc-head">
        {view.icon}
        <strong>{view.fn}</strong>
        <MetaRow items={view.meta} />
        <span className="iii-cloud-grow" />
        <StatusChip tone={view.chip.tone} live={view.chip.live}>
          {view.chip.label}
        </StatusChip>
      </div>
      {view.lines.length > 0 ? (
        <div className="iii-cloud-fc-body">
          {view.lines.map((l, i) => (
            <div className="iii-cloud-fc-line" key={i}>
              {l.tone ? <Dot tone={l.tone} /> : null}
              <span>{l.text}</span>
            </div>
          ))}
        </div>
      ) : null}
      {view.action ? (
        <div className="iii-cloud-fc-foot">
          <Button
            size="sm"
            variant="ghost"
            onClick={() => view.action && host.panels?.open({ pageId: PAGE_ID, context: view.action.open })}
          >
            {view.action.label}
          </Button>
        </div>
      ) : null}
    </Card>
  )
}

function connectView(c: Connection): CardView {
  const waiting = c.status === 'waiting'
  return {
    icon: <KeyRound />,
    fn: 'iii-cloud::connect',
    meta: [
      { label: 'via', value: c.method ?? 'browser' },
      ...(waiting && c.device ? [{ label: 'code', value: c.device.user_code }] : []),
      ...(c.status === 'connected' && c.teams[0] ? [{ label: 'team', value: c.teams[0].name }] : []),
    ],
    chip: waiting
      ? { tone: 'warning', label: 'Waiting for you' }
      : c.status === 'connected'
        ? { tone: 'success', label: 'Signed in' }
        : { tone: 'danger', label: CONNECTION_LABEL[c.status] },
    lines:
      waiting && c.device
        ? [{ tone: null, text: <span className="iii-cloud-mono">{deviceLabel(c.device.verification_uri)}</span> }]
        : c.error
          ? [{ tone: 'alert', text: c.error.message }]
          : [],
  }
}

function checkView(c: Check, env: string): CardView {
  const lines: Line[] = []
  const decide = peopleToDecide(c)
  lines.push({ tone: c.plan.blocking ? 'alert' : 'ok', text: planLine(c) })
  if (!c.ready) {
    lines.push({
      tone: c.here.problems ? 'alert' : 'ok',
      text: `Checks here: ${c.here.problems} ${c.here.problems === 1 ? 'problem' : 'problems'}`,
    })
    if (decide) lines.push({ tone: 'warn', text: `${decide} need your decision` })
  } else if (c.run) {
    lines.push({ tone: c.run.status === 'passed' ? 'ok' : 'alert', text: runLine(c.run) })
  }
  const took = c.run?.finished_at
    ? duration(Date.parse(c.run.finished_at) - Date.parse(c.run.started_at))
    : duration(c.duration_ms)
  return {
    icon: <ShieldCheck />,
    fn: 'iii-cloud::check',
    meta: [
      { label: 'env', value: env },
      { label: 'commit', value: sha7(c.commit) },
      { label: 'took', value: took },
    ],
    chip: c.ready ? { tone: 'success', label: 'Ready' } : { tone: 'danger', label: 'Not ready' },
    lines,
    action: c.ready
      ? c.run
        ? { label: 'See the run', open: { at: 'deploy', env: c.environment_id } }
        : undefined
      : { label: 'Open the check', open: { at: 'deploy', env: c.environment_id } },
  }
}

const SECRET_CHIPS = {
  waiting: { tone: 'warning', label: 'Waiting for you' },
  set: { tone: 'success', label: 'Set' },
  left_empty: { tone: 'neutral', label: 'Left empty' },
  noted: { tone: 'neutral', label: 'Noted' },
  failed: { tone: 'danger', label: 'Failed' },
} as const

function secretView(r: SecretRequest, env: string): CardView {
  return {
    icon: <ShieldQuestion />,
    fn: 'iii-cloud::secret',
    meta: [{ label: 'env', value: env }],
    chip: SECRET_CHIPS[r.state],
    lines: [
      {
        tone: null,
        text: (
          <>
            <span className="iii-cloud-mono">{r.name}</span>
            {r.state === 'waiting' ? (
              <span className="iii-cloud-faint">
                {' '}
                · type it on the iii cloud page; the value never comes back to this chat
              </span>
            ) : null}
          </>
        ),
      },
    ],
    action:
      r.state === 'waiting'
        ? { label: 'Open secrets and variables', open: { at: 'secrets', env: r.environment_id } }
        : undefined,
  }
}

function deployView(d: Deploy, fn: string, env: string): CardView {
  const rows = deployRows(d)
  const lines: Line[] = []
  if (isFailedDeploy(d)) {
    const e = d.error
    lines.push({ tone: 'alert', text: `${failedStage(d)}: ${e ? failureWords(e) : d.outcome}` })
  } else if (d.outcome === 'ready') {
    lines.push({ tone: 'ok', text: 'Ready' })
  } else if (needsAction(d)) {
    lines.push({ tone: 'warn', text: 'iii cloud holds this deploy until you approve it' })
  } else {
    const done = [...rows].reverse().find((r) => r.tone === 'ok')
    const live = rows.find((r) => r.tone === 'live')
    if (done) lines.push({ tone: 'ok', text: `${done.label}${done.time ? ` · ${done.time}` : ''}` })
    if (live) lines.push({ tone: 'live', text: `${live.label}${live.detail ? `: ${live.detail}` : ''}` })
  }
  const end = d.finished_at ? Date.parse(d.finished_at) : Date.now()
  return {
    icon: <Rocket />,
    fn,
    meta: [
      { label: 'env', value: env },
      { label: 'deploy', value: sha7(d.commit.sha) },
      { label: 'for', value: duration(end - Date.parse(d.created_at)) },
    ],
    chip: needsAction(d)
      ? { tone: 'warning', label: 'Needs your action' }
      : d.outcome === 'running'
        ? { tone: 'neutral', label: 'Deploying', live: true }
        : d.outcome === 'ready'
          ? { tone: 'success', label: 'Ready' }
          : isFailedDeploy(d)
            ? { tone: 'danger', label: 'Failed' }
            : { tone: 'neutral', label: d.outcome },
    lines,
    action: { label: 'Open deployment', open: { at: 'env', env: d.environment_id, deploy: d.id } },
  }
}

const isRec = (v: Record<string, unknown>, ...keys: string[]) => keys.every((k) => typeof v[k] === 'string')

function CallCard({ host, fid, initial }: { host: Host; fid: string; initial: Record<string, unknown> }) {
  const kind =
    fid === 'iii-cloud::connect'
      ? 'connection'
      : fid === 'iii-cloud::check'
        ? 'check'
        : fid === 'iii-cloud::secret'
          ? 'secret'
          : 'deploy'
  const id = kind === 'connection' ? 'current' : String(initial.id)
  const record = useRecord<Record<string, unknown>>(host, kind, id, initial)
  const env = envSlug(
    useStatusSnapshot(host),
    typeof record.environment_id === 'string' ? record.environment_id : undefined,
  )
  const view =
    kind === 'connection'
      ? connectView(record as unknown as Connection)
      : kind === 'check'
        ? checkView(record as unknown as Check, env)
        : kind === 'secret'
          ? secretView(record as unknown as SecretRequest, env)
          : deployView(record as unknown as Deploy, fid, env)
  return <Shell host={host} view={view} />
}

function render(host: Host, m: FunctionCallMessage): ReactNode | null {
  const d = unwrapDetails(m.output)
  if (!d) return null
  const ok =
    m.functionId === 'iii-cloud::connect'
      ? typeof d.status === 'string' && 'cloud_url' in d
      : m.functionId === 'iii-cloud::check'
        ? isRec(d, 'id', 'commit')
        : m.functionId === 'iii-cloud::secret'
          ? isRec(d, 'id', 'name', 'state')
          : isRec(d, 'id', 'created_at') && Array.isArray(d.stages)
  return ok ? <CallCard host={host} fid={m.functionId} initial={d} /> : null
}

export function createCloudRenderer(host: Host): FunctionCallRenderer {
  return {
    id: 'iii-cloud/page.js#renderer',
    isMatch: (fid) => CALL_IDS.includes(fid),
    tryRender: (m) => render(host, m),
    tryRenderDisplay: (m) => render(host, m),
    metadata: { display: true },
  }
}
