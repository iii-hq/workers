import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { pickRows, type RowNav, type RowNavOptions, useRowNav } from '../use-row-nav'

function navFor(options: RowNavOptions<string>): RowNav {
  let nav: RowNav | undefined
  function Probe() {
    nav = useRowNav(options)
    return null
  }
  renderToStaticMarkup(<Probe />)
  if (nav === undefined) throw new Error('not rendered')
  return nav
}

describe('row events held by the list', () => {
  it('act on the row an event came from, by its id, and on nothing off the rows', () => {
    const onSelect = vi.fn()
    const onClickRow = vi.fn()
    const onAct = vi.fn()
    const onMenu = vi.fn()
    const nav = navFor({
      items: ['a', 'b', 'c'],
      idOf: (item) => item,
      labelOf: (item) => item,
      domId: 'r1',
      selected: null,
      onSelect,
      onClickRow,
      onAct,
      onMenu,
    })
    // The list, row 2 in it (row 12's id shares the prefix), and a chip in the row.
    const list = { id: '', parentElement: null }
    const row = { id: 'r1-2', parentElement: list }
    const chip = { id: 'r12-0', parentElement: row }
    const event = (target: object) => ({ target, currentTarget: list, clientX: 3, clientY: 4, preventDefault: vi.fn() })

    nav.rowEvents.onClick(event({ id: '', parentElement: chip }) as never)
    expect(onSelect).toHaveBeenCalledWith('c')
    expect(onClickRow).toHaveBeenCalledWith('c', expect.objectContaining({ clientX: 3 }))
    nav.rowEvents.onDoubleClick(event(row) as never)
    expect(onAct).toHaveBeenCalledWith('c')
    nav.rowEvents.onContextMenu(event(chip) as never)
    expect(onMenu).toHaveBeenCalledWith('c', { x: 3, y: 4 })

    onSelect.mockClear()
    const off = event(list)
    nav.rowEvents.onContextMenu(off as never)
    nav.rowEvents.onClick(event(list) as never)
    expect(off.preventDefault).not.toHaveBeenCalled()
    expect(onSelect).not.toHaveBeenCalled()
    expect(onMenu).toHaveBeenCalledTimes(1)
  })
})

describe('pickRows', () => {
  const order = ['a', 'b', 'c', 'd']
  const none = new Set<string>()

  it('picks nothing new for a row that is gone', () => {
    expect([...pickRows(new Set(['a']), order, 0, -1, { toggle: true, range: false })]).toEqual(['a'])
  })

  it('picks the row alone on a plain click', () => {
    expect([...pickRows(new Set(['a', 'c']), order, 0, 3, { toggle: false, range: false })]).toEqual(['d'])
  })

  it('adds or drops a row with the toggle key', () => {
    const one = pickRows(none, order, -1, 1, { toggle: true, range: false })
    expect([...one]).toEqual(['b'])
    expect([...pickRows(one, order, 1, 3, { toggle: true, range: false })]).toEqual(['b', 'd'])
    expect([...pickRows(one, order, 1, 1, { toggle: true, range: false })]).toEqual([])
  })

  it('takes the rows from the anchor with Shift, either way, adding to the picked ones with both keys', () => {
    expect([...pickRows(new Set(['d']), order, 2, 0, { toggle: false, range: true })]).toEqual(['a', 'b', 'c'])
    expect([...pickRows(new Set(['d']), order, 0, 1, { toggle: true, range: true })]).toEqual(['d', 'a', 'b'])
    expect([...pickRows(none, order, -1, 2, { toggle: false, range: true })]).toEqual(['c'])
  })
})
