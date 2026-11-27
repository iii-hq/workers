import type { Host } from '@iii-dev/console-ui'
import { Button } from '@iii-dev/console-ui'
import { type ReactNode, useEffect, useRef } from 'react'
import { useLaunch } from './asked'
import { handOff, openAgentChat } from './handoff'
import { activeSameKind, failedStage, failureWords, sendLabel, servingDeploy, stageRows } from './model'
import { Mono, SectionCard } from './parts'
import type { Route } from './route'
import type { Deploy, Status } from './types'

/** The failure in one plain sentence: the worker's title, else the stage and what happened there. */
export function failureTitle(d: Deploy): string {
  if (d.failure?.title) return d.failure.title
  return `The deploy failed at ${failedStage(d)}${d.error ? `: ${failureWords(d.error)}` : ''}`
}

/** "Deployment #7 failed: <cause>": the failure's line where the deployment is named first. */
export function failedLine(d: Deploy): string {
  if (d.failure?.title) return `Deployment #${d.number} failed: ${d.failure.title}`
  return `Deployment #${d.number} failed at ${failedStage(d)}${d.error ? `: ${failureWords(d.error)}` : ''}`
}

/** The failing stage's last lines, as the worker kept them. */
export function failureTail(d: Deploy): string[] {
  return d.failure ? d.failure.log_tail : d.log_tail
}

const ORDINAL = ['first', 'second', 'third', 'fourth', 'fifth', 'sixth', 'seventh']
const COUNT = ['none', 'one', 'two', 'three', 'four', 'five', 'six', 'seven']

/** "Plan ✓ · Build ✓ · Database and secrets ✕ · Workers not run · Address not run": where the deployment stopped. */
export function stageLine(d: Deploy): string {
  return stageRows(d)
    .map(
      (r) =>
        `${r.short}${r.tone === 'ok' ? ' ✓' : r.tone === 'alert' ? ' ✕' : r.tone === 'live' ? ' running' : ' not run'}`,
    )
    .join(' · ')
}

/** "Database and secrets, the third of five stages": the stage a failed deployment stopped at, for the narrow record. */
export function stoppedAt(d: Deploy): string {
  const rows = stageRows(d)
  const at = rows.findIndex((r) => r.tone === 'alert')
  if (at < 0) return failedStage(d)
  return `${rows[at].short}, the ${ORDINAL[at] ?? `number ${at + 1}`} of ${COUNT[rows.length] ?? rows.length} stages`
}

/** Log lines in the shared pre, scrolled to the end, where the error is. */
export function LogLines({ lines }: { lines: string[] }) {
  const ref = useRef<HTMLPreElement>(null)
  const text = lines.join('\n')
  useEffect(() => {
    const el = ref.current
    if (el) el.scrollTop = el.scrollHeight
  }, [text])
  return (
    <pre className="iii-cloud-log" ref={ref}>
      {text}
    </pre>
  )
}

/**
 * A failed deployment's record: the cause as the title with "Fix with your agent", `children` (the facts, the
 * record's three cells) under it, then "Where it stopped" with the stage line, the explanation and the last
 * log lines. "Fix with your agent" hands the failure to the agent, then goes to the Deploy place (`go`).
 * On a narrow pane the action is a bar at the bottom.
 */
export function FailureCard({
  host,
  status,
  deploy,
  servingNumber,
  narrow = false,
  go,
  children,
}: {
  host: Host
  status: Status
  deploy: Deploy
  /** The number of the deployment that still serves; undefined: the worker's newest ready one. */
  servingNumber?: number | null
  narrow?: boolean
  go?: (r: Route) => void
  children?: ReactNode
}) {
  const f = deploy.failure
  const launching = useLaunch()?.kind === 'launching'
  const working = activeSameKind(status, 'deploy_fix')
  const hint = f ? f.hint : deploy.error?.hint
  const env = status.project?.environments.find((e) => e.id === deploy.environment_id)?.name ?? 'the environment'
  const serving =
    servingNumber === undefined ? (servingDeploy(status, deploy.environment_id)?.number ?? null) : servingNumber
  const lines = failureTail(deploy)
  const act = working ? (
    <Button variant="secondary" onClick={() => openAgentChat(host, status)}>
      Open your agent's chat
    </Button>
  ) : (
    <Button
      variant="primary"
      disabled={launching || !host.chat?.compose}
      onClick={() => {
        void handOff(host, status, 'deploy_fix', deploy).then((ok) => {
          if (ok) go?.({ at: 'deploy', env: deploy.environment_id, deploy: deploy.id })
        })
      }}
    >
      {sendLabel(status, 'deploy_fix', 'Fix with your agent')}
    </Button>
  )
  const why =
    'Your agent gets these log lines and works in a separate copy. Its fix goes through the check, and you confirm the deploy.'
  return (
    <div className="iii-cloud-failure">
      <div className="iii-cloud-rec-head">
        <div className="iii-cloud-rec-title">
          <h2 className="iii-cloud-h">{failureTitle(deploy)}</h2>
          <p className="iii-cloud-desc">
            {serving
              ? `${env} still serves #${serving}. Nothing changed for your users.`
              : `Nothing was live on ${env} then, so nobody saw an error.`}
          </p>
        </div>
        {narrow ? null : act}
      </div>
      {children}
      <SectionCard title="Where it stopped" note={stageLine(deploy)} flush>
        <div className="iii-cloud-stopped">
          {f?.silent ? <p className="iii-cloud-text">The command exited without printing an error.</p> : null}
          {hint ? <p className="iii-cloud-text">{hint}</p> : null}
          {f?.release_command ? (
            <p className="iii-cloud-text">
              Release command: <Mono>{f.release_command}</Mono>
            </p>
          ) : null}
          {lines.length ? <LogLines lines={lines} /> : null}
        </div>
        <div className="iii-cloud-stopped-foot">
          {narrow ? null : <span className="iii-cloud-desc">{why}</span>}
          <a className="iii-cloud-link iii-cloud-small" href={deploy.console_url} target="_blank" rel="noreferrer">
            Full log in iii cloud
          </a>
        </div>
      </SectionCard>
      {narrow ? (
        <div className="iii-cloud-recbar">
          {act}
          <span className="iii-cloud-desc">{why}</span>
        </div>
      ) : null}
    </div>
  )
}
