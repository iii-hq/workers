import { describe, expect, it } from 'vitest'
import { canonicalTable } from './useRowChanges'

// Observer args name the table the way the worker's `canonical_table` spells
// it (src/row_changes/sql.rs); any other spelling is NON_CANONICAL_TABLE.
describe('canonicalTable', () => {
  it('keeps a canonical name as it is', () => {
    expect(canonicalTable('orders')).toBe('orders')
  })

  it('drops the schema qualifier', () => {
    expect(canonicalTable('public.orders')).toBe('orders')
    expect(canonicalTable('main.Orders')).toBe('orders')
  })

  it('unquotes and lowercases the table', () => {
    expect(canonicalTable('"Orders"')).toBe('orders')
    expect(canonicalTable('`Orders`')).toBe('orders')
    expect(canonicalTable('[Orders]')).toBe('orders')
    expect(canonicalTable('public."Order_Lines"')).toBe('order_lines')
  })

  it('lowercases ASCII only, as the worker does', () => {
    expect(canonicalTable('ÉTAT')).toBe('État')
  })

  it('leaves a lone quote character alone', () => {
    expect(canonicalTable('"')).toBe('"')
  })
})
