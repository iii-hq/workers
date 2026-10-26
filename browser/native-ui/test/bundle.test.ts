/** The bundle itself: small, and declaring the four pages of design §8.1. */
import { expect, inject, it } from 'vitest'
import { start } from './fake'

it('stays under the SDK’s 128 KiB warning and declares root, console, network and history', async () => {
  expect(inject('bundleBytes')).toBeLessThan(128 * 1024)
  const { host } = await start({ mount: false })
  expect(host.ready.pages).toEqual([
    { name: 'root', modal: false, fullScreen: false },
    { name: 'console', modal: false, fullScreen: false },
    { name: 'network', modal: false, fullScreen: false },
    { name: 'history', modal: false, fullScreen: false },
  ])
  expect(host.ready.catalog).toBe(1)
})
