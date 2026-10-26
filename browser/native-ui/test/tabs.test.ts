/** The tab list (design §8.2, §8.4): rows, groups, sleeping tabs, new and close, live re-reads, visibility. */
import { describe, expect, it } from 'vitest'
import { callsOf, exists, FakeBrowser, rows, start, subsOf, TABS } from './fake'

const LIFECYCLE = [
  'browser::session-started',
  'browser::session-stopped',
  'browser::session-updated',
  'browser::navigated',
]

describe('the tab list', () => {
  it('groups regular and incognito tabs under headers, with sleeping tabs marked', async () => {
    const { host } = await start({ tabs: TABS })
    expect(host.tree()?.props).toMatchObject({ title: 'Browser' })
    expect(rows(host)).toEqual([
      { key: 'header:tabs', kind: 1, title: 'Tabs' },
      { key: 't1', kind: 0, title: 'Example Domain', icon: 'globe', selected: false, subtitle: 'example.com/' },
      { key: 't2', kind: 2, title: 'iii — More application.', icon: 'moon', selected: false, subtitle: 'iii.dev/' },
      { key: 'header:incognito', kind: 1, title: 'Incognito' },
      {
        key: 't3',
        kind: 0,
        title: 'Sign in to GitHub',
        icon: 'eye-off',
        selected: false,
        subtitle: 'github.com/login',
      },
    ])
    // Template 2 (asleep): faint title, the Asleep badge and the close button.
    const list = host.findAll({ type: 'List' }, { templates: true })[0]
    const asleep = host.findAll({ type: 'ListRow', tone: 'faint' }, { templates: true })
    expect(asleep).toHaveLength(1)
    expect(host.findAll({ type: 'Badge', text: 'Asleep' }, { templates: true })).toHaveLength(1)
    expect(list.props).toMatchObject({ kindField: 'kind', testTag: 'tabs' })
  })

  it('names a tab by its title, else its host, else New tab, and hides the blank address', async () => {
    const { host } = await start({
      tabs: [
        { session_id: 'a', url: 'https://news.example.org/today', title: '' },
        { session_id: 'b', url: 'about:blank', title: 'about:blank' },
        { session_id: 'c', url: 'http://localhost:3000/', title: 'Dev' },
      ],
    })
    expect(rows(host).slice(1)).toEqual([
      {
        key: 'a',
        kind: 0,
        title: 'news.example.org',
        icon: 'globe',
        selected: false,
        subtitle: 'news.example.org/today',
      },
      { key: 'b', kind: 0, title: 'New tab', icon: 'globe', selected: false },
      { key: 'c', kind: 0, title: 'Dev', icon: 'globe', selected: false, subtitle: 'localhost:3000/' },
    ])
  })

  it('opens no tab by itself: the empty state offers New tab and New incognito tab', async () => {
    const { host, fake } = await start({ tabs: [] })
    expect(rows(host)).toEqual([])
    expect(host.find('no-tabs').props).toMatchObject({
      title: 'No tabs',
      description: "Open a page in the engine's browser.",
    })
    expect(callsOf(host, 'browser::sessions::start')).toEqual([])

    await host.press('empty-new-incognito')
    expect(callsOf(host, 'browser::sessions::start')).toEqual([{ incognito: true, preview: false }])
    expect(fake.tabs).toHaveLength(1)
    expect(fake.tabs[0].incognito).toBe(true)
    await host.tick(0)
    expect(rows(host).map((r) => r.key)).toEqual(['header:incognito', 'n1'])
  })

  it('a new tab opens selected, with the address field ready', async () => {
    const { host } = await start({ tabs: TABS.slice(0, 1) })
    await host.press('new-tab')
    expect(callsOf(host, 'browser::sessions::start')).toEqual([{ preview: false }])
    expect(host.find('tabs-split').props?.showDetail).toBe(true)
    expect(host.find('screen').props?.session).toBe('n1')
    const field = host.find('address-field')
    expect(field.props).toMatchObject({
      value: '',
      keyboard: 'url',
      imeAction: 'go',
      autofocus: true,
      selectAllOnFocus: true,
    })
    // The row is there before the worker's list catches up, and stays after it.
    expect(rows(host).map((r) => r.key)).toEqual(['header:tabs', 't1', 'n1'])
    await host.tick(0)
    expect(rows(host).find((r) => r.key === 'n1')).toMatchObject({ title: 'New tab', selected: true })
  })

  it('the + long press and the ⋮ offer a new incognito tab; the tab limit fails in words', async () => {
    const fake = new FakeBrowser(TABS)
    fake.overrides['browser::sessions::start'] = () => {
      throw Object.assign(new Error('every live tab is watched; close one first (max_sessions 4)'), {
        code: 'invocation_failed',
      })
    }
    const { host } = await start({ fake, ui: { actionMenu: () => 1 } })
    await host.longPress('new-tab')
    expect(host.menus.at(-1)).toMatchObject({
      actions: [
        { label: 'New tab', icon: 'plus' },
        { label: 'New incognito tab', icon: 'eye-off' },
      ],
    })
    expect(callsOf(host, 'browser::sessions::start')).toEqual([{ incognito: true, preview: false }])
    expect(host.toasts.at(-1)).toEqual({
      text: 'every live tab is watched; close one first (max_sessions 4)',
      kind: 'error',
    })
    await host.press('browser-menu')
    expect(host.menus).toHaveLength(2)
  })

  it('tapping a row opens it; Back (dismiss) closes the detail', async () => {
    const { host } = await start({ tabs: TABS })
    expect(host.find('tabs-split').props?.showDetail ?? false).toBe(false)
    expect(exists(host, 'screen')).toBe(false)
    await host.itemPress('tabs', 't1')
    expect(host.find('tabs-split').props?.showDetail).toBe(true)
    expect(rows(host)[1]).toMatchObject({ key: 't1', selected: true })
    await host.emit('tabs-split', 'dismiss')
    expect(host.find('tabs-split').props?.showDetail ?? false).toBe(false)
    expect(exists(host, 'screen')).toBe(false)
  })

  it('closing the open tab moves to the right neighbour, then the left, then the list', async () => {
    const { host, fake } = await start({ tabs: TABS })
    await host.itemPress('tabs', 't1')
    await host.itemPress('tabs', 't1', 'tab-close')
    expect(callsOf(host, 'browser::sessions::stop')).toEqual([{ session_id: 't1' }])
    expect(host.find('screen').props).toMatchObject({ session: 't2', placeholder: 'Waking the tab…' })
    await host.tick(0)
    expect(fake.tabs.map((t) => t.session_id)).toEqual(['t2', 't3'])

    // t3 (incognito) is the last row: closing it goes left, to t2.
    await host.itemPress('tabs', 't3')
    await host.itemPress('tabs', 't3', 'tab-close')
    expect(host.find('screen').props?.session).toBe('t2')

    // The last tab: back to the list (no detail), and the empty state.
    await host.itemPress('tabs', 't2', 'tab-close')
    await host.tick(0)
    expect(host.find('tabs-split').props?.showDetail ?? false).toBe(false)
    expect(rows(host)).toEqual([])
  })

  it('closing a tab that is not open leaves the open one', async () => {
    const { host } = await start({ tabs: TABS })
    await host.itemPress('tabs', 't3')
    await host.itemPress('tabs', 't1', 'tab-close')
    await host.tick(0)
    expect(host.find('screen').props?.session).toBe('t3')
    expect(rows(host).map((r) => r.key)).toEqual(['header:tabs', 't2', 'header:incognito', 't3'])
  })

  it('a row long press offers Open, Copy URL, Share URL and Close tab', async () => {
    let answer = 1
    const { host, fake } = await start({ tabs: TABS, ui: { actionMenu: () => answer } })
    await host.itemLongPress('tabs', 't1')
    expect(host.menus.at(-1)).toMatchObject({
      title: 'Example Domain',
      subtitle: 'example.com/',
      actions: [{ label: 'Open' }, { label: 'Copy URL' }, { label: 'Share URL' }, { label: 'Close tab' }],
    })
    expect(host.clipboard).toEqual(['https://example.com/'])
    answer = 2
    await host.itemLongPress('tabs', 't1')
    expect(host.shared.at(-1)).toEqual({ text: 'https://example.com/', title: 'Example Domain' })
    answer = 0
    await host.itemLongPress('tabs', 't3')
    expect(host.find('screen').props?.session).toBe('t3')
    answer = 3
    await host.itemLongPress('tabs', 't1')
    expect(fake.tabs.map((t) => t.session_id)).toEqual(['t2', 't3'])
  })
})

describe('live', () => {
  it('binds the four lifecycle triggers unfiltered, then reads; each event re-reads 150 ms later', async () => {
    const { host, fake } = await start({ tabs: TABS })
    for (const t of LIFECYCLE) expect(subsOf(host, t)).toEqual([{}])
    expect(callsOf(host, 'browser::sessions::list')).toHaveLength(1)
    // The read comes after the bindings (bind first, read after ready).
    const firstList = host.requests.findIndex((r) => r.args.fn === 'browser::sessions::list')
    const lastOn = host.requests.map((r) => r.method).lastIndexOf('on')
    expect(firstList).toBeGreaterThan(lastOn)

    fake.tabs[1].active = true
    fake.tabs[1].title = 'iii'
    await host.emitTrigger('browser::session-updated', {
      session_id: 't2',
      active: true,
      url: 'https://iii.dev/',
      title: 'iii',
    })
    await host.emitTrigger('browser::navigated', { session_id: 't1', url: 'https://example.com/a' })
    await host.tick(100)
    expect(callsOf(host, 'browser::sessions::list')).toHaveLength(1)
    await host.tick(50)
    expect(callsOf(host, 'browser::sessions::list')).toHaveLength(2)
    expect(rows(host)[2]).toMatchObject({ key: 't2', kind: 0, title: 'iii', icon: 'globe' })
  })

  it('stops listening while hidden and reads again when visible', async () => {
    const { host, fake } = await start({ tabs: TABS })
    await host.env({ visible: false })
    for (const t of LIFECYCLE) expect(subsOf(host, t)).toEqual([])
    fake.tabs.push({ session_id: 't4', url: 'https://later.example/', title: 'Later' })
    await host.env({ visible: true })
    for (const t of LIFECYCLE) expect(subsOf(host, t)).toEqual([{}])
    expect(callsOf(host, 'browser::sessions::list')).toHaveLength(2)
    expect(rows(host).map((r) => r.key)).toContain('t4')
  })

  it('a failed read shows a panel with Retry; pull to refresh reads too', async () => {
    const fake = new FakeBrowser(TABS)
    fake.overrides['browser::sessions::list'] = () => {
      throw new Error('the browser worker is restarting')
    }
    const { host } = await start({ fake })
    expect(host.find('list-error').props).toMatchObject({
      variant: 'alert',
      headline: "Couldn't read the tabs",
      detail: 'the browser worker is restarting',
    })
    delete fake.overrides['browser::sessions::list']
    await host.press('list-retry')
    expect(exists(host, 'list-error')).toBe(false)
    expect(rows(host)).toHaveLength(5)
    await host.emit('tabs', 'refresh')
    expect(callsOf(host, 'browser::sessions::list')).toHaveLength(3)
    expect(host.find('tabs').props?.refreshing ?? false).toBe(false)
  })

  it('offline disables what calls the worker; back online reads again', async () => {
    const { host } = await start({ tabs: TABS })
    await host.env({ online: false })
    expect(host.find('new-tab').props?.enabled).toBe(false)
    await host.env({ online: true })
    await host.tick(0)
    expect(host.find('new-tab').props?.enabled ?? true).toBe(true)
    expect(callsOf(host, 'browser::sessions::list')).toHaveLength(2)
  })
})
