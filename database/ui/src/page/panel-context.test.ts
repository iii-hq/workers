import { describe, expect, it } from 'vitest'
import { parseDatabasePanelContext } from './panel-context'

describe('parseDatabasePanelContext', () => {
  it('opens a table, a connection, or both', () => {
    expect(parseDatabasePanelContext({ db: 'shop', table: 'public.orders' })).toEqual({
      db: 'shop',
      table: 'public.orders',
    })
    expect(parseDatabasePanelContext({ table: 'orders' })).toEqual({ db: undefined, table: 'orders' })
    expect(parseDatabasePanelContext({ db: 'shop' })).toEqual({ db: 'shop', table: undefined })
  })

  it('ignores contexts that name nothing', () => {
    expect(parseDatabasePanelContext({})).toBeNull()
    expect(parseDatabasePanelContext({ db: '', table: '' })).toBeNull()
    expect(parseDatabasePanelContext('shop')).toBeNull()
    expect(parseDatabasePanelContext(null)).toBeNull()
  })
})
