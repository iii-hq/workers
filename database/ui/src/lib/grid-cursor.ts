/**
 * Grid cursor movement, as a pure function.
 *
 * Kept out of React because it is the one piece of this page with enough edge
 * cases to be worth testing directly: clamping, page-edge crossing, and
 * surviving a re-sort. It has no imports and never touches the DOM.
 *
 * The grid is *server*-paged, which is what makes this more than a clamp.
 * Pressing Down on the last visible row of page 2 should land on the first row
 * of page 3, not sit still — a grid that stops dead at the page edge feels
 * broken in a way a desktop client never does. Movement therefore returns an
 * optional `pageDelta` alongside the new cursor, and the caller decides
 * whether it can honour it (it cannot, at the first or last page).
 */

export interface Cursor {
  row: number
  col: number
}

export interface Bounds {
  rowCount: number
  colCount: number
  /** Rows to jump for PageUp/PageDown. */
  pageRows: number
  /** False on the first page — Up at row 0 then has nowhere to go. */
  hasPrevPage: boolean
  hasNextPage: boolean
}

export type CursorAction =
  | { type: 'up' }
  | { type: 'down' }
  | { type: 'left' }
  | { type: 'right' }
  | { type: 'rowStart' }
  | { type: 'rowEnd' }
  | { type: 'gridStart' }
  | { type: 'gridEnd' }
  | { type: 'pageUp' }
  | { type: 'pageDown' }
  | { type: 'to'; row: number; col: number }

export interface CursorMove {
  cursor: Cursor
  /**
   * -1 / +1 when the move ran off the top or bottom of the page. The caller
   * turns the page and places the cursor at `landing`.
   */
  pageDelta?: -1 | 1
  /** Which edge row to land on after the caller turns the page. */
  landing?: 'first' | 'last'
}

const clamp = (n: number, max: number) => Math.max(0, Math.min(max, n))

export function moveCursor(cursor: Cursor, action: CursorAction, bounds: Bounds): CursorMove {
  const { rowCount, colCount, pageRows, hasPrevPage, hasNextPage } = bounds
  if (rowCount <= 0 || colCount <= 0) return { cursor }

  const lastRow = rowCount - 1
  const lastCol = colCount - 1
  const at = { row: clamp(cursor.row, lastRow), col: clamp(cursor.col, lastCol) }

  switch (action.type) {
    case 'up':
      if (at.row === 0 && hasPrevPage) {
        return { cursor: at, pageDelta: -1, landing: 'last' }
      }
      return { cursor: { ...at, row: clamp(at.row - 1, lastRow) } }

    case 'down':
      if (at.row === lastRow && hasNextPage) {
        return { cursor: at, pageDelta: 1, landing: 'first' }
      }
      return { cursor: { ...at, row: clamp(at.row + 1, lastRow) } }

    case 'left':
      return { cursor: { ...at, col: clamp(at.col - 1, lastCol) } }

    case 'right':
      return { cursor: { ...at, col: clamp(at.col + 1, lastCol) } }

    case 'rowStart':
      return { cursor: { ...at, col: 0 } }

    case 'rowEnd':
      return { cursor: { ...at, col: lastCol } }

    case 'gridStart':
      return { cursor: { row: 0, col: 0 } }

    case 'gridEnd':
      return { cursor: { row: lastRow, col: lastCol } }

    case 'pageUp':
      // Only cross a page boundary from the very top. Otherwise jump within
      // the page, which is what the key means when there is room to move.
      if (at.row === 0 && hasPrevPage) {
        return { cursor: at, pageDelta: -1, landing: 'last' }
      }
      return { cursor: { ...at, row: clamp(at.row - pageRows, lastRow) } }

    case 'pageDown':
      if (at.row === lastRow && hasNextPage) {
        return { cursor: at, pageDelta: 1, landing: 'first' }
      }
      return { cursor: { ...at, row: clamp(at.row + pageRows, lastRow) } }

    case 'to':
      return { cursor: { row: clamp(action.row, lastRow), col: clamp(action.col, lastCol) } }
  }
}

/**
 * Re-find the cursor after the rows underneath it changed.
 *
 * Anchoring on the row *key* and the column *name* rather than their indices
 * is the whole point: after a sort, index 4 is a different row, and a cursor
 * that stayed at index 4 would silently point somewhere else. When the row is
 * gone — filtered out, or on another page now — the column is kept and the row
 * clamps, which is the least surprising thing available.
 */
export function reanchor(
  cursor: Cursor,
  previousKey: string | null,
  keys: string[],
  columns: string[],
  previousColumn: string | null,
): Cursor {
  const row = previousKey === null ? cursor.row : keys.indexOf(previousKey)
  const col = previousColumn === null ? cursor.col : columns.indexOf(previousColumn)
  return {
    row: row >= 0 ? row : clamp(cursor.row, Math.max(0, keys.length - 1)),
    col: col >= 0 ? col : clamp(cursor.col, Math.max(0, columns.length - 1)),
  }
}

/**
 * A stable identity for a row.
 *
 * Primary key when there is one. Otherwise the row's own values, which is not
 * guaranteed unique but is stable across a re-sort — and a duplicate is a far
 * smaller error than an index that silently means a different row.
 */
export function rowKey(row: Record<string, unknown>, primaryKeys: string[]): string {
  const cols = primaryKeys.length > 0 ? primaryKeys : Object.keys(row)
  return cols.map((c) => `${c}=${String(row[c])}`).join(KEY_SEP)
}

/**
 * Field separator for composite keys. A control character rather than an empty
 * string or a comma: joining with nothing makes `{a:'1',b:'2'}` collide with
 * `{a:'12',b:''}`, and any printable separator can occur inside a value.
 */
const KEY_SEP = '\u0001'

/** A row as tab-separated text, so it pastes into a spreadsheet as cells. */
export function rowAsTsv(row: Record<string, unknown>, columns: string[]): string {
  return columns.map((c) => cellText(row[c])).join('\t')
}

export function cellText(value: unknown): string {
  if (value === null || value === undefined) return ''
  if (typeof value === 'object') {
    try {
      return JSON.stringify(value)
    } catch {
      return String(value)
    }
  }
  return String(value)
}
