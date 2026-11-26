// The route: old contexts, the default, the derived setup step, deep links (U1; frozen with route.ts).
import { describe, expect, it } from 'vitest'
import { boards } from '../fixtures'
import { base, ENV, following, ready, signedOut } from '../fixtures/base'
import {
  defaultRoute,
  envState,
  legacyRoute,
  pathOf,
  type Route,
  routeFromContext,
  routeLink,
  SETUP_STEPS,
  setupStep,
} from '../route'
import type { RecentProject, Status } from '../types'

const main = base
const signin = boards['setup-signin'].status
const target = boards['setup-target'].status
const live = (s: Status): Status => ({ ...s, deploys: [{ ...following, outcome: 'ready', status: 'ready' }] })

describe('routeFromContext', () => {
  it('maps every old context; overview and the palette open the default', () => {
    expect(routeFromContext({ tab: 'readiness' })).toEqual({ at: 'env', env: '', tab: 'checks' })
    expect(routeFromContext({ tab: 'run' })).toEqual({ at: 'env', env: '', tab: 'checks' })
    expect(routeFromContext({ tab: 'secrets' })).toEqual({ at: 'env', env: '', tab: 'settings' })
    expect(routeFromContext({ tab: 'deploys' })).toEqual({ at: 'env', env: '', tab: 'deploys' })
    expect(routeFromContext({ tab: 'deploys', deployId: 'dep-1' })).toEqual({
      at: 'env',
      env: '',
      tab: 'deploys',
      deploy: 'dep-1',
    })
    expect(routeFromContext({ tab: 'deploys', pr: true })).toEqual({ at: 'deploy', env: '', step: 'review' })
    expect(routeFromContext({ tab: 'overview' })).toBeNull()
    expect(routeFromContext({})).toBeNull()
    for (const junk of [null, undefined, 'env', 3, [], { at: 'nowhere' }]) expect(routeFromContext(junk)).toBeNull()
  })

  it('takes a route as sent, dropping what is not one', () => {
    const r: Route = { at: 'env', env: ENV.id, tab: 'deploys', deploy: 'dep-1', check: 'chk-1' }
    expect(routeFromContext(r)).toEqual(r)
    expect(routeFromContext({ at: 'env', env: ENV.id, tab: 'nope', extra: 1 })).toEqual({ at: 'env', env: ENV.id })
    expect(routeFromContext({ at: 'deploy', step: 'follow', deploy: 'd' })).toEqual({
      at: 'deploy',
      env: '',
      step: 'follow',
      deploy: 'd',
    })
    expect(routeFromContext({ at: 'setup', step: 'live' })).toEqual({ at: 'setup', step: 'live' })
    expect(routeFromContext({ at: 'setup', step: 'later' })).toEqual({ at: 'setup' })
    expect(routeFromContext({ at: 'setup', step: 'target', newProject: true })).toEqual({
      at: 'setup',
      step: 'target',
      newProject: true,
    })
    expect(routeFromContext({ at: 'setup', step: 'target', newProject: 'yes' })).not.toHaveProperty('newProject')
    expect(routeFromContext({ at: 'home' })).toEqual({ at: 'home' })
    expect(routeFromContext({ at: 'new-env', branch: 'feat/x' })).toEqual({ at: 'new-env', branch: 'feat/x' })
  })

  it('start, and setup in add mode, round-trip', () => {
    expect(routeFromContext({ at: 'start', extra: 1 })).toEqual({ at: 'start' })
    const adding: Route = { at: 'setup', step: 'target', adding: true, newProject: true }
    expect(routeFromContext(JSON.parse(JSON.stringify(adding)))).toEqual(adding)
    expect(routeFromContext({ at: 'setup', step: 'code', adding: 'yes' })).toEqual({ at: 'setup', step: 'code' })
  })

  it('an old context on a named environment keeps it', () => {
    expect(legacyRoute({ tab: 'run' }, 'e2')).toEqual({ at: 'env', env: 'e2', tab: 'checks' })
    expect(legacyRoute({ tab: 'overview' }, 'e2')).toBeNull()
  })

  it('a deep link carries the route back', () => {
    const r: Route = { at: 'deploy', env: ENV.id, step: 'review' }
    const link = routeLink(r)
    expect(link.startsWith('#/worker/iii-cloud/iii-cloud?context=')).toBe(true)
    expect(routeFromContext(JSON.parse(decodeURIComponent(link.split('context=')[1])))).toEqual(r)
  })
})

const billing: RecentProject = {
  path: '/home/you/workspaces/billing',
  repository: 'iii-hq/billing',
  cloud_project_id: 'p-billing',
  cloud_project_name: 'billing',
}

describe('defaultRoute', () => {
  it('nothing known is setup; a project in use or a recent one is Start, signed in or not', () => {
    expect(defaultRoute(signin)).toEqual({ at: 'setup' })
    expect(defaultRoute(target)).toEqual({ at: 'setup' })
    expect(defaultRoute({ ...main, project: null })).toEqual({ at: 'setup' })
    // In use, never live: Start (its row says Continue setup).
    expect(defaultRoute(main)).toEqual({ at: 'start' })
    expect(defaultRoute(live(main))).toEqual({ at: 'start' })
    expect(defaultRoute({ ...target, recent_projects: [billing] })).toEqual({ at: 'start' })
    expect(defaultRoute({ ...main, connection: signedOut, project: null, recent_projects: [billing] })).toEqual({
      at: 'start',
    })
  })
})

describe('pathOf', () => {
  const p = main.project!
  const dev = { id: 'e-dev', name: 'dev', production: false, branch: 'develop' }
  const s: Status = { ...main, project: { ...p, environments: [ENV, dev] } }
  /** Each item as `label` or `label→where`. */
  const path = (r: Route, status = s) =>
    pathOf(r, status).map((i) => (i.to ? `${i.label}→${i.to.at}${'env' in i.to ? `:${i.to.env}` : ''}` : i.label))

  it('every place of the map, its parent last before it', () => {
    expect(path({ at: 'start' })).toEqual(['Projects'])
    expect(path({ at: 'home' })).toEqual(['Projects→start', 'registry'])
    expect(path({ at: 'env', env: 'e-dev', tab: 'settings' })).toEqual(['Projects→start', 'registry→home', 'dev'])
    expect(path({ at: 'env', env: 'e-dev', tab: 'deploys', deploy: 'd1' })).toEqual([
      'Projects→start',
      'registry→home',
      'dev',
    ])
    expect(path({ at: 'env', env: 'e-dev', tab: 'checks', check: 'c1' })).toEqual([
      'Projects→start',
      'registry→home',
      'dev',
    ])
    expect(path({ at: 'new-env' })).toEqual(['Projects→start', 'registry→home', 'New environment'])
    expect(path({ at: 'deploy', env: 'e-dev', step: 'review' })).toEqual([
      'Projects→start',
      'registry→home',
      'dev→env:e-dev',
      'Deploy',
    ])
    // Setup: the first project has no way out; one in use goes back to it; adding goes back to Start.
    expect(path({ at: 'setup' }, target)).toEqual(['Set up'])
    expect(path({ at: 'setup', step: 'check' })).toEqual(['Projects→start', 'registry→home', 'Set up'])
    expect(path({ at: 'setup', step: 'code', adding: true })).toEqual(['Projects→start', 'Add a project'])
    expect(path({ at: 'setup', step: 'target', adding: true, newProject: true })).toEqual([
      'Projects→start',
      'Add a project',
    ])
  })
})

describe('setupStep', () => {
  it('follows the status: sign in, the folder, the target, the check, the deploy, live', () => {
    expect(setupStep(signin)).toBe('signin')
    expect(setupStep({ ...main, project: null })).toBe('code')
    expect(setupStep(target)).toBe('target')
    expect(setupStep(main)).toBe('check')
    expect(setupStep({ ...main, check: ready })).toBe('ship')
    expect(setupStep(boards['setup-deploying'].status)).toBe('ship')
    expect(setupStep(live(main))).toBe('live')
    const sir = { ...main.check!.problems[0], code: 'SIGN_IN_REQUIRED' }
    expect(setupStep({ ...main, check: { ...main.check!, problems: [sir] } })).toBe('signin')
  })

  it('an explicit step goes back, never ahead', () => {
    expect(setupStep(main, 'code')).toBe('code')
    expect(setupStep(main, 'check')).toBe('check')
    expect(setupStep(main, 'ship')).toBe('check')
    expect(setupStep(main, 'live')).toBe('check')
    expect(setupStep(signin, 'target')).toBe('signin')
    for (const s of SETUP_STEPS) expect(setupStep(live(main), s)).toBe(s)
  })

  it('Values comes between where it runs and the check; the status never puts setup there, a route does', () => {
    expect(SETUP_STEPS).toEqual(['signin', 'code', 'target', 'values', 'check', 'ship', 'live'])
    expect(setupStep(main)).toBe('check')
    expect(setupStep(main, 'values')).toBe('values')
    expect(setupStep({ ...main, check: ready }, 'values')).toBe('values')
    expect(setupStep(target, 'values')).toBe('target')
    expect(routeFromContext({ at: 'setup', step: 'values' })).toEqual({ at: 'setup', step: 'values' })
  })
})

describe('envState', () => {
  it('the selected env, one of the project being selected, or none of its', () => {
    const p = main.project!
    const status = { ...main, project: { ...p, environments: [...p.environments, { ...ENV, id: 'e2', name: 'dev' }] } }
    expect(envState(status, ENV.id)).toBe('selected')
    expect(envState(status, 'e2')).toBe('switching')
    expect(envState(status, 'gone')).toBe('unknown')
  })
})
