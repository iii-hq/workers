/** Chromium missing on the engine (design §8.4): install, progress, failure with the hint, done → the tabs. */
import { describe, expect, it } from 'vitest'
import { callsOf, exists, FakeBrowser, rows, start, subsOf } from './fake'

const PROGRESS = 'browser::chromium-install-progress'

describe('Chromium missing', () => {
  it('shows the install instead of the tabs, without listening to tabs', async () => {
    const fake = new FakeBrowser([])
    fake.chromium = { found: false }
    const { host } = await start({ fake })
    expect(host.find('chromium-missing').props).toMatchObject({
      title: "Chromium isn't installed on the engine",
      description: "The browser worker needs Chrome for Testing on the engine's host.",
    })
    expect(exists(host, 'tabs')).toBe(false)
    expect(exists(host, 'new-tab')).toBe(false)
    expect(subsOf(host, 'browser::session-started')).toEqual([])
  })

  it('installs: binds the progress first, shows phase and megabytes, then lists the tabs', async () => {
    const fake = new FakeBrowser([])
    fake.chromium = { found: false }
    const { host } = await start({ fake })
    await host.press('chromium-install')
    expect(subsOf(host, PROGRESS)).toEqual([{}])
    const bound = host.requests.findIndex((r) => r.method === 'on' && r.args.trigger === PROGRESS)
    const install = host.requests.findIndex((r) => r.args.fn === 'browser::chromium::install')
    expect(install).toBeGreaterThan(bound)
    expect(host.find('chromium-phase').props?.text).toBe('Finding the latest Chrome for Testing…')
    expect(host.find('chromium-progress').props?.value).toBeUndefined()

    await host.emitTrigger(PROGRESS, {
      job_id: 'job1',
      phase: 'downloading',
      version: '131.0.6778.85',
      bytes_done: 82_000_000,
      bytes_total: 150_000_000,
      timestamp: 1,
    })
    expect(host.find('chromium-phase').props?.text).toBe('Downloading Chrome for Testing 131.0.6778.85 · 82 of 150 MB')
    expect(host.find('chromium-progress').props?.value).toBeCloseTo(0.5467, 3)

    await host.emitTrigger(PROGRESS, { job_id: 'job1', phase: 'extracting', bytes_done: 10_000_000, timestamp: 2 })
    expect(host.find('chromium-phase').props?.text).toBe('Unpacking · 10 MB')

    fake.chromium = { found: true }
    fake.tabs.push({ session_id: 's1', url: 'https://example.com/', title: 'Example' })
    await host.emitTrigger(PROGRESS, { job_id: 'job1', phase: 'done', bytes_done: 1, path: '/c/chrome', timestamp: 3 })
    expect(exists(host, 'chromium')).toBe(false)
    expect(subsOf(host, PROGRESS)).toEqual([])
    expect(rows(host).map((r) => r.key)).toEqual(['header:tabs', 's1'])
  })

  it('a failed install shows the error and the hint, and Retry installs again', async () => {
    const fake = new FakeBrowser([])
    fake.chromium = { found: false }
    const { host } = await start({ fake })
    await host.press('chromium-install')
    await host.emitTrigger(PROGRESS, {
      job_id: 'job1',
      phase: 'failed',
      bytes_done: 0,
      error: 'Chromium does not start: libnss3.so is missing',
      hint: 'sudo apt-get install libnss3',
      timestamp: 1,
    })
    expect(host.find('chromium-failed').props).toMatchObject({
      variant: 'alert',
      headline: "Chromium didn't install",
      detail: 'Chromium does not start: libnss3.so is missing\n\nsudo apt-get install libnss3',
    })
    await host.press('chromium-retry')
    expect(callsOf(host, 'browser::chromium::install')).toHaveLength(2)
  })

  it('already installed meanwhile: straight to the tabs', async () => {
    const fake = new FakeBrowser([])
    fake.chromium = { found: false }
    fake.overrides['browser::chromium::install'] = () => ({ status: 'already-installed', path: '/c/chrome' })
    const { host } = await start({ fake })
    await host.press('chromium-install')
    expect(exists(host, 'chromium')).toBe(false)
    expect(exists(host, 'no-tabs')).toBe(true)
  })

  it('a job already running shows its progress; visible again catches up from the status', async () => {
    const fake = new FakeBrowser([])
    fake.chromium = {
      found: false,
      job: { job_id: 'j', phase: 'downloading', bytes_done: 1_000_000, bytes_total: 2_000_000 },
    }
    const { host } = await start({ fake })
    expect(host.find('chromium-phase').props?.text).toBe('Downloading Chrome for Testing · 1 of 2 MB')
    expect(callsOf(host, 'browser::chromium::install')).toEqual([])
    await host.env({ visible: false })
    expect(subsOf(host, PROGRESS)).toEqual([])
    fake.chromium.job = { job_id: 'j', phase: 'verifying', bytes_done: 2_000_000 }
    await host.env({ visible: true })
    expect(host.find('chromium-phase').props?.text).toBe('Checking that it starts…')
  })

  it('a platform without a download says why, with no Install', async () => {
    const fake = new FakeBrowser([])
    fake.chromium = { found: false, supported: false }
    const { host } = await start({ fake })
    expect(host.find('chromium-missing').props?.description).toBe(
      "The browser worker needs Chrome for Testing on the engine's host. No build for this platform.",
    )
    expect(exists(host, 'chromium-install')).toBe(false)
  })

  it('a worker without chromium::status is taken as having a Chromium', async () => {
    const fake = new FakeBrowser([])
    fake.overrides['browser::chromium::status'] = () => {
      throw new Error('Function browser::chromium::status not found')
    }
    const { host } = await start({ fake })
    expect(exists(host, 'chromium')).toBe(false)
    expect(exists(host, 'no-tabs')).toBe(true)
  })
})
