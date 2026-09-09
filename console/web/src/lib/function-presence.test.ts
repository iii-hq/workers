import { beforeEach, describe, expect, it, vi } from 'vitest'

const fetchFunctionsCatalog = vi.fn()
vi.mock('./functions-catalog', () => ({
  fetchFunctionsCatalog: (...args: unknown[]) => fetchFunctionsCatalog(...args),
}))

import { functionRegistered } from './function-presence'

describe('functionRegistered', () => {
  beforeEach(() => {
    fetchFunctionsCatalog.mockReset()
  })

  it('answers from the cached catalog', async () => {
    fetchFunctionsCatalog.mockResolvedValue([
      { id: 'shell::workspace::validate', description: '' },
      { id: 'state::get', description: '' },
    ])
    await expect(
      functionRegistered('shell::workspace::validate'),
    ).resolves.toBe(true)
    await expect(functionRegistered('editor::workspace::open')).resolves.toBe(
      false,
    )
  })

  /** Prevents: a console that stops calling anything because the catalog
   * read itself failed — the caller keeps its old behaviour and surfaces its
   * own error instead. */
  it('fails open when the catalog cannot be read', async () => {
    fetchFunctionsCatalog.mockRejectedValue(new Error('engine unreachable'))
    await expect(functionRegistered('worker::list')).resolves.toBe(true)
  })
})
