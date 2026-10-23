/** The bundle itself: under the SDK's warning size, the eight pages of the design, catalogue 2. */
import { expect, inject, it } from 'vitest'
import { start } from './fake'

it('stays under the SDK’s 128 KiB warning and declares the design’s pages, needing catalogue 2', async () => {
  expect(inject('bundleBytes')).toBeLessThan(128 * 1024)
  const { host } = await start({ mount: false })
  expect(host.ready.pages).toEqual([
    { name: 'root', modal: false, fullScreen: false },
    { name: 'ticket', modal: false, fullScreen: false },
    { name: 'new', modal: true, fullScreen: false },
    { name: 'text', modal: false, fullScreen: false },
    { name: 'pick', modal: false, fullScreen: false },
    { name: 'filters', modal: false, fullScreen: false },
    { name: 'releases', modal: false, fullScreen: false },
    { name: 'projects', modal: false, fullScreen: false },
  ])
  expect(host.ready.catalog).toBe(2)
})

it('an iii app that speaks only catalogue 1 does not run it ("Needs a newer iii app")', async () => {
  const { createTestHost } = await import('@iii-dev/native-ui/testing')
  await expect(createTestHost({ bundle: inject('bundle'), ns: 'kanban', catalog: 1 })).rejects.toThrow(/catalog 2/)
})
