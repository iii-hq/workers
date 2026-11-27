// An environment's page (Env1, Env2): the next-action card, one state at a time and in a fixed order; Live now;
// and what is no longer on this page (machines, domains, restart, stop).
import { describe, expect, it, vi } from 'vitest'
import { clock } from '../asked'
import { type Next, nextAction } from '../EnvOverview'
import { appFile, withAppFile } from '../model'
import { valueName, needsYou as youOf } from '../findings'
import { agent, apiPath, base, check, DEPLOY_BASE, ENV, pr, problems, ready, T } from '../fixtures/base'
import { DEV, devDeploys } from '../fixtures/deploys'
import { NEW_SHA } from '../fixtures/manage'
import { envPageStatus } from '../fixtures/overview'
import type { Deploy, Status } from '../types'
import { board, btn, byText, click, cloudAnswers, ctx, el, flush, mount, text, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

const settle = async () => {
  for (let i = 0; i < 5; i++) await flush()
}
const path = () => el.querySelector('[role=toolbar] nav')?.textContent
const card = () => el.querySelector('section[aria-label=Next]')

// ---- the pure function ----

const LIVE = devDeploys[0]
const live = { id: LIVE.id, sha: LIVE.commit.sha }
type Folder = NonNullable<Status['project']>['checkout']
/** dev as the Env1 board has it (one commit to deploy), with `over` on the status and `folder` on the checkout. */
const dev = (over: Partial<Status> = {}, folder: Partial<Folder> = {}): Status => ({
  ...envPageStatus,
  ...over,
  project: { ...envPageStatus.project!, checkout: { ...envPageStatus.project!.checkout, ...folder } },
})
const clean = { not_served: [], changed_files: [] }
const next = (s: Status, over: Parameters<typeof nextAction>[2] = {}): Next =>
  nextAction(s, DEV.id, { live, now: Date.now(), ...over })
const deploy = (n: number, over: Partial<Deploy> = {}): Deploy => ({
  ...DEPLOY_BASE,
  id: `dep-${n}`,
  number: n,
  environment_id: DEV.id,
  commit: { sha: String(n).repeat(40).slice(0, 40), message: 'Add slow-query stats' },
  type: 'push',
  pr: null,
  approved_by_worker: false,
  checked_ready: false,
  created_at: T(3),
  ...over,
})
const failed7 = deploy(7, {
  status: 'failed',
  outcome: 'failed',
  finished_at: T(1),
  failure: {
    code: 'MIGRATION_RIGHTS',
    stage: 'config_secrets',
    worker: null,
    title: "a migration needs rights iii cloud doesn't give",
    hint: null,
    log_tail: [],
  },
})
const running7 = deploy(7, {
  stages: [{ name: 'workers', status: 'in_progress', detail: '2 of 3 ready', started_at: T(1), finished_at: null }],
})
const held7 = deploy(7, { status: 'needs_action' })
const passed = { ...ready, environment_id: DEV.id, commit: NEW_SHA, checked_at: T(2) }
const needsYou = {
  ...check(false, false),
  environment_id: DEV.id,
  problems: problems.filter((p) => p.owner === 'person'),
}
const agentOnly = {
  ...check(false, false),
  environment_id: DEV.id,
  problems: problems.filter((p) => p.owner === 'agent').slice(0, 2),
}

describe('the next-action card: one state for each combination of Env2', () => {
  it('something to deploy: the folder has a commit the environment does not serve', () => {
    const n = next(dev())
    expect(n).toMatchObject({
      state: 'deploy',
      wait: true,
      primary: true,
      action: { label: 'Deploy to dev', kind: 'check' },
    })
    expect(n.title).toBe("1 commit on feat/search-rank isn't on dev")
    expect(n.detail).toBe('2be71d0 Search: tweak weights · nothing uncommitted in your folder')
  })

  it('something to deploy: uncommitted files count too, and are said', () => {
    const n = next(dev({}, { ...clean, changed_files: ['a.ts', 'b.ts'] }))
    expect(n).toMatchObject({ state: 'deploy', action: { kind: 'check' } })
    expect(n.title).toBe("2 uncommitted files in your folder aren't on dev")
    expect(next(dev({}, { changed_files: ['a.ts'] })).detail).toBe(
      '2be71d0 Search: tweak weights · plus 1 uncommitted file',
    )
  })

  it('the check needs you: its title counts what only you can do, and the agent working meanwhile is said', () => {
    const n = next(dev({ check: needsYou }))
    expect(n).toMatchObject({
      state: 'you',
      wait: true,
      action: { label: 'Open the check', kind: 'go', to: { at: 'deploy', env: DEV.id } },
    })
    expect(n.title).toBe('2 things need you before dev can get feat/search-rank')
    const meanwhile = next(dev({ check: needsYou, agent: agent('working') }))
    expect(meanwhile.state).toBe('you')
    expect(meanwhile.detail).toContain('Your agent is fixing 1 problem meanwhile.')
  })

  it('the agent is working and nothing needs you: a plain card with Open the check as the secondary action', () => {
    const n = next(dev({ check: agentOnly, agent: agent('working') }))
    expect(n).toMatchObject({
      state: 'agent',
      wait: false,
      primary: false,
      action: { label: 'Open the check', kind: 'go', to: { at: 'deploy', env: DEV.id } },
    })
    expect(n.title).toBe('Your agent is fixing 1 problem for dev')
    expect(n.detail).toMatch(/^Working \d+ min in a separate copy\./)
  })

  it('the check passed on a commit the environment does not serve: Review and deploy', () => {
    const n = next(dev({ check: passed }))
    expect(n).toMatchObject({
      state: 'passed',
      wait: true,
      action: { label: 'Review and deploy', kind: 'go', to: { at: 'deploy', env: DEV.id } },
    })
    expect(n.title).toBe(`2be71d0 passed the check at ${clock(T(2))}`)
    expect(n.detail).toContain('It waits for you.')
  })

  it('a deployment is running: Follow, to that deployment on the Deploy place', () => {
    const n = next(dev({ deploys: [running7, ...devDeploys] }))
    expect(n).toMatchObject({
      state: 'running',
      wait: false,
      action: { label: 'Follow', kind: 'go', to: { at: 'deploy', env: DEV.id, deploy: 'dep-7' } },
    })
    expect(n.title).toBe('Deployment #7 is deploying')
    expect(n.detail).toContain('Workers: 2 of 3 ready')
    expect(n.detail).toContain("dev serves #6 until it's live.")
  })

  it('a deployment iii cloud holds for approval needs you: Review it, on the Deploy place', () => {
    const n = next(dev({ deploys: [held7, ...devDeploys] }))
    expect(n).toMatchObject({
      state: 'held',
      wait: true,
      action: { label: 'Review it', to: { at: 'deploy', env: DEV.id, deploy: 'dep-7' } },
    })
  })

  it('the last deployment failed: its cause, who started it, what still serves; See what failed opens its record', () => {
    const n = next(dev({ deploys: [failed7, ...devDeploys] }, clean))
    expect(n).toMatchObject({
      state: 'failed',
      wait: false,
      primary: true,
      action: { label: 'See what failed', kind: 'go', to: { at: 'env', env: DEV.id, deploy: 'dep-7' } },
    })
    expect(n.title).toBe("Deployment #7 failed: a migration needs rights iii cloud doesn't give")
    expect(n.detail).toBe('A push to GitHub, not checked. dev still serves #6.')
  })

  it('up to date: no action', () => {
    const n = next(dev({}, clean))
    expect(n).toMatchObject({ state: 'current', wait: false })
    expect(n.action).toBeUndefined()
    expect(n.title).toBe('dev serves everything in your folder')
    expect(n.detail).toBe('feat/search-rank at 2be71d0, nothing uncommitted.')
  })

  it('not deployed yet where a pull request deploys: Open pull request #n, a link, with who merges it', () => {
    const s: Status = { ...base, deploys: [], check: null, pr: { ...pr, number: 170 } }
    const n = nextAction(s, ENV.id, { live: null })
    expect(n).toMatchObject({
      state: 'pull-request',
      wait: false,
      action: { label: 'Open pull request #170', kind: 'link', href: pr.url },
    })
    expect(n.title).toBe('Nothing is live on production yet')
    expect(n.detail).toBe("It deploys when pull request #170 into main is merged. You're allowed to merge it.")
    const role = (r: string): Status => ({
      ...s,
      connection: { ...s.connection, teams: [{ ...s.connection.teams[0], role: r }] },
    })
    expect(nextAction(role('Developer'), ENV.id, { live: null }).detail).toContain(
      "An Owner or Admin merges it; you're a Developer.",
    )
    expect(nextAction(role('Admin'), ENV.id, { live: null }).detail).toContain("You're allowed to merge it.")
  })

  it('nothing live and nothing to deploy: says so, and the check is one click away', () => {
    const n = next(dev({ deploys: [] }, clean), { live: null })
    expect(n).toMatchObject({ state: 'empty', action: { label: 'Open the check', kind: 'go' } })
    expect(n.title).toBe('Nothing is live on dev yet')
  })
})

describe('the next-action card: the same count and the same agent states as the Deploy screen', () => {
  it('a first deploy with names a .env can fill: the card counts the one row the Deploy title counts, or prints no number', () => {
    const person = problems.find((p) => p.owner === 'person')!
    const secrets = ['NAME_A', 'NAME_B', 'NAME_C'].map((n) => ({
      ...person,
      id: `p-${n}`,
      code: 'SECRET_MISSING',
      severity: 'block' as const,
      title: `${n} has no value`,
      detail: `See api/x.yaml:1. (at containers.api.environment.${n})`,
    }))
    const s = dev({ check: { ...needsYou, problems: secrets }, deploys: [] })
    const total = youOf(s).count
    expect(next(s).title).toMatch(new RegExp(`^${total} things? (needs|need) you`))
    const names = youOf(s).items.flatMap((i) => valueName(i) ?? [])
    const read = {
      environment_id: DEV.id,
      environment: 'dev',
      manifest: 'api/worker-compose-prod.yaml',
      read_from: 'workspace' as const,
      cloud: 'read' as const,
      save: 'ok' as const,
      items: names.map((name) => ({
        name,
        need: 'required' as const,
        default: null,
        type: 'secret' as const,
        database: false,
        containers: [],
        line: 1,
        in_cloud: 'unset' as const,
        value: null,
        updated_at: null,
        asked: null,
        suggestions: [{ source: 'env_file' as const, file: '.env', preselect: true, local: false }],
      })),
    }
    // Every nameable need is fillable: they collapse into one row, as in the Deploy title.
    expect(names.length).toBeGreaterThan(1)
    const grouped = next(s, { values: read })
    expect(youOf(s, read).count).toBeLessThan(total)
    expect(grouped.title).toMatch(new RegExp(`^${youOf(s, read).count} things? `))
    // The read has not arrived: no number the card cannot stand behind.
    expect(next(s, { valuesPending: true }).title).toMatch(/^Things need you before dev can get /)
  })

  it.each([
    'waiting_for_you',
    'stalled',
  ] as const)('an agent that is %s: the check needs you, not "Deploy to dev"', (state) => {
    const n = next(dev({ agent: agent(state) }))
    expect(n).toMatchObject({ state: 'you', action: { label: 'Open the check', kind: 'go' } })
    expect(n.title).toBe('The check for dev needs you')
  })

  it("a stalled agent says why when the worker did: its question is the card's detail", () => {
    const why = 'The worktree is on another branch than iii-cloud/ready-dev.'
    const n = next(dev({ agent: { ...agent('stalled'), question: why } }))
    expect(n.detail).toBe(why)
    expect(next(dev({ agent: agent('stalled') })).detail).toContain('no progress')
  })
})

describe('the next-action card: when two states apply, the first of the order wins', () => {
  it('a failed last deployment while a new check is running: the running check', () => {
    const running = { ...passed, ready: false, run: { ...ready.run!, status: 'running' as const } }
    const n = next(dev({ deploys: [failed7, ...devDeploys], check: running }, clean))
    expect(n.state).toBe('agent')
    expect(n.title).toBe('A check of dev is running')
  })

  it('a deployment running beats a check that needs you; a held one beats a running one', () => {
    expect(next(dev({ deploys: [running7, ...devDeploys], check: needsYou })).state).toBe('running')
    const second = { ...running7, id: 'dep-8', number: 8 }
    expect(next(dev({ deploys: [second, held7, ...devDeploys], check: needsYou })).state).toBe('held')
  })

  it('what needs you beats the agent working, which beats a passed check, which beats a failed deployment', () => {
    expect(next(dev({ check: needsYou, agent: agent('working') })).state).toBe('you')
    expect(next(dev({ check: passed, agent: agent('working') })).state).toBe('agent')
    expect(next(dev({ check: passed, deploys: [failed7, ...devDeploys] })).state).toBe('passed')
  })

  it('a passed check of the very commit whose deployment failed is not offered again: the failure comes first', () => {
    const same = { ...passed, commit: failed7.commit.sha }
    expect(next(dev({ check: same, deploys: [failed7, ...devDeploys] }, clean)).state).toBe('failed')
  })

  it('a passed check of what the environment already serves is no state: it serves everything', () => {
    expect(next(dev({ check: { ...passed, commit: LIVE.commit.sha } }, clean)).state).toBe('current')
  })

  it('a failed deployment while the folder has moved on (uncommitted files, or newer commits): deploy again', () => {
    expect(next(dev({ deploys: [failed7, ...devDeploys] }, { ...clean, changed_files: ['x'] })).state).toBe('deploy')
    const newer = [{ sha: 'f'.repeat(40), message: 'Fix the migration' }]
    expect(next(dev({ deploys: [failed7, ...devDeploys] }, { ...clean, not_served: newer })).state).toBe('deploy')
    // The only commit the folder has beyond the environment is the one that failed: it is still the failure.
    const same = [{ sha: failed7.commit.sha, message: 'Add slow-query stats' }]
    expect(next(dev({ deploys: [failed7, ...devDeploys] }, { ...clean, not_served: same })).state).toBe('failed')
  })

  it('a pull request beats the folder when nothing is live, and never when something is', () => {
    const s: Status = { ...base, deploys: [], check: null, pr }
    expect(nextAction(s, ENV.id, { live: null }).state).toBe('pull-request')
    expect(nextAction(s, ENV.id, { live: { id: 'x', sha: 'a' } }).state).toBe('deploy')
  })
})

// ---- the page ----

describe('the environment page', () => {
  it('the card, Live now and the footer: Env1', async () => {
    await board('env-page')
    await settle()
    expect(card()?.textContent).toContain("1 commit on feat/search-rank isn't on dev")
    const liveNow = el.querySelector('section[aria-label="Live now"]')?.textContent ?? ''
    expect(liveNow).toContain('Serving9c04e7f Search: rank by starsdeployment #6, since')
    expect(liveNow).toContain('Health 11 of 11 workers upchecked')
    expect(liveNow).toContain("Deploys fromthis machine, after the checka push to dev's branch deploys without it")
    expect(text()).toContain('Secrets and variables')
    expect(text()).toContain("Machines, domains, rollback and a new environment are in iii cloud's console.")
    expect(byText('a', "iii cloud's console")?.getAttribute('href')).toBe('https://cloud.iii.dev/iii/registry/dev')
    // Every link to the console is a page it has: never `/<team>/<project>` and never `/variables`.
    const hrefs = [...el.querySelectorAll('a')].map((a) => a.getAttribute('href') ?? '').filter((h) => h.startsWith('https://cloud.iii.dev/'))
    expect(hrefs.length).toBeGreaterThan(1)
    for (const h of hrefs) {
      expect(h).not.toMatch(/\/variables$/)
      expect(h).not.toMatch(/^https:\/\/cloud\.iii\.dev\/[^/]+\/[^/]+$/)
    }
    expect(path()).toBe('Projects / registry · dev')
  })

  it('an environment that deploys from a pull request says when: Deploys from', async () => {
    await mount(base, ctx({ at: 'env', env: ENV.id }), cloudAnswers({ [apiPath('/deployments')]: { deployments: [] } }))
    await settle()
    const liveNow = el.querySelector('section[aria-label="Live now"]')?.textContent ?? ''
    expect(liveNow).toContain('Deploys fromwhen its pull request is mergedinto its base branch')
    expect(liveNow).toContain('Nothing is live yet')
  })

  it('a refused check is not lost on the way: the Deploy place shows what it said', async () => {
    const h = await board(
      'env-page',
      {},
      { 'iii-cloud::check': () => Promise.reject(Object.assign(new Error('x'), { code: 'NO_COMMITS' })) },
    )
    await settle()
    await click(btn('Deploy to dev'))
    await settle()
    expect(h.calls.some((c) => c.id === 'iii-cloud::check')).toBe(true)
    expect(path()).toBe('Projects / registry · dev / Deploy')
    expect(text()).toContain('Commit something first')
  })

  it('Deploy to dev starts the check and goes to the Deploy place', async () => {
    const h = await board('env-page')
    await settle()
    await click(btn('Deploy to dev'))
    expect(h.calls.find((c) => c.id === 'iii-cloud::check')?.payload).toMatchObject({ run: true })
    expect(path()).toBe('Projects / registry · dev / Deploy')
  })

  it.each([
    ['Open the check', { check: needsYou }, 'Projects / registry · dev / Deploy'],
    ['Review and deploy', { check: passed }, 'Projects / registry · dev / Deploy'],
    ['Follow', { deploys: [running7, ...devDeploys] }, 'Projects / registry · dev / Deploy'],
    ['See what failed', { deploys: [failed7, ...devDeploys] }, 'Projects / registry · dev / Deployment #7'],
  ] as [
    string,
    Partial<Status>,
    string,
  ][])('"%s" goes where Env2 says, and starts no work', async (label, over, to) => {
    const h = await mount(
      dev(over, over.deploys ? clean : {}),
      ctx({ at: 'env', env: DEV.id }),
      cloudAnswers({ [apiPath('/deployments', DEV.id)]: { deployments: [] } }),
    )
    await settle()
    await click(btn(label))
    expect(path()).toBe(to)
    expect(h.calls.some((c) => c.id === 'iii-cloud::check')).toBe(false)
  })

  it('a card that waits for you has a strong outline, one that does not a plain one, a failed one is marked', async () => {
    await board('env-page')
    expect(card()?.hasAttribute('data-wait')).toBe(true)
    await mount(dev({ deploys: [failed7, ...devDeploys] }, clean), ctx({ at: 'env', env: DEV.id }, 2), cloudAnswers({}))
    await settle()
    expect(card()?.hasAttribute('data-wait')).toBe(false)
    expect(card()?.hasAttribute('data-failed')).toBe(true)
  })

  it('no machines, domains, restart, stop or rollback here, and none is read from iii cloud', async () => {
    const h = await board('env-page')
    await settle()
    for (const gone of ['Restart a group', 'Add a domain', 'Stop dev', 'Roll back', 'Your domains', 'Change…']) {
      expect(text()).not.toContain(gone)
    }
    const reads = h.calls.filter((c) => c.id === 'iii-cloud::api').map((c) => String(c.payload?.path))
    expect(reads.filter((p) => /machines|domains|rollback|logs/.test(p))).toEqual([])
    expect(h.calls.filter((c) => c.id === 'iii-cloud::api::write')).toEqual([])
  })
})

describe('a page that is out of date', () => {
  it('cannot start a check or approve a deployment, but can still be read and followed', async () => {
    let down = false
    const status = () => {
      if (down) throw new Error('worker gone')
      return envPageStatus
    }
    await mount(status as unknown as Status, ctx({ at: 'env', env: DEV.id }), cloudAnswers({}))
    await settle()
    expect((btn('Deploy to dev') as HTMLButtonElement).disabled).toBe(false)
    down = true
    await click(btn('Refresh'))
    expect((btn('Deploy to dev') as HTMLButtonElement).disabled).toBe(true)
  })
})

describe("the compose file of an environment's work is its newest check's", () => {
  it("Next and a page check name the newest check's file, and the environment's own when it has none", () => {
    const x = 'api/worker-compose-cloud.yaml'
    const s = dev({ check: { ...passed, manifest: x } })
    expect(appFile(s)).toMatchObject({ path: x, switches: true })
    expect(withAppFile(s, { run: true })).toEqual({ run: true, manifest: x })
    const own = dev({ check: { ...passed, manifest: null } })
    expect(withAppFile(own).manifest).toBe(appFile(own).path)
    expect(appFile(own).switches).toBe(false)
  })
})
