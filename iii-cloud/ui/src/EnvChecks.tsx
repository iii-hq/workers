import { Button, EmptyState, IconButton, List, ListItem, Skeleton, Toolbar } from '@iii-dev/console-ui'
import { errorCode, errorMessage } from '@iii-dev/console-ui/format'
import { ArrowLeft, ChevronRight, CircleAlert, CircleCheck, ShieldCheck } from 'lucide-react'
import { type ReactNode, useEffect, useState } from 'react'
import { clock } from './asked'
import { CheckStage } from './CheckStage'
import { Section, when } from './EnvOverview'
import { useChecking } from './hub'
import { openProblems, plural, servingDeploy, sha7, workerError } from './model'
import { ErrorPanel, Mono, Spinner } from './parts'
import type { EnvTabProps } from './route'
import type { Check, CheckSummary, Status, WorkerError } from './types'

const ACTIVE = ['starting', 'working', 'waiting_for_you', 'waiting']

/**
 * An environment's Checks tab (Mng4): where the current check stands, the check itself (CheckStage, the
 * run here included), and the earlier ones by hand-off. `route.check` opens one: the current one live,
 * an earlier one read-only as `iii-cloud::check::get` returns it.
 */
export function EnvChecks(props: EnvTabProps) {
  const { host, status, route, go, envId } = props
  const checking = useChecking(host)
  if (route.check) return <OpenedCheck {...props} id={route.check} />
  const env = status.project?.environment?.name ?? 'this environment'
  const now = nowLine(status, envId, checking)
  const groups = groupsOf(status, envId).filter((g) => !g.checks.some((c) => c.id === status.check?.id))
  // The check is opened in Deploy from this machine: its review when it passed on what isn't served yet.
  const c = status.check
  const waiting =
    !!c?.ready &&
    servingDeploy(status, envId)?.commit.sha !== c.commit &&
    !status.deploys.some((d) => d.environment_id === envId && d.outcome === 'running')
  return (
    <>
      <Section title="Now">
        <div className="iii-cloud-now">
          {now.mark}
          <span className="iii-cloud-grow iii-cloud-two">
            <span className="iii-cloud-ptitle">{now.title}</span>
            <span className="iii-cloud-psmall">{now.sub}</span>
          </span>
          {c ? (
            <Button
              variant={waiting ? 'primary' : 'secondary'}
              onClick={() => go({ at: 'deploy', env: envId, step: waiting ? 'review' : 'check' })}
            >
              {waiting ? 'Review and deploy' : 'Open the check'}
            </Button>
          ) : null}
        </div>
      </Section>
      {groups.length ? (
        <Section title={`Earlier on ${env}`} flush>
          <List aria-label={`Earlier checks of ${env}`} className="iii-cloud-earlier">
            {groups.map((g) => {
              const r = groupRow(status, envId, g)
              return (
                <ListItem
                  key={g.key}
                  leading={
                    r.ready ? (
                      <CircleCheck className="iii-cloud-mark-ok" />
                    ) : (
                      <CircleAlert className="iii-cloud-mark-warn" />
                    )
                  }
                  label={r.title}
                  description={r.facts}
                  trailing={
                    <span className="iii-cloud-row">
                      <span className="iii-cloud-psmall">{when(g.checks[0].checked_at)}</span>
                      <ChevronRight />
                    </span>
                  }
                  onClick={() => go({ at: 'env', env: envId, tab: 'checks', check: g.checks[0].id })}
                />
              )
            })}
          </List>
        </Section>
      ) : (
        <Section title={`Earlier on ${env}`}>
          <EmptyState
            compact
            title={`No earlier checks of ${env}`}
            description="Each check this machine runs is kept here, the latest 20."
          />
        </Section>
      )}
      {groups.length ? (
        <p className="iii-cloud-desc">
          Each item opens its check, with your agent's chat when it worked on it. Checks and their history are kept on
          this machine.
        </p>
      ) : null}
    </>
  )
}

/** The Now line: what the current check is doing, and whether the environment serves it. */
function nowLine(s: Status, envId: string, checking: boolean): { mark: ReactNode; title: ReactNode; sub: string } {
  const c = s.check
  const env = s.project?.environment?.name ?? 'this environment'
  if (!c) {
    return {
      mark: <ShieldCheck className="iii-cloud-faint" />,
      title: `No check of ${env} yet`,
      sub: 'Deploy from this machine checks your code on iii cloud and here before it deploys.',
    }
  }
  const sha = <Mono>{sha7(c.commit)}</Mono>
  const at = clock(c.checked_at)
  if (checking || c.run?.status === 'running') {
    return { mark: <Spinner />, title: 'Checking', sub: 'On iii cloud and here. Nothing is deployed while it runs.' }
  }
  const a = s.agent
  const n = openProblems(s).length
  if (a && ACTIVE.includes(a.state) && ['ready', 'deploy_fix'].includes(a.kind ?? 'ready')) {
    return {
      mark: <Spinner />,
      title: (
        <>
          Your agent is fixing {plural(n, 'problem', 'problems')} in {sha}
        </>
      ),
      sub: a.note || 'Nothing is deployed while it works.',
    }
  }
  if (!c.ready) {
    return {
      mark: <CircleAlert className="iii-cloud-mark-warn" />,
      title: (
        <>
          {plural(n, 'problem', 'problems')} in {sha}
        </>
      ),
      sub: `Checked at ${at}`,
    }
  }
  const ok = <CircleCheck className="iii-cloud-mark-ok" />
  const passed = `Check passed on iii cloud and here at ${at}`
  if (servingDeploy(s, envId)?.commit.sha === c.commit) {
    return {
      mark: ok,
      title: (
        <>
          {sha} is live on {env}
        </>
      ),
      sub: passed,
    }
  }
  if (s.deploys.some((d) => d.environment_id === envId && d.outcome === 'running')) {
    return { mark: ok, title: <>{sha} passed</>, sub: `${passed} · a deploy of ${env} is running` }
  }
  return {
    mark: ok,
    title: (
      <>
        Deploy {sha} to {env}
      </>
    ),
    sub: `${passed}${c.handoff_id ? '' : ' · no agent work needed'} · waiting for you`,
  }
}

type Group = { key: string; handoff: string | null; checks: CheckSummary[] }

/** The environment's kept checks (newest first), one group per run of checks with the same hand-off. */
function groupsOf(s: Status, envId: string): Group[] {
  const out: Group[] = []
  for (const c of s.checks ?? []) {
    if (c.environment_id !== envId) continue
    const last = out.at(-1)
    if (last && last.handoff === c.handoff_id) last.checks.push(c)
    else out.push({ key: c.id, handoff: c.handoff_id, checks: [c] })
  }
  return out
}

/** A group's title (by its hand-off's kind, when this page still knows it) and its counts. */
function groupRow(s: Status, envId: string, g: Group): { title: string; facts: string; ready: boolean } {
  const newest = g.checks[0]
  const oldest = g.checks[g.checks.length - 1]
  const brief = g.handoff && s.handoff_brief?.handoff_id === g.handoff ? s.handoff_brief : null
  const kind = brief?.kind ?? (g.handoff && s.agent?.handoff_id === g.handoff ? s.agent.kind : undefined)
  const fixing = kind === 'deploy_fix' ? s.deploys.find((d) => d.id === brief?.deploy?.id) : undefined
  const title =
    kind === 'deploy_fix'
      ? fixing
        ? `Fix deploy #${fixing.number}`
        : 'Fix a failed deploy'
      : kind === 'import'
        ? `Set up ${s.project?.cloud_project?.name ?? s.project?.repository ?? 'the project'}`
        : `Deploy ${sha7(newest.commit)}`
  const from = Date.parse(oldest.checked_at)
  const to = Date.parse(newest.checked_at)
  const saved = s.secrets.filter(
    (r) =>
      r.environment_id === envId &&
      r.state === 'set' &&
      !!r.resolved_at &&
      Date.parse(r.resolved_at) >= from &&
      Date.parse(r.resolved_at) <= to,
  ).length
  const fixed = g.handoff ? oldest.problems - newest.problems : 0
  const ids = new Set(g.checks.map((c) => c.id))
  const deployed = s.deploys.find((d) => !!d.check_id && ids.has(d.check_id))
  const facts = [
    plural(g.checks.length, 'check', 'checks'),
    fixed > 0 ? `your agent fixed ${plural(fixed, 'problem', 'problems')}` : null,
    saved > 0 ? `you saved ${plural(saved, 'secret', 'secrets')}` : null,
    deployed ? `deployed as #${deployed.number}` : null,
  ]
  return { title, facts: facts.filter(Boolean).join(' · '), ready: newest.ready }
}

/**
 * An earlier check as it was recorded: CheckStage reads it from `status.check`, and gets its agent only
 * when that check's hand-off is the one still open (an older chat is not known here).
 */
function asOf(s: Status, c: Check): Status {
  const ours = !!c.handoff_id && c.handoff_id === s.agent?.handoff_id
  return { ...s, check: c, ...(ours ? {} : { agent: null, handoff_brief: null }) }
}

function OpenedCheck({ host, status, go, narrow, envId, id }: EnvTabProps & { id: string }) {
  const current = status.check?.id === id
  const [got, setGot] = useState<{ id: string; check?: Check; error?: WorkerError } | null>(null)
  useEffect(() => {
    if (current) return
    let alive = true
    host.iii.call<Check>('iii-cloud::check::get', { id }).then(
      (check) => {
        if (alive) setGot({ id, check })
      },
      (err) => {
        if (alive) setGot({ id, error: workerError(err, errorCode(err), errorMessage(err)) })
      },
    )
    return () => {
      alive = false
    }
  }, [host, id, current])
  const back = () => go({ at: 'env', env: envId, tab: 'checks' })
  const mine = got?.id === id ? got : null
  const record = current ? status.check : mine?.check
  const commit = record?.commit ?? status.checks?.find((c) => c.id === id)?.commit
  return (
    <>
      <Toolbar
        aria-label="Check"
        className="iii-cloud-crumb"
        end={record ? <span className="iii-cloud-desc">checked at {clock(record.checked_at)}</span> : undefined}
      >
        <IconButton label="Back to Checks" variant="ghost" onClick={back}>
          <ArrowLeft />
        </IconButton>
        <h3 className="iii-cloud-head-title">{commit ? `Check ${sha7(commit)}` : 'Check'}</h3>
      </Toolbar>
      {current ? (
        <CheckStage host={host} status={status} scenario="change" go={go} narrow={narrow} />
      ) : mine?.check ? (
        <CheckStage
          host={host}
          status={asOf(status, mine.check)}
          scenario="change"
          go={go}
          narrow={narrow}
          checkId={id}
        />
      ) : mine?.error ? (
        <ErrorPanel
          error={mine.error}
          action={
            <Button size="sm" variant="secondary" onClick={back}>
              All checks
            </Button>
          }
        />
      ) : (
        <Skeleton className="iii-cloud-skeleton" />
      )}
    </>
  )
}
