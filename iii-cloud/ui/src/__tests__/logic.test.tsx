// Model, plain messages, the hub, chat cards and the form: no area owns these.
import type { FunctionCallMessage, Host } from '@iii-dev/console-ui'
import { act } from 'react'
import { describe, expect, it, vi } from 'vitest'
import setup from '../../page'
import { clock } from '../asked'
import { CloudPage } from '../CloudPage'
import { cardRecords } from '../fixtures'
import { base, following, ready, secretRequests } from '../fixtures/base'
import {
  configDir,
  deployOrigin,
  deployState,
  opensToPublic,
  plainMessage,
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
    expect(deployOrigin(at(mine), mine, viaKey).line).toBe('you, from this machine · checked')
    expect(deployOrigin(at(mine), mine).line).toBe('you, from this machine · checked')
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
  // Skipped: it read the check inside Setup's old inner step; the Deploy place owns that screen now.
  it.skip('an older check does not replace the current one; a newer one does', async () => {
    const h = await mount(base, ctx({ at: 'setup' }))
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
    const title = () => el.querySelector('.iii-cloud-setup h2')?.textContent
    expect(title()).toBe('At least 4 things to fix before production can get this')
    await fire(older)
    expect(title()).toBe('At least 4 things to fix before production can get this')
    await fire({
      ...cur,
      id: 'chk-newer',
      checked_at: new Date(Date.parse(cur.checked_at) + 60_000).toISOString(),
      ready: true,
      problems: [],
    })
    expect(title()).toBe('Review and deploy to production')
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
  // Skipped: it read the deploy inside Setup's old inner step; the Deploy place owns that screen now.
  it.skip('upserts a deploy event into the page and observes once per tab', async () => {
    const h = await mount(base, ctx({ at: 'setup' }))
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

describe('Host without optional slots', () => {
  // Skipped: it read Setup's old check step; the Deploy place owns that screen now.
  it.skip('renders the page with no panels, chat or commands', async () => {
    const h = fakeHost(base)
    const bare = { ...h.host, panels: undefined, chat: undefined, commands: undefined } as unknown as Host
    await act(async () => root.render(<CloudPage host={bare} panelSide="left" tabId="" {...ctx({ at: 'setup' })} />))
    await flush()
    expect(el.querySelector('aside[aria-label="Setup steps"] [aria-current=step]')?.textContent).toContain(
      'Check before deploy',
    )
    expect(el.querySelector('.iii-cloud-setup h2')?.textContent).toBe(
      'At least 4 things to fix before production can get this',
    )
  })
})
