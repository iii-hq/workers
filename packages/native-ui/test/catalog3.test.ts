/** Catalogue 3 in the SDK: the display and data types, Checkbox, Card.tone, List.anchor and the toast's action. */
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { ICONS, LIMITS } from '../src/generated/catalog'
import { createTestHost, VmFailure } from '../src/testing/index.mjs'
import { fixture, mounted, opsSince, pkg } from './helpers'

const catalog = JSON.parse(readFileSync(join(pkg, 'catalog.json'), 'utf8'))

describe('the catalogue', () => {
  it('is version 3, with its icons sorted and unique', () => {
    expect(catalog.catalogVersion).toBe(3)
    expect([...ICONS]).toEqual(catalog.icons.names)
    expect([...ICONS]).toEqual([...new Set(ICONS)].sort())
    for (const name of ['bot', 'gauge', 'trending-up', 'list-checks', 'circle-question-mark']) {
      expect(ICONS).toContain(name)
    }
  })

  it('gives every new limit a value', () => {
    expect(LIMITS).toMatchObject({
      maxCodeChars: 200000,
      maxChartPoints: 512,
      maxChartSeries: 8,
      maxStatPoints: 120,
      maxMeterSegments: 16,
      maxStepsItems: 64,
      maxWaveformPeaks: 1024,
    })
  })

  it('keeps old tags where they were', () => {
    const tags = Object.fromEntries(catalog.types.map((t: { name: string; tag: number }) => [t.name, t.tag]))
    expect(tags).toMatchObject({ Switch: 23, Chip: 25, Card: 32, List: 30, Lanes: 33, Page: 40, RemoteScreen: 64 })
    expect(tags).toMatchObject({
      Checkbox: 26,
      Avatar: 70,
      Timestamp: 71,
      CodeView: 72,
      Stat: 73,
      Chart: 74,
      Gauge: 75,
      Meter: 76,
      Steps: 77,
      Waveform: 78,
    })
  })
})

describe('the new types', () => {
  it('send their props, enums by index and json as given', async () => {
    const h = await mounted('dashboard')
    expect(h.find('ada').props).toMatchObject({ name: 'Ada Lovelace', status: 'warn', size: 28 })
    expect(h.find('bot').props).toMatchObject({ icon: 'bot', hue: 'purple', shape: 'rounded' })
    expect(h.find('when').props).toMatchObject({ at: 1764250000000, style: 'elapsed', role: 'small', tone: 'muted' })
    expect(h.find('todo').props).toMatchObject({ text: 'Ship catalogue 3', strike: true, events: ['change'] })
    expect(h.find('card').props).toMatchObject({ tone: 'danger' })
    expect(h.find('stat').props).toMatchObject({ label: 'Runs', value: '1.2k', series: [1, 2, 3], events: ['select'] })
    expect(h.find('chart').props).toMatchObject({
      kind: 'bar',
      series: [3, 5, null, 2],
      hues: ['amber'],
      thresholds: [{ value: 4, tone: 'warning' }],
    })
    expect(h.find('gauge').props).toMatchObject({ kind: 'half', value: 0.5, thresholds: [0.6, 0.9] })
    expect(h.find('meter').props).toMatchObject({ values: [1, 2], hues: ['teal', 'rose'], thickness: 6 })
    expect(h.find('steps').props).toMatchObject({ axis: 'horizontal', compact: true })
    expect(h.find('code').props).toMatchObject({
      text: '+ added\n- removed',
      language: 'diff',
      startLine: 5,
      highlight: [[5, 6]],
    })
    expect(h.find('wave').props).toMatchObject({ peaks: [0.2, 0.8] })
    expect(h.find('log').props).toMatchObject({ anchor: 'bottom', events: ['startReached'] })
    expect(h.violations).toEqual([])
    expect(h.logs.filter((l) => l.level === 'warn')).toEqual([])
  })

  it('hand each select its own value: an index, a line, a fraction', async () => {
    const h = await mounted('dashboard')
    await h.select('stat', 2)
    expect(h.prop('last', 'text')).toBe('stat 2')
    await h.select('chart', 3)
    expect(h.prop('last', 'text')).toBe('chart 3')
    await h.select('steps', 1)
    expect(h.prop('last', 'text')).toBe('step 1')
    await h.select('code', 6)
    expect(h.prop('last', 'text')).toBe('line 6')
    await h.select('wave', 0.25)
    expect(h.prop('last', 'text')).toBe('seek 0.25')
  })

  it('refuse a select the host would never send', async () => {
    const h = await mounted('dashboard')
    await expect(h.select('chart', 4)).rejects.toThrow(/4 is not a point of Chart/)
    await expect(h.select('steps', 2)).rejects.toThrow(/2 is not a step of Steps/)
    await expect(h.select('code', 4)).rejects.toThrow(/4 is not a line of CodeView/)
    await expect(h.select('code', 7)).rejects.toThrow(/7 is not a line of CodeView/)
    await expect(h.select('wave', 1.5)).rejects.toThrow(/is not a fraction/)
    await expect(h.select('gauge', 0)).rejects.toThrow(/is a Gauge/)
  })
})

describe('Checkbox', () => {
  it('flips at once and takes the app’s value', async () => {
    const h = await mounted('dashboard')
    const { id } = h.find('todo')
    const from = h.messages.length
    await h.toggle('todo')
    expect(h.toVm.at(-1)).toBe(`[[2,${id},3,[true]]]`)
    expect(opsSince(h, from)).toEqual([[2, id, 16, true]])
    expect(h.prop('todo', 'checked')).toBe(true)
  })

  it('turns checked from indeterminate', async () => {
    const h = await mounted('dashboard')
    await h.toggle('all')
    expect(h.prop('last', 'text')).toBe('all true')
    expect(h.find('all').props?.indeterminate).toBe(false)
  })

  it('goes back to its own value when the app does not take the change', async () => {
    const h = await createTestHost({ bundle: await fixture('dashboard') })
    const inst = await h.mount('stubborn')
    const { id } = h.find('stubborn')
    const from = h.messages.length
    await h.toggle('stubborn')
    expect(opsSince(h, from)).toEqual([[2, id, 16, false]])
    expect(h.tree(inst)?.props?.checked).toBe(false)
  })
})

describe('List.anchor', () => {
  it('asks for older items with startReached; they go in at index 0', async () => {
    const h = await mounted('dashboard')
    await h.emit('log', 'startReached')
    expect(h.keys('log')).toEqual([9, 10])
  })
})

describe('a toast with an action', () => {
  it('waits for the host: true when the person tapped the action', async () => {
    const h = await mounted('dashboard', { ui: { toast: () => true } })
    await h.press('toast')
    expect(h.toasts).toEqual([{ text: 'Deleted', kind: 'info', action: 'Undo' }])
    expect(h.prop('undo', 'text')).toBe('true')
  })

  it('resolves false when the toast went away', async () => {
    const h = await mounted('dashboard')
    await h.press('toast')
    expect(h.prop('undo', 'text')).toBe('false')
  })
})

describe('an app that declares catalogue 2', () => {
  it('warns once per catalogue 3 piece it uses', async () => {
    const h = await mounted('undeclared3')
    await h.press('toast')
    const warnings = h.logs.filter((l) => l.level === 'warn').map((l) => l.message)
    expect(warnings).toEqual([
      'native-ui: Card.tone is catalogue 3, and this app declares 2: defineApp({ catalog: 3, … }), and NativeUi::catalog(3) in the worker',
      'native-ui: Chart is catalogue 3, and this app declares 2: defineApp({ catalog: 3, … }), and NativeUi::catalog(3) in the worker',
      'native-ui: iii.ui.toast with an action is catalogue 3, and this app declares 2: defineApp({ catalog: 3, … }), and NativeUi::catalog(3) in the worker',
    ])
  })

  it('a catalogue 2 host refuses a catalogue 3 bundle', async () => {
    const err = await createTestHost({ bundle: await fixture('dashboard'), catalog: 2 }).catch((e) => e)
    expect(err).toBeInstanceOf(VmFailure)
    expect(err.message).toMatch(/needs catalog 3; this host renders 2/)
  })
})
