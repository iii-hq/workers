import { describe, expect, it } from 'vitest'
import { type Bounds, moveCursor, reanchor, rowAsTsv, rowKey } from './grid-cursor'

const bounds = (over: Partial<Bounds> = {}): Bounds => ({
  rowCount: 10,
  colCount: 4,
  pageRows: 5,
  hasPrevPage: false,
  hasNextPage: false,
  ...over,
})

describe('moveCursor', () => {
  it('clamps at the edges rather than wrapping', () => {
    expect(moveCursor({ row: 0, col: 0 }, { type: 'up' }, bounds()).cursor).toEqual({ row: 0, col: 0 })
    expect(moveCursor({ row: 0, col: 0 }, { type: 'left' }, bounds()).cursor).toEqual({ row: 0, col: 0 })
    expect(moveCursor({ row: 9, col: 3 }, { type: 'down' }, bounds()).cursor).toEqual({ row: 9, col: 3 })
    expect(moveCursor({ row: 9, col: 3 }, { type: 'right' }, bounds()).cursor).toEqual({ row: 9, col: 3 })
  })

  it('crosses to the next page from the last row', () => {
    const m = moveCursor({ row: 9, col: 2 }, { type: 'down' }, bounds({ hasNextPage: true }))
    expect(m.pageDelta).toBe(1)
    expect(m.landing).toBe('first')
    // The column is preserved across the page turn — moving down a column
    // should not also move sideways.
    expect(m.cursor.col).toBe(2)
  })

  it('crosses to the previous page from the first row', () => {
    const m = moveCursor({ row: 0, col: 1 }, { type: 'up' }, bounds({ hasPrevPage: true }))
    expect(m.pageDelta).toBe(-1)
    expect(m.landing).toBe('last')
  })

  it('does not cross a page that does not exist', () => {
    expect(moveCursor({ row: 9, col: 0 }, { type: 'down' }, bounds()).pageDelta).toBeUndefined()
    expect(moveCursor({ row: 0, col: 0 }, { type: 'up' }, bounds()).pageDelta).toBeUndefined()
  })

  it('pages within the grid before crossing a boundary', () => {
    // From the middle, PageDown jumps inside the page even when a next page
    // exists — the key means "move far", not "turn the page".
    const m = moveCursor({ row: 2, col: 0 }, { type: 'pageDown' }, bounds({ hasNextPage: true }))
    expect(m.pageDelta).toBeUndefined()
    expect(m.cursor.row).toBe(7)
  })

  it('turns the page when PageDown is pressed at the bottom', () => {
    const m = moveCursor({ row: 9, col: 0 }, { type: 'pageDown' }, bounds({ hasNextPage: true }))
    expect(m.pageDelta).toBe(1)
  })

  it('moves to row and grid extremes', () => {
    expect(moveCursor({ row: 3, col: 2 }, { type: 'rowStart' }, bounds()).cursor).toEqual({ row: 3, col: 0 })
    expect(moveCursor({ row: 3, col: 2 }, { type: 'rowEnd' }, bounds()).cursor).toEqual({ row: 3, col: 3 })
    expect(moveCursor({ row: 3, col: 2 }, { type: 'gridStart' }, bounds()).cursor).toEqual({ row: 0, col: 0 })
    expect(moveCursor({ row: 3, col: 2 }, { type: 'gridEnd' }, bounds()).cursor).toEqual({ row: 9, col: 3 })
  })

  it('clamps an out-of-range cursor before moving it', () => {
    // The page shrank underneath the cursor.
    const m = moveCursor({ row: 99, col: 99 }, { type: 'up' }, bounds())
    expect(m.cursor).toEqual({ row: 8, col: 3 })
  })

  it('does nothing on an empty grid', () => {
    const m = moveCursor({ row: 0, col: 0 }, { type: 'down' }, bounds({ rowCount: 0 }))
    expect(m).toEqual({ cursor: { row: 0, col: 0 } })
  })
})

describe('reanchor', () => {
  const columns = ['id', 'email', 'plan']

  it('follows a row to its new index after a sort', () => {
    const keys = ['id=3', 'id=1', 'id=2']
    expect(reanchor({ row: 0, col: 1 }, 'id=2', keys, columns, 'email')).toEqual({ row: 2, col: 1 })
  })

  it('follows a column to its new index', () => {
    const keys = ['id=1']
    expect(reanchor({ row: 0, col: 0 }, 'id=1', keys, ['plan', 'id', 'email'], 'email')).toEqual({
      row: 0,
      col: 2,
    })
  })

  it('clamps when the row is gone', () => {
    // Filtered out. Keep the column, put the cursor somewhere valid.
    expect(reanchor({ row: 5, col: 2 }, 'id=99', ['id=1', 'id=2'], columns, 'plan')).toEqual({
      row: 1,
      col: 2,
    })
  })
})

describe('rowKey', () => {
  it('uses the primary key when there is one', () => {
    expect(rowKey({ id: 7, email: 'a@b.c' }, ['id'])).toBe('id=7')
  })

  it('falls back to the whole row, which survives a re-sort', () => {
    expect(rowKey({ id: 7, email: 'a@b.c' }, [])).toBe('id=7\u0001email=a@b.c')
  })

  it('distinguishes null from the empty string', () => {
    expect(rowKey({ a: null }, [])).not.toBe(rowKey({ a: '' }, []))
  })

  it('does not let adjacent fields run together', () => {
    // With an empty separator these two rows produce the same key, and the
    // cursor would follow the wrong one after a sort.
    expect(rowKey({ a: '1', b: '2' }, ['a', 'b'])).not.toBe(rowKey({ a: '12', b: '' }, ['a', 'b']))
  })
})

describe('rowAsTsv', () => {
  it('joins with tabs so it pastes as spreadsheet cells', () => {
    expect(rowAsTsv({ a: 1, b: 'x' }, ['a', 'b'])).toBe('1\tx')
  })

  it('renders null as empty rather than the text NULL', () => {
    expect(rowAsTsv({ a: null, b: 2 }, ['a', 'b'])).toBe('\t2')
  })

  it('serialises objects rather than pasting [object Object]', () => {
    expect(rowAsTsv({ a: { x: 1 } }, ['a'])).toBe('{"x":1}')
  })
})
