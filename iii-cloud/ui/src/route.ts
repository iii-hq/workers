// Where the iii cloud page is: one page id, five places (projects, first time, check and deploy, an
// environment, its secrets and variables). The route travels as `panelContext.context` (chat cards, the
// palette, deep links) and is kept per pane with `usePaneState`. CloudPage owns the routing; every area gets
// `AreaProps` and calls `go` to move.

import type { Host } from '@iii-dev/console-ui'
import type { Status } from './types'

export type SetupStep = 'signin' | 'confirm'

/**
 * `env` names a cloud environment id. `''` means the selected one: old contexts carry no env, and
 * CloudPage fills it in before an area sees the route.
 */
export type Route =
  /** The projects on this machine: where the page opens once one is known, and the way to open or add one. */
  | { at: 'start' }
  /**
   * First time: sign in, then confirm what the page read (folder, project, environment, compose file).
   * `adding`: Start's "Add a project": the steps read `status.candidate`, and nothing is selected before
   * Continue. `newProject`: confirm opens with the new cloud project form.
   */
  | { at: 'setup'; step?: SetupStep; newProject?: boolean; adding?: boolean }
  /** The check and the deploy of an environment, one screen driven by state; `deploy` follows that deployment. */
  | { at: 'deploy'; env: string; deploy?: string }
  /** An environment; `deploy` opens that deployment's record. `note`: a line shown on arrival, never in a context. */
  | { at: 'env'; env: string; deploy?: string; note?: string }
  /** The environment's secrets and variables. */
  | { at: 'secrets'; env: string }

export type RouteAt<A extends Route['at']> = Extract<Route, { at: A }>

export const SETUP_STEPS: readonly SetupStep[] = ['signin', 'confirm']

/**
 * What CloudPage hands every area. `paneKey` keys per-pane state (`usePaneState('iii-cloud:<what>:' + paneKey)`).
 * An env, deploy or secrets route may name an environment that is not selected yet: CloudPage selects it
 * (`project::use`) and the area shows a Skeleton until `envState` says `selected` (see there).
 */
export type AreaProps<R extends Route = Route> = {
  host: Host
  status: Status
  route: R
  go: (r: Route) => void
  narrow: boolean
  paneKey: string
  /** The chat's working folder, for the setup folder picker. */
  workingDir?: string | null
}

/** What an environment's pages (`EnvOverview`, `EnvDeploys`, `EnvSettings`) get: `envId` is the selected env. */
export type EnvTabProps = AreaProps<RouteAt<'env' | 'secrets'>> & { envId: string }

const str = (v: unknown) => (typeof v === 'string' && v ? v : undefined)
const oneOf = <T extends string>(v: unknown, of: readonly T[]) => (of.includes(v as T) ? (v as T) : undefined)

/**
 * A route out of a page context: a route as sent, or an old context (a chat card or deep link made before
 * the redesign: `home`, an env `tab`, `new-env`, the seven setup steps, a deploy `step`). Null: the default.
 */
export function routeFromContext(ctx: unknown): Route | null {
  if (!ctx || typeof ctx !== 'object') return null
  const c = ctx as Record<string, unknown>
  switch (c.at) {
    case 'start':
      return { at: 'start' }
    case 'setup': {
      // The old steps after Your code and Where it runs are the check and the deploy.
      if (['values', 'check', 'ship', 'live'].includes(c.step as string)) return { at: 'deploy', env: '' }
      return {
        at: 'setup',
        step: c.step === 'code' || c.step === 'target' ? 'confirm' : oneOf(c.step, SETUP_STEPS),
        ...(c.newProject === true ? { newProject: true } : {}),
        ...(c.adding === true ? { adding: true } : {}),
      }
    }
    case 'home':
    case 'new-env':
      return { at: 'env', env: '' }
    case 'deploy':
      return { at: 'deploy', env: str(c.env) ?? '', deploy: str(c.deploy) }
    case 'secrets':
      return { at: 'secrets', env: str(c.env) ?? '' }
    case 'env':
      if (c.tab === 'settings') return { at: 'secrets', env: str(c.env) ?? '' }
      return { at: 'env', env: str(c.env) ?? '', deploy: str(c.deploy) }
  }
  return legacyRoute(c)
}

/**
 * An older page context (`{tab, deployId, pr}`, from before the areas) on `env` ('' = the selected one).
 * `overview` and anything unknown are null: the caller's default.
 */
export function legacyRoute(c: { tab?: unknown; deployId?: unknown; pr?: unknown }, env = ''): Route | null {
  if (c.pr === true) return { at: 'deploy', env }
  switch (c.tab) {
    case 'readiness':
    case 'run':
      return { at: 'env', env }
    case 'secrets':
      return { at: 'secrets', env }
    case 'deploys':
      return { at: 'env', env, deploy: str(c.deployId) }
  }
  return null
}

/** A project is known on this machine: one in use, or one used before. */
export const known = (s: Status) => !!s.project?.workspace || (s.recent_projects?.length ?? 0) > 0

/** Where the page opens without a route: the projects once one is known, signed in or not; else setup. */
export function defaultRoute(s: Status): Route {
  return known(s) ? { at: 'start' } : { at: 'setup' }
}

export type PathItem = { label: string; to?: Route }

/**
 * The path bar of a route: its ancestors, each with where it goes, then the place itself (`Projects`,
 * `Projects / registry · dev`, `… / Deploy` (on a phone `Deploy to dev`), `… / Deployment #6`, `… / Secrets and variables`,
 * `Projects / Add a project`). Back goes to the last ancestor with a `to`: always the parent, never the history.
 */
export function pathOf(r: Route, s: Status, narrow = false): PathItem[] {
  const p = s.project
  const projects: PathItem = { label: 'Projects', to: { at: 'start' } }
  const label = (id: string) => {
    const env = p?.environments.find((e) => e.id === id)?.name
    const project = p?.cloud_project?.name ?? p?.repository
    return [project, env].filter(Boolean).join(' · ')
  }
  const env = (id: string): PathItem => ({ label: label(id), to: { at: 'env', env: id } })
  switch (r.at) {
    case 'start':
      return [{ label: 'Projects' }]
    case 'env': {
      if (!r.deploy) return [projects, { label: label(r.env) }]
      const n = s.deploys.find((d) => d.id === r.deploy)?.number
      return [projects, env(r.env), { label: n ? `Deployment #${n}` : 'Deployment' }]
    }
    case 'deploy': {
      // A phone's bar names only the place, so it carries the environment: "Deploy to dev".
      const name = p?.environments.find((e) => e.id === r.env)?.name
      return [projects, env(r.env), { label: narrow && name ? `Deploy to ${name}` : 'Deploy' }]
    }
    case 'secrets':
      return [projects, env(r.env), { label: 'Secrets and variables' }]
    case 'setup':
      return known(s) || r.adding
        ? [projects, { label: 'Add a project' }]
        : [{ label: 'Put this project on iii cloud' }]
  }
}

/**
 * Where an env route stands: the selected environment, one of the project's that CloudPage is
 * selecting, or none of this project's (show an EmptyState).
 */
export function envState(s: Status, envId: string): 'selected' | 'switching' | 'unknown' {
  const p = s.project
  if (p?.environment?.id === envId) return 'selected'
  return p?.environments.some((e) => e.id === envId) ? 'switching' : 'unknown'
}
