// Model, plain messages, the hub, chat cards and the form: no area owns these.
import type { FunctionCallMessage, Host } from '@iii-dev/console-ui'
import { act } from 'react'
import { describe, expect, it, vi } from 'vitest'
import setup from '../../page'
import { CloudPage } from '../CloudPage'
import { cardRecords } from '../fixtures'
import { base, following, ready, secretRequests } from '../fixtures/base'
import { configDir, opensToPublic, plainMessage, settingWords, whoLine, workerError } from '../model'
import type { ChangeEvent } from '../types'
import { fakeHost } from './host'
import { btn, click, ctx, el, flush, mount, root, text, usePage } from './util'

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
    await mount(
      { ...s, check: { ...s.check!, run } },
      { panelContext: { id: 1, pageId: 'iii-cloud', context: { at: 'deploy', env: '', step: 'check' } } },
    )
    expect(text()).toContain('iii cloud had a problem')
    expect(text()).not.toContain('CLOUD_ERROR')
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
  it('upserts a deploy event into the page and observes once per tab', async () => {
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

  it('check not ready opens the check; ready opens the run, both its own check on the Checks tab of its environment', async () => {
    const { h, renderer } = await registered()
    await show(renderer.tryRender(env(wrap(cardRecords.checkBlocked), 'iii-cloud::check')))
    expect(text()).toContain('Not ready')
    expect(text()).toContain('registry/production')
    expect(text()).toContain('commit 3c4d2e0')
    expect(text()).toContain('iii cloud check: 3 to fix, at least')
    expect(text()).toContain('Checks here: 4 problems')
    expect(text()).toContain('2 need your decision')
    const checks = (id: string) => ({
      at: 'env',
      env: cardRecords.checkBlocked.environment_id,
      tab: 'checks',
      check: id,
    })
    await click(btn('Open the check'))
    expect(h.open).toHaveBeenCalledWith({ pageId: 'iii-cloud', context: checks(cardRecords.checkBlocked.id) })
    await show(renderer.tryRenderDisplay(env(wrap(cardRecords.checkReady), 'iii-cloud::check')))
    expect(text()).toContain('iii cloud check is clean')
    expect(text()).toContain('Ran here like the cloud: 3 of 3 workers, 14 routes, healthcheck 200')
    await click(btn('See the run'))
    expect(h.open).toHaveBeenLastCalledWith({ pageId: 'iii-cloud', context: checks(cardRecords.checkReady.id) })
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

  it('secret card never shows a value and opens Settings', async () => {
    const { h, renderer } = await registered()
    await show(renderer.tryRender(env(wrap(cardRecords.secret), 'iii-cloud::secret')))
    expect(text()).toContain('SESSION_SECRET')
    expect(text()).toContain('type it on the iii cloud page; the value never comes back to this chat')
    await click(btn('Open settings'))
    expect(h.open).toHaveBeenCalledWith({
      pageId: 'iii-cloud',
      context: { at: 'env', env: cardRecords.secret.environment_id, tab: 'settings' },
    })
  })

  it('watch card: live stage lines, then failed; it opens the deploy on its environment', async () => {
    const { h, renderer } = await registered()
    await show(renderer.tryRender(env(wrap(cardRecords.watch), 'iii-cloud::watch')))
    expect(text()).toContain('iii-cloud::watch')
    expect(text()).toContain('Deploying')
    expect(text()).toContain('Workers: 2 of 3 ready')
    expect(text()).toMatch(/Release command · 22(\.0)?s/)
    await show(renderer.tryRender(env(wrap(cardRecords.failed), 'iii-cloud::deploy')))
    expect(text()).toContain('Failed')
    expect(text()).toContain('Workers: registry ran out of memory')
    await click(btn('Open deploy'))
    expect(h.open).toHaveBeenCalledWith({
      pageId: 'iii-cloud',
      context: { at: 'env', env: cardRecords.failed.environment_id, tab: 'deploys', deploy: cardRecords.failed.id },
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
  it('renders the page with no panels, chat or commands', async () => {
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
