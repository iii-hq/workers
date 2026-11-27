import type { Host } from '@iii-dev/console-ui'
import { EmptyState } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { useCloud } from './api'
import { type DeploymentList, StateChip } from './EnvOverview'
import { FailureCard, failureTitle, stoppedAt } from './FailureCard'
import { deployOrigin, deployState, isFailedDeploy, needsAction, envPageUrl, servingId, sha7, when } from './model'
import { Mono, SectionCard } from './parts'
import type { Route } from './route'
import type { Deploy, Members, Status } from './types'

const message = (d: Deploy) => d.commit.message?.split('\n')[0]

/** What a deployment is called in the list: the commit's message, a failed one its cause, else its short sha (the number is the first column). */
function titleOf(d: Deploy): string {
  if (isFailedDeploy(d)) return failureTitle(d)
  return message(d) ?? sha7(d.commit.sha)
}

/**
 * "your agent fixed 2" where the worker's records know the deployment came from a check its agent worked on:
 * the check carries the hand-off (`handoff_id`), and the count is the hand-off's own while it is the one on file.
 */
function agentNote(s: Status, d: Deploy): string | null {
  const check = d.check_id ? s.checks?.find((c) => c.id === d.check_id) : undefined
  if (!check?.handoff_id) return null
  const brief = s.handoff_brief?.handoff_id === check.handoff_id ? s.handoff_brief : null
  const n = brief?.agent_problems.length ?? 0
  return n ? `your agent fixed ${n}` : "with your agent's fixes"
}

/** The one line under a deployment: when, who and from where, whether it was checked, and what the records add. */
export function deployMeta(s: Status, d: Deploy, cloud: DeploymentList | null, members: Members | null): string {
  const o = deployOrigin(
    s,
    d,
    cloud?.deployments.find((x) => x.id === d.id),
    members,
  )
  const note = agentNote(s, d)
  const replaced =
    d.outcome === 'canceled' && d.replaced_by
      ? `#${s.deploys.find((x) => x.id === d.replaced_by)?.number ?? sha7(d.replaced_by)} replaced it before approval`
      : null
  return [`${when(d.created_at)} · ${o.line}${note && o.checked ? `, ${note}` : ''}`, replaced]
    .filter(Boolean)
    .join(' · ')
}

/** The members of the team, read only when someone else started one of the deployments. */
export function useMembers(host: Host, status: Status, cloud: DeploymentList | null) {
  const me = status.connection.user?.id
  const other = !!cloud?.deployments.some((d) => d.triggered_by.user_id && d.triggered_by.user_id !== me)
  return useCloud<Members>(host, status, null, other ? '/members' : null).data
}

const SHOWN = 10

/** Deployments: one row per deployment, as every other place reads it; a failed one opens its record. */
export function Deployments({
  host,
  status,
  envId,
  cloud,
  go,
}: {
  host: Host
  status: Status
  envId: string
  cloud: DeploymentList | null
  go: (r: Route) => void
}) {
  const deploys = status.deploys.filter((d) => d.environment_id === envId).slice(0, SHOWN)
  const members = useMembers(host, status, cloud)
  const live = servingId(status, envId, cloud?.deployments)
  const envName = status.project?.environments.find((e) => e.id === envId)?.name
  const env = envName ?? 'this environment'
  const url = envPageUrl(status, 'deployments', envName)
  return (
    <SectionCard
      title="Deployments"
      flush
      end={
        url ? (
          <a className="iii-cloud-link" href={url} target="_blank" rel="noreferrer">
            All deployments and logs in iii cloud
          </a>
        ) : undefined
      }
    >
      {deploys.length === 0 ? (
        <EmptyState
          compact
          title={`No deployments of ${env} yet`}
          description="The first one appears in this list as soon as it starts."
        />
      ) : (
        <ul className="iii-cloud-deps">
          {deploys.map((d) => (
            <li key={d.id} className="iii-cloud-dep">
              <span className="iii-cloud-mono iii-cloud-dnum">#{d.number}</span>
              <StateChip live={deployState(d, live)} />
              <span className="iii-cloud-dep-text">
                <span className="iii-cloud-dep-title">{titleOf(d)}</span>
                <span className="iii-cloud-desc">{deployMeta(status, d, cloud, members)}</span>
              </span>
              {isFailedDeploy(d) || needsAction(d) ? (
                <button
                  type="button"
                  className="iii-cloud-linkbtn"
                  aria-label={`Open deployment #${d.number}`}
                  // A deployment iii cloud holds waits for the person's Confirm, which is on the Deploy screen.
                  onClick={() =>
                    go(
                      needsAction(d)
                        ? { at: 'deploy', env: envId, deploy: d.id }
                        : { at: 'env', env: envId, deploy: d.id },
                    )
                  }
                >
                  Open
                </button>
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </SectionCard>
  )
}

/** The headline of a deployment that did not fail. */
function headline(d: Deploy, live: string | null): string {
  const said = message(d)
  if (said) return said
  const how =
    d.id === live
      ? 'is live'
      : d.status === 'gone'
        ? 'no longer exists'
        : needsAction(d)
          ? 'needs your action'
          : d.outcome === 'running'
            ? 'is deploying'
            : d.outcome === 'canceled'
              ? 'was canceled'
              : 'was replaced'
  return `Deployment #${d.number} ${how}`
}

/** A deployment's record: the facts every other place tells the same way, and for a failed one where it stopped. */
export function DeploymentRecord({
  host,
  status,
  envId,
  deployId,
  narrow,
  go,
}: {
  host: Host
  status: Status
  envId: string
  deployId: string
  narrow: boolean
  go: (r: Route) => void
}) {
  const feed = useCloud<DeploymentList>(host, status, envId, '/deployments').data
  const members = useMembers(host, status, feed)
  const d = status.deploys.find((x) => x.id === deployId && x.environment_id === envId)
  const env = status.project?.environments.find((e) => e.id === envId)?.name ?? 'this environment'
  if (!d) {
    return (
      <EmptyState
        title="Deployment not found"
        description={`It is not among the latest 20 deployments of ${env}.`}
        action={{ label: `Back to ${env}`, onClick: () => go({ at: 'env', env: envId }) }}
      />
    )
  }
  const live = servingId(status, envId, feed?.deployments)
  const servingNumber = live
    ? (status.deploys.find((x) => x.id === live)?.number ??
      feed?.deployments.find((x) => x.id === live)?.number ??
      null)
    : null
  const o = deployOrigin(
    status,
    d,
    feed?.deployments.find((x) => x.id === d.id),
    members,
  )
  const failed = isFailedDeploy(d)
  const rows: [string, ReactNode][] = [
    [
      'Commit',
      <>
        <Mono>{sha7(d.commit.sha)}</Mono> {message(d) ?? ''}
      </>,
    ],
    ['Started', `${when(d.created_at)} · ${o.origin}`],
    ['Checked first', o.checked ? 'Yes, before it was deployed.' : 'No.'],
    ...(narrow && failed ? ([['Stopped at', stoppedAt(d)]] as [string, string][]) : []),
  ]
  const cells = (
    <dl className="iii-cloud-cells">
      {rows.map(([k, v]) => (
        <div key={k} className="iii-cloud-cell">
          <dt className="iii-cloud-cell-label">{k}</dt>
          <dd className="iii-cloud-cell-value">{v}</dd>
        </div>
      ))}
    </dl>
  )
  if (failed) {
    return (
      <FailureCard host={host} status={status} deploy={d} servingNumber={servingNumber} narrow={narrow} go={go}>
        {cells}
      </FailureCard>
    )
  }
  return (
    <div className="iii-cloud-failure">
      <div className="iii-cloud-rec-head">
        <div className="iii-cloud-rec-title">
          <h2 className="iii-cloud-h">{headline(d, live)}</h2>
          <p className="iii-cloud-desc">{`Deployment #${d.number} of ${env}`}</p>
        </div>
      </div>
      {cells}
      <p className="iii-cloud-desc">
        <a className="iii-cloud-link" href={d.console_url} target="_blank" rel="noreferrer">
          Full log in iii cloud
        </a>
      </p>
    </div>
  )
}
