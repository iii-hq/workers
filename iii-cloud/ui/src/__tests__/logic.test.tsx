// Model, plain messages, the hub, chat cards and the form: no area owns these.
import type { FunctionCallMessage, Host } from '@iii-dev/console-ui'
import { act } from 'react'
import { describe, expect, it, vi } from 'vitest'
// The shared cases the worker's Rust rule reads too.
import wayCases from '../../../tests/fixtures/deploy-way.json'
import setup from '../../page'
import { clock } from '../asked'
import { CloudPage } from '../CloudPage'
import { cardRecords } from '../fixtures'
import { base, ENV, following, ready, secretRequests } from '../fixtures/base'
import { forgetHubMemory, hubOf, useActions } from '../hub'
import {
  configDir,
  deployOrigin,
  deployState,
  deployWay,
  listMayHideServing,
  liveOf,
  opensToPublic,
  plainMessage,
  servingId,
  settingWords,
  shortDate,
  whoLine,
  workerError,
} from '../model'
import type { ChangeEvent, CloudDeployment, Deploy, Status } from '../types'
import { fakeHost } from './host'
import { btn, click, ctx, el, flush, mount, root, text, ui, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

describe('how an environment deploys', () => {
  it('follows the cases the worker reads too', () => {
    expect(wayCases.length).toBeGreaterThanOrEqual(8)
    for (const c of wayCases) {
      const others = c.others.map((branch, i) => ({ name: `other-${i}`, branch }))
      const got = deployWay(c.default, c.env, [c.env, ...others])
      expect(got, c.case).toEqual(c.way === 'pull_request' ? { way: 'pull_request' } : { way: 'assisted', own: c.own })
    }
  })
})

describe('the header who-line (P5)', () => {
  it('names the person (name, else email) and the team with its role; an API key and signed out say so; never "as you"', () => {
    expect(whoLine(base)).toBe('you@example.com · team iii (Owner)')
    const named = { ...base, connection: { ...base.connection, user: { id: 'u1', email: 'a@b.c', name: 'Ana' } } }
    expect(whoLine(named)).toBe('Ana · team iii (Owner)')
    const key = { secret: 'K', team_id: base.connection.teams[0].id, environment_id: 'e' }
    expect(whoLine({ ...base, connection: { ...base.connection, api_key: key } })).toBe('API key · iii')
    expect(whoLine({ ...base, connection: { ...base.connection, status: 'signed_out' } })).toBe('Not connected')
  })
})

describe('O1/O2: override words and the config folder', () => {
  it('says web in short words (serve, container, port, api.prefix, access); any other setting as compact JSON', () => {
    const web = { serve: 'container', container: 'web', port: 3000, access: 'team' }
    expect(settingWords('web', web)).toBe('serves web on 3000 · team only')
    expect(settingWords('web', { ...web, access: 'public', api: { prefix: '/api' } })).toBe(
      'serves web on 3000 · API under /api · public',
    )
    expect(settingWords('web', { serve: 'container', api: { prefix: null } })).toBe(
      'serves the detected container · no API prefix',
    )
    expect(settingWords('web', { serve: 'static', directory: 'dist' })).toBe('serves static files from dist')
    expect(settingWords('release', { worker: 'api', command: 'pnpm db:migrate' })).toBe(
      '{"worker":"api","command":"pnpm db:migrate"}',
    )
    expect(settingWords('web', null)).toBe('not set')
    expect(opensToPublic({ console: web, file: { ...web, access: 'public' } })).toBe(true)
    expect(opensToPublic({ console: { ...web, access: 'public' }, file: web })).toBe(false)
    expect(opensToPublic({ console: null, file: { ...web, access: 'public' } })).toBe(false)
  })

  it("reads the config folder from the problem's excerpt", () => {
    const at = (lines: string[]) =>
      ({ excerpt: { start_line: 1, lines } }) as unknown as Parameters<typeof configDir>[0]
    expect(configDir(at(['        name: fs', '        config:', '          directory: ./config']))).toBe('./config')
    expect(configDir(at(['  directory: "./conf"']))).toBe('./conf')
    expect(configDir(at(['  name: fs']))).toBeNull()
    expect(configDir(null)).toBeNull()
  })
})

describe('plain error messages', () => {
  it('drop a code prefix and never show the code as detail', async () => {
    const s = { ...base, check: ready }
    const run = {
      ...s.check!.run!,
      status: 'failed' as const,
      error: { stage: 'install' as const, code: 'CLOUD_ERROR', message: 'CLOUD_ERROR: boom' },
    }
    // The person moved on from "What goes" to the check: the screen remembers its step in the pane.
    ui.panes[`iii-cloud:deploy-step:t:${base.project!.environment!.id}`] = 'check'
    await mount(
      { ...s, check: { ...s.check!, ready: false, run } },
      { panelContext: { id: 1, pageId: 'iii-cloud', context: { at: 'deploy', env: '' } } },
    )
    expect(text()).toContain('iii cloud had a problem')
    expect(text()).not.toContain('CLOUD_ERROR')
  })
})

describe('one source per fact (the review, defects 1, 2 and 7)', () => {
  const dep = (over: Partial<Deploy>): Deploy => ({
    ...following,
    pr: null,
    approved_by_worker: false,
    from_console: false,
    check_id: null,
    ...over,
  })
  const cloud = (d: Deploy, over: Partial<CloudDeployment> = {}): CloudDeployment => ({
    id: d.id,
    number: d.number,
    environment_id: d.environment_id,
    type: d.type,
    status: 'ready',
    current: false,
    commit: { sha: d.commit.sha },
    triggered_by: { user_id: null, api_key_id: null },
    created_at: d.created_at,
    finished_at: null,
    ...over,
  })
  const at = (...ds: Deploy[]): Status => ({ ...base, deploys: ds })

  it('the same deployment reads the same wherever it is asked: local record alone, or with iii cloud, a push never "from the console"', () => {
    // A push: the worker did not create it, so it marks `from_console` too; its type says it is a push.
    const push = dep({ id: 'p', type: 'push', from_console: true, checked_ready: false })
    const s = at(push)
    const alone = deployOrigin(s, push)
    expect(alone.line).toBe('a push to GitHub, not checked')
    expect(deployOrigin(s, push, cloud(push)).line).toBe(alone.line)
    expect(deployOrigin(s, { id: 'p', type: 'push' }, cloud(push)).line).toBe(alone.line)
    // Made by this worker: it was checked first, and an API key from iii cloud is still "you".
    const mine = dep({ id: 'm', type: 'manual', check_id: 'chk-1', checked_ready: true })
    const viaKey = cloud(mine, { triggered_by: { user_id: null, api_key_id: 'k' } })
    expect(deployOrigin(at(mine), mine, viaKey).line).toBe('you, from this page · checked')
    expect(deployOrigin(at(mine), mine).line).toBe('you, from this page · checked')
    // A merge, a console deployment, a teammate by name (iii cloud wins for who).
    const merged = dep({ id: 'g', pr: 12, approved_by_worker: true, checked_ready: true })
    expect(deployOrigin(at(merged), merged).line).toBe('you, from the merge of #12 · checked')
    const con = dep({ id: 'c', type: 'manual', from_console: true, checked_ready: false })
    const bruno = cloud(con, { triggered_by: { user_id: 'u-bruno', api_key_id: null } })
    const members = { members: [{ user: { id: 'u-bruno', email: 'b@x.y', name: 'bruno' } }] }
    expect(deployOrigin(at(con), con, bruno, members).line).toBe('bruno, from the console, not checked')
  })

  it('an older live deployment is Replaced, the one serving is Live, in the local record and in iii cloud', () => {
    const old = dep({ id: 'old', outcome: 'ready', status: 'ready' })
    const now = dep({ id: 'now', outcome: 'ready', status: 'ready' })
    expect(deployState(old, 'now')).toEqual({ label: 'Replaced', tone: 'ghost' })
    expect(deployState(now, 'now')).toEqual({ label: 'Live', tone: 'ok' })
    expect(deployState(cloud(old), 'now').label).toBe('Replaced')
    expect(deployState(cloud(now, { current: true })).label).toBe('Live')
    expect(deployState(dep({ outcome: 'canceled', status: 'canceled' })).label).toBe('Canceled')
    expect(deployState(dep({ outcome: 'failed', status: 'failed' })).label).toBe('Failed')
    expect(deployState(dep({ outcome: 'running', status: 'in_progress' })).label).toBe('Deploying')
  })

  it('a date is English whatever the browser language: it never asks the browser for its own', () => {
    const spy = vi.spyOn(Date.prototype, 'toLocaleDateString')
    expect(shortDate('2026-11-25T12:00:00Z')).toBe('25 Nov')
    expect(spy.mock.calls.every((c) => c[0] === 'en-GB')).toBe(true)
    spy.mockRestore()
    const time = vi.spyOn(Date.prototype, 'toLocaleTimeString')
    expect(clock('2026-11-25T12:00:00Z')).toMatch(/^\d{2}:\d{2}$/)
    expect(time.mock.calls.every((c) => c[0] === 'en-GB')).toBe(true)
    time.mockRestore()
  })
})

describe('replaced deploys, dirty runs and new codes', () => {
  it('NOT_SAVED, RUN_PANIC and PR_HEAD_CHANGED have plain messages', () => {
    expect(plainMessage({ code: 'NOT_SAVED', message: 'x' })).toContain("this API key can't write variables")
    expect(plainMessage({ code: 'RUN_PANIC', message: 'x' })).toBe('The run stopped unexpectedly. Run it again.')
    expect(plainMessage({ code: 'PR_HEAD_CHANGED', message: 'x' })).toBe(
      'The pull request changed after the check. Check again.',
    )
  })
})

describe('check events', () => {
  it('an older check does not replace the current one; a newer one does', async () => {
    const h = await mount(base, ctx({ at: 'deploy', env: ENV.id }))
    const cur = base.check!
    const fire = (c: typeof cur) =>
      act(async () =>
        h.handlers.get('iii::iii-cloud-ui::events::console-test')?.({
          kind: 'check',
          id: c.id,
          terminal: true,
          summary: '',
          record: c,
        }),
      )
    const older = {
      ...cur,
      id: 'chk-older',
      checked_at: new Date(Date.parse(cur.checked_at) - 60_000).toISOString(),
      ready: true,
      problems: [],
    }
    const title = () => el.querySelector('h2.iii-cloud-h')?.textContent
    expect(title()).toBe('2 things need you · 2 for your agent to fix')
    await fire(older)
    expect(title()).toBe('2 things need you · 2 for your agent to fix')
    await fire({
      ...cur,
      id: 'chk-newer',
      checked_at: new Date(Date.parse(cur.checked_at) + 60_000).toISOString(),
      ready: true,
      problems: [],
    })
    expect(title()).toBe('Ready to deploy to production')
  })
})

describe('final core fields', () => {
  it('the new codes read as plain sentences; RUN_BUSY reads its start from the message', () => {
    for (const code of ['NO_COMMITS', 'WRONG_WORKTREE', 'NEEDS_BROWSER_SIGN_IN']) {
      const m = plainMessage({ code, message: `${code}: raw` })
      expect(m).not.toContain(code)
      expect(m.length).toBeGreaterThan(30)
    }
    expect(plainMessage({ code: 'NO_COMMITS', message: 'x' })).toContain('Commit something first')
    expect(workerError(null, 'RUN_BUSY', 'run run-1a2b3c is running since 2026-11-25T14:03:00.000Z').started_at).toBe(
      '2026-11-25T14:03:00.000Z',
    )
  })
})

describe("codes that keep the worker's own words", () => {
  it("its plain sentence when it sends one; the page's own when it is empty or raw", () => {
    const own = {
      PROJECT_DELETED: 'billing no longer exists in iii cloud.',
      MERGE_IN_PROGRESS: 'A merge is still open in the worktree ~/w.',
      WORKTREE_PATH_TAKEN: '~/w-iii-cloud is already a folder of another repository.',
      NO_ACCESS: 'You are not a member of team acme.',
      OWN_BRANCH_TAKEN:
        "GitHub already has a branch iii-cloud/test with other commits. Delete it, or set test's branch in iii cloud. Nothing changed.",
      BRANCH_NOT_MOVED:
        'test could not move to its own branch iii-cloud/test (iii cloud answered 403). Nothing was deployed. Deploy again to retry.',
    }
    for (const [code, message] of Object.entries(own)) {
      expect(plainMessage({ code, message })).toBe(message)
      expect(plainMessage({ code, message: `${code}: ${message}` })).toBe(message)
      const fallback = plainMessage({ code, message: '' })
      expect(fallback).not.toBe('Something went wrong. Try again in a moment.')
      expect(plainMessage({ code, message: 'fatal: not a git repository\nat x' })).toBe(fallback)
    }
  })
})

describe('live data', () => {
  it('upserts a deploy event into the page and observes once per tab', async () => {
    const h = await mount(base, ctx({ at: 'deploy', env: ENV.id, deploy: following.id }))
    expect(h.observed).toEqual([{ source: 'iii-cloud::on-change', target: 'iii::iii-cloud-ui::events::console-test' }])
    expect(text()).not.toContain('7a1f3b2')
    const ev: ChangeEvent = {
      kind: 'deploy',
      id: 'dep-7a1f3b2',
      terminal: false,
      summary: '',
      record: following,
    }
    await act(async () => h.handlers.get('iii::iii-cloud-ui::events::console-test')?.(ev))
    expect(text()).toContain('7a1f3b2')
    expect(text()).toContain('Deploying')
  })
})

describe('board 11: chat cards', () => {
  const env = (output: unknown, functionId: string, extra: Partial<FunctionCallMessage> = {}): FunctionCallMessage => ({
    id: 'm1',
    role: 'function-call',
    functionId,
    input: {},
    output,
    createdAt: 0,
    ...extra,
  })
  const wrap = (details: unknown) => ({ content: [{ type: 'text', text: '{}' }], details })
  async function registered(status = { ...base, check: ready, secrets: [secretRequests[0]], deploys: [following] }) {
    const h = fakeHost(status)
    setup(h.host)
    const renderer = (h.host.functionCalls.register as ReturnType<typeof vi.fn>).mock.calls[0][0]
    return { h, renderer }
  }
  const show = async (node: unknown) => {
    await act(async () => root.render(node as never))
    await flush()
  }
  it('registers the page, the five renderers, the form and the palette command', async () => {
    const { h, renderer } = await registered()
    expect((h.host.pages.register as ReturnType<typeof vi.fn>).mock.calls[0][0]).toMatchObject({
      id: 'iii-cloud',
      title: 'iii cloud',
      configurationId: 'iii-cloud',
    })
    expect((h.host.configForms.register as ReturnType<typeof vi.fn>).mock.calls[0][0]).toBe('iii-cloud')
    expect(
      (h.host.commands as unknown as { register: ReturnType<typeof vi.fn> }).register.mock.calls[0][1][0].title,
    ).toBe('Open iii cloud')
    for (const n of ['connect', 'check', 'secret', 'deploy', 'watch'])
      expect(renderer.isMatch(`iii-cloud::${n}`)).toBe(true)
    expect(renderer.isMatch('iii-cloud::status')).toBe(false)
    expect(renderer.metadata).toEqual({ display: true })
  })

  it('error envelopes and non-records fall through to the host', async () => {
    const { renderer } = await registered()
    expect(renderer.tryRender(env({ error: { message: 'x' } }, 'iii-cloud::check'))).toBeNull()
    expect(renderer.tryRender(env(wrap({ nope: 1 }), 'iii-cloud::deploy'))).toBeNull()
    expect(renderer.tryRender(env('not json', 'iii-cloud::secret'))).toBeNull()
    expect(renderer.tryRenderDisplay(env(undefined, 'iii-cloud::check'))).toBeNull()
  })

  it('connect waiting and signed in', async () => {
    const { renderer } = await registered()
    await show(renderer.tryRender(env(wrap(cardRecords.waiting), 'iii-cloud::connect')))
    expect(text()).toContain('iii-cloud::connect')
    expect(text()).toContain('Waiting for you')
    expect(text()).toContain('cloud.iii.dev/device')
    expect(text()).toContain('WXYZ-1234')
    await show(renderer.tryRender(env(JSON.stringify(wrap(cardRecords.connect)), 'iii-cloud::connect')))
    expect(text()).toContain('Signed in')
  })

  it('check not ready opens the check; ready opens the run, both on the check and deploy screen of the environment', async () => {
    const { h, renderer } = await registered()
    await show(renderer.tryRender(env(wrap(cardRecords.checkBlocked), 'iii-cloud::check')))
    expect(text()).toContain('Not ready')
    expect(text()).toContain('registry/production')
    expect(text()).toContain('commit 3c4d2e0')
    expect(text()).toContain("iii cloud's plan: 3 to fix, at least")
    expect(text()).toContain('Checks on this machine: 4 problems')
    expect(text()).toContain('2 need your decision')
    const check = (c: { environment_id: string }) => ({ at: 'deploy', env: c.environment_id })
    await click(btn('Open the check'))
    expect(h.open).toHaveBeenCalledWith({ pageId: 'iii-cloud', context: check(cardRecords.checkBlocked) })
    await show(renderer.tryRenderDisplay(env(wrap(cardRecords.checkReady), 'iii-cloud::check')))
    expect(text()).toContain("iii cloud's plan is clean")
    expect(text()).toContain('Local run: 3 of 3 workers, 14 routes, healthcheck 200')
    await click(btn('See the local run'))
    expect(h.open).toHaveBeenLastCalledWith({ pageId: 'iii-cloud', context: check(cardRecords.checkReady) })
  })

  it('a check card turns ready when the hub gets the record of the same id', async () => {
    const { h, renderer } = await registered()
    await show(renderer.tryRender(env(wrap(cardRecords.checkBlocked), 'iii-cloud::check')))
    expect(text()).toContain('Not ready')
    const ev: ChangeEvent = {
      kind: 'check',
      id: cardRecords.checkBlocked.id,
      terminal: true,
      summary: '',
      record: {
        ...cardRecords.checkBlocked,
        ready: true,
        problems: [],
        plan: { ...cardRecords.checkBlocked.plan, blocking: 0, status: 'clean' },
      },
    }
    await act(async () => h.handlers.get('iii::iii-cloud-ui::events::console-test')?.(ev))
    expect(text()).toContain('Ready')
    expect(text()).not.toContain('Not ready')
  })

  it('secret card never shows a value and opens Secrets and variables', async () => {
    const { h, renderer } = await registered()
    await show(renderer.tryRender(env(wrap(cardRecords.secret), 'iii-cloud::secret')))
    expect(text()).toContain('SESSION_SECRET')
    expect(text()).toContain('type it on the iii cloud page; the value never comes back to this chat')
    await click(btn('Secrets and variables'))
    expect(h.open).toHaveBeenCalledWith({
      pageId: 'iii-cloud',
      context: { at: 'secrets', env: cardRecords.secret.environment_id },
    })
  })

  it('watch card: live stage lines, then failed; it opens the deploy on its environment', async () => {
    const { h, renderer } = await registered()
    await show(renderer.tryRender(env(wrap(cardRecords.watch), 'iii-cloud::watch')))
    expect(text()).toContain('iii-cloud::watch')
    expect(text()).toContain('Deploying')
    expect(text()).toContain('Workers: 2 of 3 ready')
    expect(text()).toMatch(/Release command · 22(\.0)?s/)
    // A deployment going is followed in the Deploy place.
    await click(btn('Follow'))
    expect(h.open).toHaveBeenCalledWith({
      pageId: 'iii-cloud',
      context: { at: 'deploy', env: cardRecords.watch.environment_id, deploy: cardRecords.watch.id },
    })
    await show(renderer.tryRender(env(wrap(cardRecords.failed), 'iii-cloud::deploy')))
    expect(text()).toContain('Failed')
    expect(text()).toContain('Workers: registry ran out of memory')
    await click(btn('Open the deployment'))
    expect(h.open).toHaveBeenCalledWith({
      pageId: 'iii-cloud',
      context: { at: 'env', env: cardRecords.failed.environment_id, deploy: cardRecords.failed.id },
    })
  })
})

describe('configuration form', () => {
  it('shows defaults without writing them, edits surgically, keeps unknown keys', async () => {
    const h = fakeHost(base)
    setup(h.host)
    const Form = (h.host.configForms.register as ReturnType<typeof vi.fn>).mock.calls[0][1]
    const onChange = vi.fn()
    await act(async () =>
      root.render(
        <Form id="iii-cloud" schema={null} value={{ merge_method: 'rebase', future: { a: 1 } }} onChange={onChange} />,
      ),
    )
    await flush()
    expect(text()).toContain('Cloud URL')
    expect(text()).toContain('detected 0.27.0 · cloud engine 0.27.0')
    expect(text()).toContain('Signed in as you@example.com · browser')
    expect((el.querySelector('#iii-cloud-url') as HTMLInputElement).value).toBe('https://cloud.iii.dev')
    expect((el.querySelector('#iii-cloud-timeout') as HTMLInputElement).value).toBe('900')
    expect((el.querySelector('#iii-cloud-pg') as HTMLInputElement).value).toBe('pgvector/pgvector:pg17')
    const pg = el.querySelector('#iii-cloud-pg') as HTMLInputElement
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set?.call(pg, 'postgres:16')
      pg.dispatchEvent(new Event('input', { bubbles: true }))
    })
    expect(onChange).toHaveBeenLastCalledWith({
      merge_method: 'rebase',
      future: { a: 1 },
      postgres_image: 'postgres:16',
    })
  })
})

describe('the hub keeps what a page that is gone cannot', () => {
  const refusal = { code: 'MERGE_CONFLICT', message: 'x' }
  const grab = async (host: Host) => {
    const ref: { current: ReturnType<typeof useActions> | null } = { current: null }
    const Caller = () => {
      ref.current = useActions(host)
      return null
    }
    await act(async () => root.render(<Caller />))
    return ref as { current: ReturnType<typeof useActions> }
  }
  const staging: Status = {
    ...base,
    project: {
      ...base.project!,
      environment: { ...base.project!.environment!, id: 'e-staging', name: 'staging' },
    },
  }

  it("a check record of the environment replaces the refusal kept for it; another environment's record does not", async () => {
    const h = fakeHost(base)
    const hub = hubOf(h.host)
    hub.subscribe(() => undefined)
    hub.setCheckError(ENV.id, refusal)
    hub.setCheckError('e-other', refusal)
    await act(async () => hub.refresh(false))
    const handler = h.handlers.get('iii::iii-cloud-ui::events::console-test')
    handler?.({ kind: 'check', id: 'c9', terminal: true, summary: '', record: { ...ready, environment_id: ENV.id } })
    expect(hub.checkError(ENV.id)).toBeNull()
    expect(hub.checkError('e-other')).not.toBeNull()
  })

  it('a refusal is kept for the environment the call was made for, not the one selected when it fails', async () => {
    let status: Status = base
    let reject: (e: Error) => void = () => undefined
    const h = fakeHost(() => status, {
      'iii-cloud::check': () => new Promise((_, r) => (reject = r)),
    })
    const hub = hubOf(h.host)
    await act(async () => hub.refresh(false))
    const a = await grab(h.host)
    let done: Promise<unknown> = Promise.resolve()
    await act(async () => {
      done = a.current.call('check', 'iii-cloud::check', {})
    })
    status = staging
    await act(async () => hub.refresh(true))
    await act(async () => {
      reject(Object.assign(new Error('x'), { code: 'MERGE_CONFLICT' }))
      await done
    })
    expect(hub.checkError(ENV.id)).not.toBeNull()
    expect(hub.checkError('e-staging')).toBeNull()
  })

  const useCall = (a: Awaited<ReturnType<typeof grab>>, env: string, changes: boolean) =>
    act(async () => {
      await a.current.call('use', 'iii-cloud::project::use', { environment_id: env }, { changesSource: changes })
    })

  it('a project::use that changes the source marks the environment: its check is stale, any other check is not', async () => {
    const h = fakeHost(base, { 'iii-cloud::project::use': {} })
    const hub = hubOf(h.host)
    await act(async () => hub.refresh(false))
    const a = await grab(h.host)
    await useCall(a, ENV.id, true)
    expect(hub.movedFrom(ENV.id)).toBe(base.check?.id)
    expect(hub.movedFrom('e-other')).toBeNull()
  })

  it('a plain re-selection (CloudPage for a route) marks nothing, and never erases a recorded move', async () => {
    const h = fakeHost(base, { 'iii-cloud::project::use': {} })
    const hub = hubOf(h.host)
    await act(async () => hub.refresh(false))
    const a = await grab(h.host)
    await useCall(a, ENV.id, false)
    expect(hub.movedFrom(ENV.id)).toBeNull()
    await useCall(a, ENV.id, true)
    await useCall(a, ENV.id, false)
    expect(hub.movedFrom(ENV.id)).toBe(base.check?.id)
  })

  it("the move is of that environment's latest check (not the selected one's), under the environment the Project says", async () => {
    const devCheck = {
      id: 'chk-dev-1',
      environment_id: 'e-dev',
      commit: 'a',
      checked_at: '',
      duration_ms: 1,
      ready: true,
      problems: 0,
      handoff_id: null,
      manifest: null,
    }
    // staging is selected (status.check is its check); dev's is in the list. The use returns dev (the payload names none).
    const status: Status = {
      ...staging,
      check: { ...base.check!, id: 'chk-stg', environment_id: 'e-staging' },
      checks: [devCheck],
    }
    const h = fakeHost(status, { 'iii-cloud::project::use': { environment: { id: 'e-dev' } } })
    const hub = hubOf(h.host)
    await act(async () => hub.refresh(false))
    const a = await grab(h.host)
    await act(async () => {
      await a.current.call('use', 'iii-cloud::project::use', { path: '/p' }, { changesSource: true })
    })
    expect(hub.movedFrom('e-dev')).toBe('chk-dev-1')
    expect(hub.movedFrom('e-staging')).toBeNull()
  })

  it('the environment a use selected lasts only until the next status, whatever it reports', async () => {
    const h = fakeHost(base)
    const hub = hubOf(h.host)
    await act(async () => hub.refresh(false))
    // A use selected e-x (its call named it); the status that follows reports another one (the worker fell back).
    hub.used('e-x', false)
    expect(hub.selectedEnv()).toBe('e-x')
    await act(async () => hub.refresh(true))
    expect(hub.selectedEnv()).toBe(ENV.id)
  })

  it("a status read ends a recorded move once the environment's latest check is another one", async () => {
    let status: Status = base
    const h = fakeHost(() => status, { 'iii-cloud::project::use': {} })
    const hub = hubOf(h.host)
    await act(async () => hub.refresh(false))
    const a = await grab(h.host)
    await useCall(a, ENV.id, true)
    await act(async () => hub.refresh(true))
    expect(hub.movedFrom(ENV.id)).toBe(base.check?.id)
    status = { ...base, check: { ...base.check!, id: 'chk-after-read' } }
    await act(async () => hub.refresh(true))
    expect(hub.movedFrom(ENV.id)).toBeNull()
  })

  it('a use that changes the source is marked before it is sent: a timeout keeps the mark, a refusal takes it back', async () => {
    let answer: (v?: unknown) => void = () => undefined
    let fail: (e: Error) => void = () => undefined
    const h = fakeHost(base, {
      'iii-cloud::project::use': () =>
        new Promise((res, rej) => {
          answer = res
          fail = rej
        }),
    })
    const hub = hubOf(h.host)
    await act(async () => hub.refresh(false))
    const a = await grab(h.host)
    let sent: Promise<unknown> = Promise.resolve()
    await act(async () => {
      sent = a.current.call('use', 'iii-cloud::project::use', { environment_id: ENV.id }, { changesSource: true })
    })
    expect(hub.movedFrom(ENV.id)).toBe(base.check?.id) // marked while the worker is still at it
    await act(async () => {
      fail(Object.assign(new Error('timed out'), { code: 'TIMEOUT' }))
      await sent
    })
    expect(hub.movedFrom(ENV.id)).toBe(base.check?.id) // the page gave up; the worker may have finished it
    hub.restoreMove(ENV.id, null)
    await act(async () => {
      sent = a.current.call('use', 'iii-cloud::project::use', { environment_id: ENV.id }, { changesSource: true })
    })
    await act(async () => {
      fail(Object.assign(new Error('no'), { code: 'ENVIRONMENT_DELETED' }))
      await sent
    })
    expect(hub.movedFrom(ENV.id)).toBeNull() // the worker answered that it refused
    answer()
  })

  it('the hub takes its storage listener off when the last subscriber leaves, not before', () => {
    const add = vi.spyOn(window, 'addEventListener')
    const remove = vi.spyOn(window, 'removeEventListener')
    const ours = (spy: typeof add) => spy.mock.calls.filter((c) => c[0] === 'storage').length
    const hub = hubOf(fakeHost(base).host)
    const first = hub.subscribe(() => undefined)
    const second = hub.subscribe(() => undefined)
    expect(ours(add)).toBe(1)
    first()
    expect(ours(remove)).toBe(0)
    second()
    expect(ours(remove)).toBe(1)
    add.mockRestore()
    remove.mockRestore()
  })

  it("another tab's mark (a storage event) is read again by this one", () => {
    const hub = hubOf(fakeHost(base).host)
    const seen = vi.fn()
    hub.subscribe(seen)
    window.dispatchEvent(new StorageEvent('storage', { key: 'iii-cloud:moved:e1' }))
    expect(seen).toHaveBeenCalled()
    seen.mockClear()
    window.dispatchEvent(new StorageEvent('storage', { key: 'someone-else' }))
    expect(seen).not.toHaveBeenCalled()
  })

  it('liveOf: no current row but something was live (a rollback beyond the first page) is Live with no sha; only failed rows is nothing live', () => {
    const row = (status: string) =>
      ({ id: 'x', number: 1, status, current: false, commit: { sha: 'abc' } }) as unknown as CloudDeployment
    // A replaced row never served: push #1 waits, push #2 replaces it, #2 is canceled: nothing is live.
    expect(liveOf([row('canceled'), row('replaced')])).toMatchObject({ label: 'Nothing live yet' })
    expect(liveOf([row('replaced')])).toMatchObject({ label: 'Nothing live yet' })
    expect(liveOf([row('ready')])).toMatchObject({ label: 'Live', sha: null })
    expect(liveOf([row('failed')])).toMatchObject({ label: 'Last deployment failed' })
    expect(liveOf([row('canceled')])).toMatchObject({ label: 'Nothing live yet' })
    expect(liveOf([{ ...row('ready'), current: true }])).toMatchObject({ label: 'Live', sha: 'abc' })
  })

  it('liveOf and the footer share one rule: a first page of replaced rows with a next page may hide what serves', () => {
    const row = (status: string) =>
      ({ id: 'x', number: 1, status, current: false, commit: { sha: 'abc' } }) as unknown as CloudDeployment
    const page = [row('canceled'), row('replaced'), row('replaced')]
    // dev serves #3 beyond the first page: the screens say Live (no sha), as the footer says it serves something unseen
    expect(liveOf(page, 'next-page')).toMatchObject({ label: 'Live', sha: null })
    expect(listMayHideServing({ deployments: page, next_cursor: 'next-page' })).toBe(true)
    // without a next page the same rows mean nothing was ever live, for both
    expect(liveOf(page, null)).toMatchObject({ label: 'Nothing live yet' })
    expect(listMayHideServing({ deployments: page, next_cursor: null })).toBe(false)
  })

  it('a newer check ends the move; the move and the stop survive a reload (only storage carries them)', async () => {
    // A working storage (this test environment's is absent) and a reload that forgets everything but it.
    const store = new Map<string, string>()
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
      removeItem: (k: string) => void store.delete(k),
    })
    const h = fakeHost(base, { 'iii-cloud::project::use': {} })
    const hub = hubOf(h.host)
    hub.subscribe(() => undefined)
    await act(async () => hub.refresh(false))
    const a = await grab(h.host)
    await useCall(a, ENV.id, true)
    hub.markStopped(ENV.id, true)
    forgetHubMemory()
    const fresh = hubOf(fakeHost(base).host)
    expect(fresh.movedFrom(ENV.id)).toBe(base.check?.id)
    expect(fresh.stopped(ENV.id)).toBe(true)
    h.handlers.get('iii::iii-cloud-ui::events::console-test')?.({
      kind: 'check',
      id: 'chk-newer',
      terminal: true,
      summary: '',
      record: { ...base.check!, id: 'chk-newer', environment_id: ENV.id },
    })
    expect(hub.movedFrom(ENV.id)).toBeNull()
    fresh.markStopped(ENV.id, false)
    forgetHubMemory()
    expect(hubOf(fakeHost(base).host).stopped(ENV.id)).toBe(false)
    expect(store.size).toBe(0)
    vi.unstubAllGlobals()
  })

  it('a recorded stop IS in storage; once cleared, nothing is left to carry over', () => {
    const store = new Map<string, string>()
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
      removeItem: (k: string) => void store.delete(k),
    })
    hubOf(fakeHost(base).host).markStopped(ENV.id, true)
    expect([...store.keys()].some((k) => k.includes(ENV.id))).toBe(true)
    hubOf(fakeHost(base).host).markStopped(ENV.id, false)
    forgetHubMemory()
    expect(store.size).toBe(0)
    expect(hubOf(fakeHost(base).host).stopped(ENV.id)).toBe(false)
    vi.unstubAllGlobals()
  })

  it("with iii cloud's list read and no current row, no deployment is named as serving", () => {
    const row = (id: string, current: boolean) => ({ id, number: 1, current }) as unknown as CloudDeployment
    // The worker knows a ready deployment (#older); the list (a rollback beyond its first page) has no current row.
    const known: Status = { ...base, deploys: [{ ...following, outcome: 'ready', environment_id: ENV.id }] }
    expect(servingId(known, ENV.id, [row('a', false)])).toBeNull()
    expect(servingId(known, ENV.id, [row('a', true), row('b', false)])).toBe('a')
    // without the list, the worker's own newest ready deployment stands in
    expect(servingId(known, ENV.id)).toBe(following.id)
  })
})

describe('Host without optional slots', () => {
  it('renders the page with no panels, chat or commands', async () => {
    const h = fakeHost(base)
    const bare = { ...h.host, panels: undefined, chat: undefined, commands: undefined } as unknown as Host
    await act(async () =>
      root.render(<CloudPage host={bare} panelSide="left" tabId="" {...ctx({ at: 'deploy', env: ENV.id })} />),
    )
    await flush()
    expect(el.querySelector('h2.iii-cloud-h')?.textContent).toBe('2 things need you · 2 for your agent to fix')
  })
})
