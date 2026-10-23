/** The open tab (design §8.2-§8.4): the live view's props, the address bar, loading, history, menus, handoff. */
import type { TestHost } from '@iii-dev/native-ui/testing'
import { describe, expect, it } from 'vitest'
import { callsOf, exists, FakeBrowser, shown, start, subsOf, TABS } from './fake'

/** The RemoteScreen wire of the browser, as design §6.7 writes it. */
const WIRE = {
  frames: { trigger: 'browser::frame-event', config: { session_id: '$session' } },
  seed: { fn: 'browser::frame', args: { session_id: '$session', since_frame: '$seq' } },
  start: {
    fn: 'browser::screencast::start',
    args: { session_id: '$session', viewer_id: '$viewer', lease_ms: 20000, scale: '$scale' },
    renewMs: 6000,
  },
  stop: { fn: 'browser::screencast::stop', args: { session_id: '$session', viewer_id: '$viewer' } },
  resize: {
    fn: 'browser::resize',
    args: {
      session_id: '$session',
      width: '$w',
      height: '$h',
      device_scale_factor: '$dpr',
      mobile: '$mobile',
      fit: true,
    },
    debounceMs: 180,
    minChangeDp: 4,
  },
  input: {
    fn: 'browser::act',
    tap: { session_id: '$session', action: 'click', x: '$x', y: '$y', button: 'left', click_count: '$count' },
    scroll: { session_id: '$session', action: 'scroll', x: '$x', y: '$y', delta_x: '$dx', delta_y: '$dy' },
    type: { session_id: '$session', action: 'type', text: '$text' },
    key: { session_id: '$session', action: 'press', key: '$key' },
  },
}

/** Answers the action menus in order. */
function answers(...list: number[]) {
  return { actionMenu: () => list.shift() ?? -1 }
}

async function opened(id = 't1', options: Parameters<typeof start>[0] = {}) {
  const started = await start({ tabs: TABS, ...options })
  await started.host.itemPress('tabs', id)
  return started
}

const loading = (host: TestHost) => host.find('loading').props?.visible !== false

describe('the live view', () => {
  it('opens the tab in a RemoteScreen with the browser wire, as a phone', async () => {
    const { host } = await opened('t1')
    const screen = host.find('screen').props ?? {}
    expect(screen).toMatchObject({ session: 't1', placeholder: 'Loading…', weight: 1 })
    expect(screen.wire).toEqual(WIRE)
    expect(screen.mobile ?? true).toBe(true)
    expect(screen.mode ?? 'interact').toBe('interact')
    expect(screen.enabled ?? true).toBe(true)
    expect(screen.keyboard ?? false).toBe(false)
    expect(screen.events).toEqual(['longPress', 'frame', 'error', 'keyboard', 'ready'])
    expect(host.find('tab').props).toMatchObject({
      title: 'Example Domain',
      subtitle: 'example.com',
      subtitleMono: true,
    })
    // The page's zoom is read when the tab is awake.
    expect(callsOf(host, 'browser::zoom')).toEqual([{ session_id: 't1', action: 'read' }])
  })

  it('a sleeping tab says it is waking; a read-only one is only watched', async () => {
    const { host } = await start({
      tabs: [...TABS, { session_id: 'ro', url: 'https://docs.example/', title: 'Docs', read_only: true }],
    })
    await host.itemPress('tabs', 't2')
    expect(host.find('screen').props).toMatchObject({ session: 't2', placeholder: 'Waking the tab…' })
    expect(callsOf(host, 'browser::zoom')).toEqual([])
    await host.itemPress('tabs', 'ro')
    expect(host.find('screen').props).toMatchObject({ session: 'ro', mode: 'view' })
    expect(host.find('keyboard').props?.visible).toBe(false)
  })

  it('a stopped live view says why, and Retry turns it off and on', async () => {
    const { host } = await opened('t1')
    await host.emit('screen', 'error', ['the screencast could not start'])
    expect(host.find('view-error').props).toMatchObject({
      headline: 'The live view stopped',
      detail: 'the screencast could not start',
    })
    await host.press('view-retry')
    expect(host.find('screen').props?.enabled).toBe(false)
    await host.tick(100)
    expect(host.find('screen').props?.enabled).toBe(true)
    expect(exists(host, 'view-error')).toBe(false)
  })

  it('the page keyboard: the bar button, the view closing it, the bar hidden meanwhile on compact', async () => {
    const { host } = await opened('t1')
    await host.press('keyboard')
    expect(host.find('screen').props?.keyboard).toBe(true)
    expect(host.find('bars').props?.visible).toBe(false)
    await host.emit('screen', 'keyboard', [false])
    expect(host.find('screen').props?.keyboard).toBe(false)
    expect(host.find('bars').props?.visible ?? true).toBe(true)
  })

  it('a long press on the page: right-click, type here, copy the URL', async () => {
    const { host } = await opened('t1', { ui: answers(0, 1, 2) })
    await host.emit('screen', 'longPress', [120, 340])
    expect(host.menus.at(-1)).toMatchObject({
      actions: [{ label: 'Right-click here' }, { label: 'Type here' }, { label: 'Copy page URL' }],
    })
    expect(callsOf(host, 'browser::act')).toEqual([
      { session_id: 't1', action: 'click', x: 120, y: 340, button: 'right' },
    ])
    await host.emit('screen', 'longPress', [10, 20])
    expect(callsOf(host, 'browser::act').at(-1)).toEqual({
      session_id: 't1',
      action: 'click',
      x: 10,
      y: 20,
      button: 'left',
    })
    expect(host.find('screen').props?.keyboard).toBe(true)
    await host.emit('screen', 'longPress', [10, 20])
    expect(host.clipboard).toEqual(['https://example.com/'])
  })
})

describe('the address bar', () => {
  it('shows the address without its scheme, with the lock on https', async () => {
    const { host } = await opened('t1')
    expect(host.find('address').props).toMatchObject({ text: 'example.com/', icon: 'lock' })
    expect(host.findAll('address-field')).toEqual([])
  })

  it('submitting navigates with the bar shown until the page loads', async () => {
    const { host, fake } = await opened('t1')
    fake.hold('browser::navigate')
    await host.press('address')
    expect(host.find('address-field').props).toMatchObject({ value: 'https://example.com/', autofocus: true })
    await host.type('address-field', 'iii.dev/docs')
    await host.submit('address-field')
    expect(callsOf(host, 'browser::navigate')).toEqual([{ session_id: 't1', url: 'https://iii.dev/docs' }])
    const call = host.requests.find((r) => r.args.fn === 'browser::navigate')
    expect(call?.args.timeoutMs).toBe(45_000)
    expect(loading(host)).toBe(true)
    expect(host.find('address').props?.text).toBe('iii.dev/docs')
    fake.release('browser::navigate')
    await host.settle()
    expect(loading(host)).toBe(false)
    expect(host.find('tab').props).toMatchObject({ title: 'Loaded page', subtitle: 'iii.dev' })
  })

  it('takes URLs only, like the web bar: a local host gets http, a scheme passes', async () => {
    const { host } = await opened('t1')
    for (const [typed, url] of [
      ['localhost:3000/app', 'http://localhost:3000/app'],
      ['192.168.1.20', 'http://192.168.1.20'],
      ['ftp://files.example/x', 'ftp://files.example/x'],
      ['  example.org ', 'https://example.org'],
    ]) {
      await host.press('address')
      await host.type('address-field', typed)
      await host.press('address-go')
      expect(callsOf(host, 'browser::navigate').at(-1)?.url).toBe(url)
    }
  })

  it('Cancel, or the field losing focus, goes back to the pill without navigating', async () => {
    const { host } = await opened('t1')
    await host.press('address')
    await host.type('address-field', 'half typed')
    await host.press('address-cancel')
    expect(exists(host, 'address-field')).toBe(false)
    await host.press('address')
    await host.emit('address-field', 'focus', [true])
    await host.emit('address-field', 'focus', [false])
    await host.tick(250)
    expect(exists(host, 'address-field')).toBe(false)
    expect(callsOf(host, 'browser::navigate')).toEqual([])
  })

  it('a page that does not load shows a panel with Retry', async () => {
    const fake = new FakeBrowser(TABS)
    let fail = true
    fake.overrides['browser::navigate'] = (p) =>
      fail
        ? { ok: false, url: 'chrome-error://chromewebdata/', error: 'net::ERR_NAME_NOT_RESOLVED', timed_out: false }
        : { ok: true, url: p.url, title: 'Back up' }
    const { host } = await opened('t1', { fake })
    await host.press('address')
    await host.type('address-field', 'nope.invalid')
    await host.submit('address-field')
    expect(host.find('load-error').props).toMatchObject({
      variant: 'alert',
      headline: "Couldn't load nope.invalid",
      detail: 'net::ERR_NAME_NOT_RESOLVED',
    })
    expect(host.find('address').props?.text).toBe('nope.invalid')
    fail = false
    await host.press('load-retry')
    expect(callsOf(host, 'browser::navigate').map((p) => p.url)).toEqual([
      'https://nope.invalid',
      'https://nope.invalid',
    ])
    expect(exists(host, 'load-error')).toBe(false)
    expect(host.find('tab').props?.title).toBe('Back up')
  })

  it('a refused address (scheme or origin policy) is a toast in words', async () => {
    const fake = new FakeBrowser(TABS)
    fake.overrides['browser::navigate'] = () => {
      throw Object.assign(new Error("scheme 'file' is not allowed (allowed_schemes: http, https)"), {
        code: 'invocation_failed',
      })
    }
    const { host } = await opened('t1', { fake })
    await host.press('address')
    await host.type('address-field', 'file:///etc/hosts')
    await host.submit('address-field')
    expect(host.toasts.at(-1)).toEqual({
      text: "scheme 'file' is not allowed (allowed_schemes: http, https)",
      kind: 'error',
    })
    expect(host.find('address').props?.text).toBe('example.com/')
    expect(loading(host)).toBe(false)
  })

  it('back and forward wait as a spinner; nothing to move to says so', async () => {
    const fake = new FakeBrowser(TABS)
    const { host } = await opened('t1', { fake })
    fake.hold('browser::history')
    await host.press('back')
    expect(exists(host, 'back')).toBe(false)
    expect(exists(host, 'back-busy')).toBe(true)
    expect(host.find('forward').props?.enabled).toBe(false)
    fake.release('browser::history', { ok: true, url: 'https://example.com/', moved: false })
    await host.settle()
    expect(exists(host, 'back')).toBe(true)
    expect(host.toasts.at(-1)).toEqual({ text: 'No earlier page', kind: 'info' })
    fake.overrides['browser::history'] = () => ({ ok: true, url: 'https://example.com/', moved: false })
    await host.press('forward')
    expect(host.toasts.at(-1)).toEqual({ text: 'No later page', kind: 'info' })
    fake.overrides['browser::history'] = () => ({ ok: true, url: 'https://example.com/next', moved: true })
    await host.press('forward')
    expect(host.toasts).toHaveLength(2)
    expect(host.find('address').props?.text).toBe('example.com/next')
    expect(callsOf(host, 'browser::history').map((p) => p.action)).toEqual(['back', 'forward', 'forward'])
  })

  it('moves between the bottom (compact) and the top (wide) as one node, keeping a half-typed address', async () => {
    const { host } = await opened('t1')
    const bottom = () => host.find('tab').slots?.bottom ?? []
    const top = () => host.find('bars-top').slots?.children ?? []
    expect(bottom().map((n) => n.props?.testTag)).toEqual(['bars'])
    expect(top()).toEqual([])
    await host.press('address')
    await host.type('address-field', 'half typed')
    const field = host.find('address-field').id
    const from = host.ops.length
    await host.env({ windowClass: 'wide' })
    expect(bottom()).toEqual([])
    expect(top().map((n) => n.props?.testTag)).toEqual(['bars'])
    expect(host.find('address-field')).toMatchObject({ id: field, props: { value: 'half typed' } })
    expect(host.ops.slice(from).some((e) => e[0] === 6)).toBe(false)
    await host.env({ windowClass: 'compact' })
    expect(bottom().map((n) => n.props?.testTag)).toEqual(['bars'])
    expect(host.find('address-field')).toMatchObject({ id: field, props: { value: 'half typed' } })
  })
})

describe('loading without a loading event', () => {
  it('a navigation the page made shows the bar until the next title, or 3 s', async () => {
    const { host } = await opened('t1')
    expect(loading(host)).toBe(false)
    await host.emitTrigger('browser::navigated', { session_id: 't1', url: 'https://example.com/clicked', timestamp: 1 })
    expect(loading(host)).toBe(true)
    expect(host.find('address').props?.text).toBe('example.com/clicked')
    await host.emitTrigger('browser::session-updated', {
      session_id: 't1',
      active: true,
      url: 'https://example.com/clicked',
      title: 'Clicked',
      timestamp: 2,
    })
    expect(loading(host)).toBe(false)
    expect(host.find('tab').props?.title).toBe('Clicked')

    await host.emitTrigger('browser::navigated', { session_id: 't1', url: 'https://example.com/again', timestamp: 3 })
    expect(loading(host)).toBe(true)
    await host.tick(2999)
    expect(loading(host)).toBe(true)
    await host.tick(1)
    expect(loading(host)).toBe(false)
  })

  it('another tab navigating does not show the bar', async () => {
    const { host } = await opened('t1')
    await host.emitTrigger('browser::navigated', { session_id: 't3', url: 'https://github.com/', timestamp: 1 })
    expect(loading(host)).toBe(false)
  })
})

describe('the mobile user agent (design §9.3)', () => {
  it('reloads once when the host says the resize changed the mode', async () => {
    const { host } = await opened('t1')
    await host.emit('screen', 'frame', [412, 780, true])
    expect(callsOf(host, 'browser::history')).toEqual([{ session_id: 't1', action: 'reload' }])
    await host.emit('screen', 'frame', [412, 700, false])
    expect(callsOf(host, 'browser::history')).toHaveLength(1)
  })

  it('without that word, reloads a waking tab once its frames narrow to the view', async () => {
    const { host } = await opened('t2')
    await host.emit('screen', 'frame', [1280, 800])
    expect(callsOf(host, 'browser::history')).toEqual([])
    await host.emit('screen', 'frame', [412, 780])
    expect(callsOf(host, 'browser::history')).toEqual([{ session_id: 't2', action: 'reload' }])
    await host.emit('screen', 'frame', [400, 780])
    expect(callsOf(host, 'browser::history')).toHaveLength(1)
  })

  it('never reloads a blank new tab, nor one already the size of the view', async () => {
    const { host } = await start({ tabs: TABS })
    await host.press('new-tab')
    await host.emit('screen', 'frame', [1280, 800])
    await host.emit('screen', 'frame', [412, 780])
    await host.itemPress('tabs', 't1')
    await host.emit('screen', 'frame', [412, 780])
    expect(callsOf(host, 'browser::history')).toEqual([])
  })
})

describe('a mode change during a navigation of the app', () => {
  const reloads = (host: TestHost, id: string) =>
    callsOf(host, 'browser::history').filter((p) => p.session_id === id && p.action === 'reload')

  /** Opens `id` and submits `typed` in the address bar, with the navigate held open. */
  async function navigating(id: string, typed: string, options: Parameters<typeof start>[0] = {}) {
    const started = await opened(id, options)
    started.fake.hold('browser::navigate')
    await started.host.press('address')
    await started.host.type('address-field', typed)
    await started.host.submit('address-field')
    expect(started.fake.waiting('browser::navigate')).toHaveLength(1)
    return started
  }

  it('reloads exactly once, after the navigation settles, however many changes came meanwhile', async () => {
    const { host, fake } = await navigating('t1', 'iii.dev')
    await host.emit('screen', 'frame', [412, 780, true])
    await host.emit('screen', 'frame', [412, 700, true])
    expect(reloads(host, 't1')).toEqual([])
    fake.release('browser::navigate')
    await host.settle()
    expect(reloads(host, 't1')).toHaveLength(1)
    // After the navigate's answer: the page it loaded is the one reloaded.
    const order = host.requests.map((r) => r.args.fn)
    expect(order.lastIndexOf('browser::history')).toBeGreaterThan(order.lastIndexOf('browser::navigate'))
    expect(loading(host)).toBe(false)
    await host.emit('screen', 'frame', [412, 700, false])
    expect(reloads(host, 't1')).toHaveLength(1)
  })

  it('still reloads once when the navigation fails', async () => {
    const { host, fake } = await navigating('t1', 'nope.invalid')
    await host.emit('screen', 'frame', [412, 780, true])
    fake.release('browser::navigate', {
      ok: false,
      url: 'chrome-error://chromewebdata/',
      error: 'net::ERR_NAME_NOT_RESOLVED',
    })
    await host.settle()
    expect(exists(host, 'load-error')).toBe(true)
    expect(reloads(host, 't1')).toHaveLength(1)
  })

  it('reloads at once when the tab is idle', async () => {
    const { host } = await opened('t1')
    await host.emit('screen', 'frame', [412, 780, true])
    expect(reloads(host, 't1')).toHaveLength(1)
  })

  it('never reloads a blank page, even when its navigation was refused', async () => {
    const fake = new FakeBrowser([{ session_id: 'b', url: 'about:blank' }])
    const { host } = await start({ fake })
    await host.itemPress('tabs', 'b')
    await host.emit('screen', 'frame', [412, 780, true])
    expect(reloads(host, 'b')).toEqual([])

    fake.hold('browser::navigate')
    await host.press('address')
    await host.type('address-field', 'file:///etc/hosts')
    await host.submit('address-field')
    await host.emit('screen', 'frame', [412, 780, true])
    fake.waiting('browser::navigate')[0].reject(new Error("scheme 'file' is not allowed"))
    await host.settle()
    expect(host.toasts.at(-1)).toEqual({ text: "scheme 'file' is not allowed", kind: 'error' })
    expect(host.find('address').props?.text).toBe('Type an address')
    expect(reloads(host, 'b')).toEqual([])
  })

  it('drops the reload when the tab closes before the navigation settles', async () => {
    const { host, fake } = await navigating('t1', 'iii.dev')
    await host.emit('screen', 'frame', [412, 780, true])
    await host.itemPress('tabs', 't1', 'tab-close')
    fake.release('browser::navigate')
    await host.settle()
    expect(reloads(host, 't1')).toEqual([])
  })

  it('drops it too when the tab is stopped elsewhere meanwhile', async () => {
    const { host, fake } = await navigating('t1', 'iii.dev')
    await host.emit('screen', 'frame', [412, 780, true])
    await host.emitTrigger('browser::session-stopped', { session_id: 't1', reason: 'crashed', timestamp: 1 })
    fake.release('browser::navigate')
    await host.settle()
    expect(reloads(host, 't1')).toEqual([])
  })
})

describe('the page menu', () => {
  const MENU = [
    'Find in page',
    'Zoom in',
    'Zoom out',
    'Actual size',
    'Screenshot',
    'Console',
    'Network',
    'History',
    'Copy URL',
    'Share URL',
    'Open in browser',
    'Clear site data…',
    'Close tab',
  ]

  it('lists the v0 actions, the zoom level as a hint', async () => {
    const { host } = await opened('t1', { ui: answers(-1) })
    await host.press('tab-menu')
    const menu = host.menus.at(-1) as {
      title: string
      actions: { label: string; hint?: string; destructive?: boolean }[]
    }
    expect(menu.title).toBe('Example Domain')
    expect(menu.actions.map((a) => a.label)).toEqual(MENU)
    expect(menu.actions[3].hint).toBe('100%')
    expect(menu.actions[11].destructive).toBe(true)
  })

  it('zooms, and applies the zoom again after each navigation', async () => {
    const { host } = await opened('t1', { ui: answers(1, 1, -1, 3) })
    await host.press('tab-menu')
    await host.press('tab-menu')
    await host.press('tab-menu')
    expect((host.menus.at(-1) as { actions: { hint?: string }[] }).actions[3].hint).toBe('125%')
    await host.emitTrigger('browser::navigated', { session_id: 't1', url: 'https://example.com/b', timestamp: 1 })
    expect(callsOf(host, 'browser::zoom').at(-1)).toEqual({ session_id: 't1', action: 'set', level: 125 })
    await host.press('tab-menu')
    expect(callsOf(host, 'browser::zoom').at(-1)).toEqual({ session_id: 't1', action: 'reset' })
    const before = callsOf(host, 'browser::zoom').length
    await host.emitTrigger('browser::navigated', { session_id: 't1', url: 'https://example.com/c', timestamp: 2 })
    expect(callsOf(host, 'browser::zoom')).toHaveLength(before)
  })

  it('find in page takes the bar: search after a pause, next, previous, close', async () => {
    const { host } = await opened('t1', { ui: answers(0) })
    await host.press('tab-menu')
    expect(exists(host, 'address-bar')).toBe(false)
    expect(host.find('find-field').props).toMatchObject({ autofocus: true, keyboard: 'search' })
    await host.type('find-field', 'domain')
    expect(callsOf(host, 'browser::find-in-page')).toEqual([])
    await host.tick(300)
    expect(callsOf(host, 'browser::find-in-page')).toEqual([{ session_id: 't1', query: 'domain', action: 'search' }])
    expect(host.find('find-count').props?.text).toBe('1/3')
    await host.press('find-next')
    expect(host.find('find-count').props?.text).toBe('2/3')
    await host.press('find-previous')
    expect(callsOf(host, 'browser::find-in-page').at(-1)?.action).toBe('previous')
    await host.press('find-close')
    expect(callsOf(host, 'browser::find-in-page').at(-1)).toEqual({ session_id: 't1', query: '', action: 'close' })
    expect(exists(host, 'address-bar')).toBe(true)
  })

  it('a screenshot is shared or saved to Downloads', async () => {
    const { host } = await opened('t1', { ui: answers(4, 0, 4, 1) })
    await host.press('tab-menu')
    expect(callsOf(host, 'browser::screenshot')).toEqual([{ session_id: 't1', format: 'png' }])
    expect(host.menus.at(-1)).toMatchObject({
      title: 'Screenshot',
      actions: [{ label: 'Share' }, { label: 'Save to Downloads' }],
    })
    expect(host.shared.at(-1)).toMatchObject({ data: 'iVBORw0KGgo=', mime: 'image/png' })
    expect(String((host.shared.at(-1) as { filename: string }).filename)).toMatch(
      /^screenshot-example\.com-\d{8}-\d{6}\.png$/,
    )
    await host.press('tab-menu')
    expect(host.saved).toHaveLength(1)
    expect(host.toasts.at(-1)?.text).toMatch(/^Saved to Downloads\/iii\/screenshot-example\.com-/)
  })

  it('clearing site data asks first (destructive), then reloads', async () => {
    let ok = false
    const { host } = await opened('t1', { ui: { actionMenu: () => 11, confirm: () => ok } })
    await host.press('tab-menu')
    expect(host.dialogs.at(-1)).toEqual({
      kind: 'confirm',
      title: 'Clear data for example.com?',
      message: 'Cookies, cache and storage of this site.',
      confirm: 'Clear',
      destructive: true,
    })
    expect(callsOf(host, 'browser::clear-data')).toEqual([])
    ok = true
    await host.press('tab-menu')
    expect(callsOf(host, 'browser::clear-data')).toEqual([
      { session_id: 't1', cookies: true, cache: true, storage: true },
    ])
    expect(host.toasts.at(-1)?.text).toBe('Cleared cookies, cache, storage')
    expect(callsOf(host, 'browser::history')).toEqual([{ session_id: 't1', action: 'reload' }])
  })

  it('copies, shares and opens the URL outside; a blank page has nothing to open', async () => {
    const { host } = await opened('t1', { ui: answers(8, 9, 10) })
    await host.press('tab-menu')
    await host.press('tab-menu')
    await host.press('tab-menu')
    expect(host.clipboard).toEqual(['https://example.com/'])
    expect(host.shared).toEqual([{ text: 'https://example.com/', title: 'Example Domain' }])
    expect(host.opened).toEqual(['https://example.com/'])

    const blank = await start({ tabs: [{ session_id: 'b', url: 'about:blank' }], ui: answers(10) })
    await blank.host.itemPress('tabs', 'b')
    await blank.host.press('tab-menu')
    expect(blank.host.opened).toEqual([])
    expect(blank.host.toasts.at(-1)?.text).toBe("This page can't open outside the engine's browser")
  })

  it('Console, Network and History open as cards of the tab; a visit picked navigates', async () => {
    const { host, fake } = await opened('t1', { ui: answers(5, 6, 7) })
    await host.press('tab-menu')
    expect(host.deck.at(-1)).toMatchObject({ name: 'console', params: { session: 't1' } })
    await host.back()
    await host.press('tab-menu')
    expect(host.deck.at(-1)).toMatchObject({ name: 'network', params: { session: 't1' } })
    await host.back()
    fake.visits = [{ url: 'https://example.com/old', title: 'Old', timestamp: 1 }]
    await host.press('tab-menu')
    expect(host.deck.at(-1)).toMatchObject({ name: 'history', params: { session: 't1' } })
    await host.itemPress('history-list', '1|https://example.com/old')
    expect(host.deck.map((c) => c.name)).toEqual(['root'])
    expect(callsOf(host, 'browser::navigate')).toEqual([{ session_id: 't1', url: 'https://example.com/old' }])
  })

  it('Close tab closes it and opens the neighbour', async () => {
    const { host } = await opened('t1', { ui: answers(12) })
    await host.press('tab-menu')
    expect(callsOf(host, 'browser::sessions::stop')).toEqual([{ session_id: 't1' }])
    expect(host.find('screen').props?.session).toBe('t2')
  })
})

describe('handoff and closed tabs', () => {
  it('a handoff for the open tab shows the banner until I’m done or resolved', async () => {
    const { host } = await opened('t1')
    expect(subsOf(host, 'browser::handoff-requested')).toEqual([{ session_id: 't1' }])
    expect(subsOf(host, 'browser::handoff-resolved')).toEqual([{ session_id: 't1' }])
    await host.emitTrigger('browser::handoff-requested', {
      session_id: 't1',
      handoff_id: 'h1',
      instructions: 'Sign in, then tap I’m done.',
      timestamp: 1,
    })
    expect(host.find('handoff').props).toMatchObject({
      variant: 'warn',
      headline: 'The agent needs you',
      detail: 'Sign in, then tap I’m done.',
    })
    await host.press('handoff-done')
    expect(callsOf(host, 'browser::handoff::confirm')).toEqual([{ session_id: 't1', handoff_id: 'h1' }])
    expect(exists(host, 'handoff')).toBe(false)

    await host.emitTrigger('browser::handoff-requested', {
      session_id: 't1',
      handoff_id: 'h2',
      instructions: 'Solve it',
    })
    expect(exists(host, 'handoff')).toBe(true)
    await host.emitTrigger('browser::handoff-resolved', { session_id: 't1', handoff_id: 'h2', via: 'page' })
    expect(exists(host, 'handoff')).toBe(false)
  })

  it('handoffs are heard only while visible', async () => {
    const { host } = await opened('t1')
    await host.env({ visible: false })
    expect(subsOf(host, 'browser::handoff-requested')).toEqual([])
    await host.env({ visible: true })
    expect(subsOf(host, 'browser::handoff-requested')).toEqual([{ session_id: 't1' }])
  })

  it('a tab that stopped while open says so, with Back to tabs', async () => {
    const { host } = await opened('t1')
    await host.emitTrigger('browser::session-stopped', { session_id: 't1', reason: 'crashed', timestamp: 1 })
    expect(host.find('tab-closed').props?.title).toBe('This tab was closed (it crashed)')
    expect(exists(host, 'screen')).toBe(false)
    expect(shown(host, 'bars')).toBe(false)
    expect(host.find('tab-menu').props?.visible).toBe(false)
    await host.press('back-to-tabs')
    expect(host.find('tabs-split').props?.showDetail ?? false).toBe(false)
  })
})
