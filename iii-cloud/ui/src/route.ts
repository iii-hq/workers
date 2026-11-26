// Where the iii cloud page is: one page id, three areas (setup, deploy, manage).
// The route travels as `panelContext.context` (chat cards, the palette, deep
// links) and is kept per pane with `usePaneState`. CloudPage owns the routing;
// every area gets `AreaProps` and calls `go` to move.

import type { Host } from '@iii-dev/console-ui'
import { needsAction, needsConnect, PAGE_ID, signInRequired } from './model'
import type { Status } from './types'

export type SetupStep = 'signin' | 'code' | 'target' | 'values' | 'check' | 'ship' | 'live'
export type DeployStep = 'what' | 'check' | 'review' | 'follow'
export type EnvTab = 'overview' | 'deploys' | 'settings' | 'checks'

/**
 * `env` names a cloud environment id. `''` means the selected one: old contexts carry no env, and
 * CloudPage fills it in before an area sees the route.
 */
export type Route =
  /** The projects on this machine: where the page opens once one is known, and the way to switch or add one. */
  | { at: 'start' }
  /**
   * `adding`: Start's "Add a project": the steps read `status.candidate`, and nothing is selected before
   * Continue. `newProject`: step 3 opens with the new cloud project form ("New project in iii cloud…").
   */
  | { at: 'setup'; step?: SetupStep; newProject?: boolean; adding?: boolean }
  | { at: 'deploy'; env: string; step?: DeployStep; deploy?: string }
  /** `note`: a line the home shows on arrival (a switch that took the default environment); never in a context. */
  | { at: 'home'; note?: string }
  | { at: 'env'; env: string; tab?: EnvTab; deploy?: string; check?: string }
  | { at: 'new-env'; branch?: string }

export type RouteAt<A extends Route['at']> = Extract<Route, { at: A }>

export const SETUP_STEPS: readonly SetupStep[] = ['signin', 'code', 'target', 'values', 'check', 'ship', 'live']
export const DEPLOY_STEPS: readonly DeployStep[] = ['what', 'check', 'review', 'follow']
export const ENV_TABS: readonly EnvTab[] = ['overview', 'deploys', 'settings', 'checks']

/**
 * What CloudPage hands every area. `paneKey` keys per-pane state (`usePaneState('iii-cloud:<what>:' + paneKey)`).
 * An env or deploy route may name an environment that is not selected yet: CloudPage selects it
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

/** An environment tab (`EnvOverview`, `EnvDeploys`, `EnvSettings`, `EnvChecks`): `envId` is the selected env. */
export type EnvTabProps = AreaProps<RouteAt<'env'>> & { envId: string }

const str = (v: unknown) => (typeof v === 'string' && v ? v : undefined)
const oneOf = <T extends string>(v: unknown, of: readonly T[]) => (of.includes(v as T) ? (v as T) : undefined)

/** A route out of a page context: a route as sent, or an old `{tab, deployId, pr}` context. Null: the default. */
export function routeFromContext(ctx: unknown): Route | null {
  if (!ctx || typeof ctx !== 'object') return null
  const c = ctx as Record<string, unknown>
  switch (c.at) {
    case 'start':
      return { at: 'start' }
    case 'setup':
      return {
        at: 'setup',
        step: oneOf(c.step, SETUP_STEPS),
        ...(c.newProject === true ? { newProject: true } : {}),
        ...(c.adding === true ? { adding: true } : {}),
      }
    case 'home':
      return { at: 'home' }
    case 'new-env':
      return { at: 'new-env', branch: str(c.branch) }
    case 'deploy':
      return { at: 'deploy', env: str(c.env) ?? '', step: oneOf(c.step, DEPLOY_STEPS), deploy: str(c.deploy) }
    case 'env':
      return {
        at: 'env',
        env: str(c.env) ?? '',
        tab: oneOf(c.tab, ENV_TABS),
        deploy: str(c.deploy),
        check: str(c.check),
      }
  }
  return legacyRoute(c)
}

/**
 * An old page context (`{tab, deployId, pr}`, from before the areas) on `env` ('' = the selected one).
 * `overview` and anything unknown are null: the caller's default.
 */
export function legacyRoute(c: { tab?: unknown; deployId?: unknown; pr?: unknown }, env = ''): Route | null {
  if (c.pr === true) return { at: 'deploy', env, step: 'review' }
  switch (c.tab) {
    case 'readiness':
    case 'run':
      return { at: 'env', env, tab: 'checks' }
    case 'secrets':
      return { at: 'env', env, tab: 'settings' }
    case 'deploys':
      return { at: 'env', env, tab: 'deploys', deploy: str(c.deployId) }
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
 * The path bar of a route (§2): its ancestors, each with where it goes, then the place itself. Back goes to
 * the last ancestor with a `to`: always the parent, never the history.
 */
export function pathOf(r: Route, s: Status): PathItem[] {
  const p = s.project
  const name = p?.cloud_project?.name ?? p?.repository ?? ''
  const projects: PathItem = { label: 'Projects', to: { at: 'start' } }
  const project: PathItem = { label: name, to: { at: 'home' } }
  const env = (id: string): PathItem => ({
    label: p?.environments.find((e) => e.id === id)?.name ?? '',
    to: { at: 'env', env: id },
  })
  switch (r.at) {
    case 'start':
      return [{ label: 'Projects' }]
    case 'home':
      return [projects, { label: name }]
    case 'env':
      return [projects, project, { label: env(r.env).label }]
    case 'new-env':
      return [projects, project, { label: 'New environment' }]
    case 'deploy':
      return [projects, project, env(r.env), { label: 'Deploy' }]
    case 'setup':
      if (r.adding) return [projects, { label: 'Add a project' }]
      if (p?.workspace) return [projects, project, { label: 'Set up' }]
      return [...(known(s) ? [projects] : []), { label: 'Set up' }]
  }
}

function derivedStep(s: Status): SetupStep {
  if (needsConnect(s) || signInRequired(s)) return 'signin'
  if (!s.project) return 'code'
  if (!s.project.workspace) return 'target'
  const latest = s.deploys.find((d) => d.environment_id === s.project?.environment?.id)
  if (latest?.outcome === 'ready') return 'live'
  if (latest?.outcome === 'running' || (latest && needsAction(latest)) || s.check?.ready) return 'ship'
  return 'check'
}

/**
 * The setup step the status is at; an `asked` step is honored only when it is that one or an earlier one. The
 * status never puts setup at `values`: the page goes there after "Where it runs", and it comes before `check`.
 */
export function setupStep(s: Status, asked?: SetupStep): SetupStep {
  const at = derivedStep(s)
  return asked && SETUP_STEPS.indexOf(asked) <= SETUP_STEPS.indexOf(at) ? asked : at
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

/** The console hash that opens the page at `r` (`#/worker/<scope>/<page>?context=<json>`). */
export const routeLink = (r: Route) => `#/worker/iii-cloud/${PAGE_ID}?context=${encodeURIComponent(JSON.stringify(r))}`
