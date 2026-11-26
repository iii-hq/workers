import { Button, EmptyState, type Host, Panel, PanelBody, PanelHeader, Skeleton } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { useCloud } from './api'
import { NewEnvironmentForm } from './CreateForms'
import { hubOf } from './hub'
import { plainMessage, roleOf, sizeOf } from './model'
import { Facts, Notice, PathBar } from './parts'
import { type AreaProps, pathOf, type RouteAt } from './route'
import type { Status } from './types'

/** The parts of `GET …/machines` and `GET …/resources` this page reads (cloud-v1 `Machines`, `ResourceList`). */
type Machines = { groups: { name: string; size: string; replicas: { min: number; max: number } }[] }
type Resources = { resources: { id: string; kind: string; alias: string; status: string; tier?: string | null }[] }

const KIND: Record<string, [label: string, noun: string]> = {
  database: ['Database', 'Postgres database'],
  storage: ['Storage', 'storage bucket'],
  queue: ['Queue', 'RabbitMQ queue'],
  state: ['State', 'Redis store'],
}

const replicas = ({ min, max }: { min: number; max: number }) => (min === max ? `${min}` : `${min}–${max}`)

/** The rows of "What iii cloud will create": the source's resources by kind, then its machine groups. */
function planRows(from: string, m: Machines, r: Resources): [string, ReactNode][] {
  const live = r.resources.filter((x) => x.status !== 'deleting' && x.status !== 'deleted')
  const kinds = [...new Set(live.map((x) => x.kind))]
  const rows: [string, ReactNode][] = kinds.map((k) => [
    KIND[k]?.[0] ?? k,
    live
      .filter((x) => x.kind === k)
      .map((x) => `${x.alias}: a new ${KIND[k]?.[1] ?? k}${x.tier ? `, ${x.tier}` : ''}`)
      .join(' · '),
  ])
  if (kinds.length === 0) rows.push(['Resources', `none in ${from}`])
  rows.push([
    'Machines',
    m.groups.length
      ? m.groups.map((g) => `${g.name} ${replicas(g.replicas)} × ${sizeOf(g.size)}`).join(' · ')
      : `none in ${from} yet: the first deploy plans them from the app file`,
  ])
  return rows
}

/** What iii cloud makes for the new environment: the machines and resources of the one it copies. */
function WillCreate({ host, status, from }: { host: Host; status: Status; from: { id: string; name: string } | null }) {
  const machines = useCloud<Machines>(host, status, from?.id ?? null, from ? '/machines' : null)
  const resources = useCloud<Resources>(host, status, from?.id ?? null, from ? '/resources' : null)
  const error = machines.error ?? resources.error
  let body: ReactNode
  if (!from) {
    body = (
      <p className="iii-cloud-desc">
        Nothing is copied: no settings, secrets or machine sizes. The first deploy plans its machines and resources from
        the app file.
      </p>
    )
  } else if (error) {
    body = (
      <Notice
        variant="warn"
        headline={`Can't read what ${from.name} runs`}
        detail={plainMessage(error)}
        action={
          <Button
            size="sm"
            variant="secondary"
            onClick={() => {
              machines.refetch()
              resources.refetch()
            }}
          >
            Try again
          </Button>
        }
      />
    )
  } else if (!machines.data || !resources.data) {
    body = <Skeleton className="iii-cloud-ne-skeleton" />
  } else {
    body = <Facts rows={planRows(from.name, machines.data, resources.data)} />
  }
  return (
    <Panel role="region" aria-label="What iii cloud will create" className="iii-cloud-ne-plan">
      <PanelHeader className="iii-cloud-panel-head">
        <span>What iii cloud will create</span>
        {from ? <span className="iii-cloud-panel-end">as in {from.name}</span> : null}
      </PanelHeader>
      <PanelBody>{body}</PanelBody>
    </Panel>
  )
}

/** A new environment of the project (Mng8); once created it is selected, and its deploy steps open. */
export function NewEnv({ host, status, route, go, narrow }: AreaProps<RouteAt<'new-env'>>) {
  const p = status.project
  const cp = p?.cloud_project
  if (!p || !cp) {
    return (
      <div className="iii-cloud-body">
        <EmptyState title="No project yet" description="Set up a project first: environments belong to one." />
      </div>
    )
  }
  const project = p.cloud_projects?.find((c) => c.id === cp.id) ?? { ...cp, environments: p.environments }
  const envs = project.environments ?? p.environments
  // A hint only: iii cloud enforces the role, and the form shows its answer.
  const role = roleOf(status)
  const admin = !role || /^(owner|admin)$/i.test(role)
  return (
    <>
      <PathBar path={pathOf(route, status)} host={host} status={status} route={route} go={go} narrow={narrow} rule />
      <div className="iii-cloud-body iii-cloud-scroll">
        <div className="iii-cloud-ne">
          {admin ? null : (
            <Notice
              variant="warn"
              headline="Owners and admins create environments"
              detail={`Your role on team ${p.team?.name ?? cp.name} is ${role}. You can still try: iii cloud decides, and this page shows its answer.`}
            />
          )}
          <NewEnvironmentForm
            host={host}
            project={project}
            defaultBranch={route.branch ?? p.checkout.branch ?? p.default_branch ?? 'main'}
            baseBranch={p.default_branch}
            baseManifest={p.environment?.manifest}
            copyFrom={envs.filter((e) => !('deleted' in e && e.deleted)).map((e) => ({ id: e.id, name: e.name }))}
            preview={(from) => <WillCreate host={host} status={status} from={from} />}
            onCancel={() => go({ at: 'home' })}
            onCreated={(out, name) => {
              // The worker selects the new environment: read the status first so the route finds it.
              const id =
                out.environment_id ??
                out.cloud_projects
                  ?.find((c) => c.id === (out.project_id ?? cp.id))
                  ?.environments?.find((e) => e.name === name)?.id
              void hubOf(host)
                .refresh(false)
                .then(() => go(id ? { at: 'deploy', env: id } : { at: 'home' }))
            }}
          />
        </div>
      </div>
    </>
  )
}
