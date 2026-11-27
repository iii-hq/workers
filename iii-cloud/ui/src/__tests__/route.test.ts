// The route: old contexts, the default, the derived setup step, deep links (U1; frozen with route.ts).
import { describe, expect, it } from 'vitest'
import { boards } from '../fixtures'
import { base, ENV, signedOut } from '../fixtures/base'
import { defaultRoute, envState, legacyRoute, pathOf, type Route, routeFromContext, SETUP_STEPS } from '../route'
import type { RecentProject, Status } from '../types'

const main = base
const signin = boards['setup-signin'].status
const target = boards['setup-existing'].status

describe('routeFromContext', () => {
  it('maps every old context; overview and the palette open the default', () => {
    expect(routeFromContext({ tab: 'readiness' })).toEqual({ at: 'env', env: '' })
    expect(routeFromContext({ tab: 'run' })).toEqual({ at: 'env', env: '' })
    expect(routeFromContext({ tab: 'secrets' })).toEqual({ at: 'secrets', env: '' })
    expect(routeFromContext({ tab: 'deploys' })).toEqual({ at: 'env', env: '' })
    expect(routeFromContext({ tab: 'deploys', deployId: 'dep-1' })).toEqual({ at: 'env', env: '', deploy: 'dep-1' })
    expect(routeFromContext({ tab: 'deploys', pr: true })).toEqual({ at: 'deploy', env: '' })
    expect(routeFromContext({ tab: 'overview' })).toBeNull()
    expect(routeFromContext({})).toBeNull()
    for (const junk of [null, undefined, 'env', 3, [], { at: 'nowhere' }]) expect(routeFromContext(junk)).toBeNull()
  })

  it('every old route lands on its new place', () => {
    // home and new-env: the selected environment's page
    expect(routeFromContext({ at: 'home' })).toEqual({ at: 'env', env: '' })
    expect(routeFromContext({ at: 'new-env', branch: 'feat/x' })).toEqual({ at: 'env', env: '' })
    // env tabs: overview, deploys and checks are the environment; settings is the secrets page
    for (const tab of ['overview', 'deploys', 'checks'])
      expect(routeFromContext({ at: 'env', env: ENV.id, tab })).toEqual({ at: 'env', env: ENV.id })
    expect(routeFromContext({ at: 'env', env: ENV.id, tab: 'deploys', deploy: 'dep-1', check: 'chk-1' })).toEqual({
      at: 'env',
      env: ENV.id,
      deploy: 'dep-1',
    })
    expect(routeFromContext({ at: 'env', env: ENV.id, tab: 'settings' })).toEqual({ at: 'secrets', env: ENV.id })
    // setup: code and target are confirm; values, check, ship and live are the deploy
    expect(routeFromContext({ at: 'setup', step: 'code' })).toEqual({ at: 'setup', step: 'confirm' })
    expect(routeFromContext({ at: 'setup', step: 'target', newProject: true })).toEqual({
      at: 'setup',
      step: 'confirm',
      newProject: true,
    })
    for (const step of ['values', 'check', 'ship', 'live'])
      expect(routeFromContext({ at: 'setup', step })).toEqual({ at: 'deploy', env: '' })
    // a deploy with a step is the deploy screen, which has none
    expect(routeFromContext({ at: 'deploy', env: ENV.id, step: 'review' })).toEqual({ at: 'deploy', env: ENV.id })
    expect(routeFromContext({ at: 'deploy', step: 'follow', deploy: 'd' })).toEqual({
      at: 'deploy',
      env: '',
      deploy: 'd',
    })
  })

  it('takes a route as sent, dropping what is not one', () => {
    const rs: Route[] = [
      { at: 'env', env: ENV.id, deploy: 'dep-1' },
      { at: 'secrets', env: ENV.id },
      { at: 'deploy', env: ENV.id, deploy: 'dep-1' },
      { at: 'setup', step: 'signin' },
      { at: 'setup', step: 'confirm' },
    ]
    for (const r of rs) expect(routeFromContext(r)).toEqual(r)
    expect(routeFromContext({ at: 'env', env: ENV.id, tab: 'nope', extra: 1 })).toEqual({ at: 'env', env: ENV.id })
    expect(routeFromContext({ at: 'setup', step: 'later' })).toEqual({ at: 'setup' })
    expect(routeFromContext({ at: 'setup', step: 'confirm', newProject: 'yes' })).not.toHaveProperty('newProject')
  })

  it('start, and setup in add mode, round-trip', () => {
    expect(routeFromContext({ at: 'start', extra: 1 })).toEqual({ at: 'start' })
    const adding: Route = { at: 'setup', step: 'confirm', adding: true, newProject: true }
    expect(routeFromContext(JSON.parse(JSON.stringify(adding)))).toEqual(adding)
    expect(routeFromContext({ at: 'setup', step: 'confirm', adding: 'yes' })).toEqual({ at: 'setup', step: 'confirm' })
  })

  it('an old context on a named environment keeps it', () => {
    expect(legacyRoute({ tab: 'run' }, 'e2')).toEqual({ at: 'env', env: 'e2' })
    expect(legacyRoute({ tab: 'secrets' }, 'e2')).toEqual({ at: 'secrets', env: 'e2' })
    expect(legacyRoute({ tab: 'overview' }, 'e2')).toBeNull()
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
    expect(defaultRoute(main)).toEqual({ at: 'start' })
    expect(defaultRoute({ ...target, recent_projects: [billing] })).toEqual({ at: 'start' })
    expect(defaultRoute({ ...main, connection: signedOut, project: null, recent_projects: [billing] })).toEqual({
      at: 'start',
    })
  })
})

describe('pathOf', () => {
  const p = main.project!
  const dev = { id: 'e-dev', name: 'dev', production: false, branch: 'develop' }
  const s: Status = {
    ...main,
    project: { ...p, environments: [ENV, dev] },
    deploys: [{ ...main.deploys[0], id: 'd1', number: 6, environment_id: 'e-dev' }],
  }
  /** Each item as `label` or `label→where`. */
  const path = (r: Route, status = s) =>
    pathOf(r, status).map((i) => (i.to ? `${i.label}→${i.to.at}${'env' in i.to ? `:${i.to.env}` : ''}` : i.label))

  it('every place of the map, its parent last before it', () => {
    expect(path({ at: 'start' })).toEqual(['Projects'])
    expect(path({ at: 'env', env: 'e-dev' })).toEqual(['Projects→start', 'registry · dev'])
    expect(path({ at: 'env', env: 'e-dev', deploy: 'd1' })).toEqual([
      'Projects→start',
      'registry · dev→env:e-dev',
      'Deployment #6',
    ])
    expect(path({ at: 'deploy', env: 'e-dev' })).toEqual(['Projects→start', 'registry · dev→env:e-dev', 'Deploy'])
    expect(path({ at: 'secrets', env: 'e-dev' })).toEqual([
      'Projects→start',
      'registry · dev→env:e-dev',
      'Secrets and variables',
    ])
    // Setup: the first project has no way out; with others known, and adding, back goes to Projects.
    expect(path({ at: 'setup' }, target)).toEqual(['Put this project on iii cloud'])
    expect(path({ at: 'setup', step: 'confirm' })).toEqual(['Projects→start', 'Add a project'])
    expect(path({ at: 'setup', step: 'confirm', adding: true, newProject: true })).toEqual([
      'Projects→start',
      'Add a project',
    ])
  })
})

describe('SETUP_STEPS', () => {
  it('first time has two steps: sign in, then confirm', () => {
    expect(SETUP_STEPS).toEqual(['signin', 'confirm'])
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
