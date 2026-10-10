// @vitest-environment jsdom

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type {
  ChromiumInstallOutcome,
  ChromiumInstallProgress,
  ChromiumState,
} from '@/lib/onboarding/chromium'
import { BrowserStep } from './BrowserStep'
import { isChromiumMissingCall } from './ChromiumMissingAction'
import { stepPosition } from './OnboardingWizard'
import { ReadyStep } from './ReadyStep'
import type {
  ActivityEntry,
  MachineSnapshot,
  OnboardingController,
} from './use-onboarding'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const MISSING: ChromiumState = {
  found: false,
  engine: 'chromium',
  canInstall: true,
  platform: 'linux64',
  installDir: '~/.cache/iii/browser/chrome',
  approxDownloadMb: 172,
  legacy: false,
}

const READY: ChromiumState = {
  ...MISSING,
  found: true,
  version: '131.0.6778.85',
  path: '~/.cache/iii/browser/chrome/131.0.6778.85/chrome-linux64/chrome',
  source: 'managed',
}

function controller(
  overrides: Partial<Omit<OnboardingController, 'snapshot'>> & {
    snapshot?: Partial<MachineSnapshot>
  } = {},
): OnboardingController {
  const { snapshot, ...rest } = overrides
  return {
    snapshot: {
      tools: [],
      toolsError: null,
      providers: [],
      providersError: null,
      detections: null,
      envFile: '.env',
      installed: new Set(['browser']),
      judgeProvider: null,
      consoleConfig: null,
      browser: MISSING,
      browserError: null,
      ...snapshot,
    },
    scanning: false,
    refresh: async () => undefined,
    activity: [],
    running: null,
    run: async () => true,
    judgeInstalled: false,
    checkChromium: async () => null,
    installChromium: async () => ({ ok: true }),
    chromiumProgress: null,
    ...rest,
  }
}

const noop = () => undefined
const POSITION = { index: 2, total: 3 }

function html(onboarding: OnboardingController): string {
  return renderToStaticMarkup(
    <BrowserStep
      onboarding={onboarding}
      position={POSITION}
      onBack={noop}
      onNext={noop}
    />,
  )
}

const downloadEntry = (status: ActivityEntry['status']): ActivityEntry => ({
  id: 1,
  group: 'browser',
  title: 'Download Chromium',
  status,
})

describe('BrowserStep', () => {
  it('offers the download in one button, with its size but not where it goes', () => {
    const out = html(controller())
    expect(out).toContain('Step 2 of 3 · Optional')
    expect(out).toContain('Let agents check their work in a browser')
    expect(out).toContain('This machine doesn&#x27;t have it yet.')
    expect(out).toContain('Download Chromium')
    expect(out).toContain('About 172 MB, downloaded once for this machine.')
    expect(out).toContain('leaves any Chrome you install yourself untouched')
    // Lean, for someone exploring iii: no install folder, no build jargon.
    expect(out).not.toContain('~/.cache/iii/browser')
    expect(out).not.toContain('Chrome for Testing')
    expect(out).toContain('Skip')
    // The manual path is there, folded away.
    expect(out).toContain('I&#x27;d rather install it myself')
    expect(out).toContain('sudo apt install chromium')
    expect(out).not.toMatch(/<details[^>]*\sopen/)
  })

  it('holds a skeleton while Chromium is being looked for', () => {
    const out = html(
      controller({ scanning: true, snapshot: { browser: null } }),
    )
    expect(out).toContain('Looking for Chromium')
    expect(out).not.toContain('This machine doesn')
  })

  it('waits for the first look before saying there is no browser worker', () => {
    const before = html(
      controller({
        snapshot: { tools: null, browser: null, installed: new Set() },
      }),
    )
    expect(before).toContain('Looking for Chromium')
    const after = html(
      controller({ snapshot: { browser: null, installed: new Set() } }),
    )
    expect(after).toContain('doesn&#x27;t run the browser worker')
    expect(after).toContain('Continue')
  })

  it('says when the browser worker could not be asked', () => {
    const out = html(
      controller({ snapshot: { browser: null, browserError: 'timeout' } }),
    )
    expect(out).toContain('did not say whether it has Chromium: timeout')
    expect(out).toContain('Check again')
  })

  it('shows MB done of the total while it downloads', () => {
    const progress: ChromiumInstallProgress = {
      job_id: 'job-1',
      phase: 'downloading',
      version: '131.0.6778.85',
      bytes_done: 43_000_000,
      bytes_total: 172_000_000,
      timestamp: 1,
    }
    const out = html(
      controller({
        running: 'browser',
        chromiumProgress: progress,
        activity: [downloadEntry('running')],
      }),
    )
    expect(out).toContain(
      'Downloading Chromium 131.0.6778.85 · 43 MB of 172 MB',
    )
    expect(out).toContain('25%')
    expect(out).toContain('aria-valuenow="25"')
    expect(out).toMatch(/<button[^>]*disabled[^>]*>.*Downloading…/)
    // Nothing else to choose mid-download.
    expect(out).not.toContain('I&#x27;d rather install it myself')
  })

  it('says Chromium is ready once it downloaded, and moves on', () => {
    const out = html(
      controller({
        snapshot: { browser: READY },
        activity: [downloadEntry('done')],
      }),
    )
    expect(out).toContain('Chromium is ready')
    expect(out).not.toContain('131.0.6778.85')
    expect(out).not.toContain('downloaded by setup')
    expect(out).not.toContain('~/.cache/iii/browser')
    expect(out).toContain('Continue')
    expect(out).not.toContain('Skip')
    expect(out).not.toMatch(/<button[^>]*>Download Chromium<\/button>/)
  })

  it('says so when a Chromium was already there', () => {
    const out = html(
      controller({
        snapshot: {
          browser: { ...READY, source: 'system', path: '/usr/bin/chromium' },
        },
      }),
    )
    expect(out).toContain('is already here')
    expect(out).not.toContain('/usr/bin/chromium')
  })

  it('offers only the manual path where the worker cannot download', () => {
    const out = html(
      controller({
        snapshot: {
          browser: {
            ...MISSING,
            canInstall: false,
            platform: 'mac-arm64',
            installReason: 'No Chrome for Testing build for this system.',
          },
        },
      }),
    )
    expect(out).not.toContain('Download Chromium')
    expect(out).toContain('No Chrome for Testing build for this system.')
    expect(out).toContain('brew install --cask google-chrome')
    expect(out).toMatch(/<details[^>]*\sopen/)
    expect(out).toContain('Check again')
  })

  it('offers only the manual path on an older browser worker', () => {
    const out = html(
      controller({
        snapshot: {
          browser: {
            found: false,
            engine: 'chromium',
            canInstall: false,
            installReason:
              'This version of the browser worker cannot download Chromium by itself.',
            approxDownloadMb: 170,
            legacy: true,
          },
        },
      }),
    )
    expect(out).not.toContain('Download Chromium')
    expect(out).toContain('cannot download Chromium by itself')
  })
})

describe('BrowserStep, interactive', () => {
  const mounted: Array<() => void> = []
  afterEach(() => {
    for (const unmount of mounted.splice(0)) unmount()
  })

  function mount(onboarding: OnboardingController) {
    const container = document.createElement('div')
    document.body.appendChild(container)
    const root = createRoot(container)
    const render = (next: OnboardingController) =>
      act(() =>
        root.render(
          <BrowserStep
            onboarding={next}
            position={POSITION}
            onBack={noop}
            onNext={noop}
          />,
        ),
      )
    render(onboarding)
    mounted.push(() => {
      act(() => root.unmount())
      container.remove()
    })
    return { container, render }
  }

  const button = (label: string) =>
    [...document.querySelectorAll('button')].find(
      (el) => el.textContent?.trim() === label,
    )

  it('shows the error and its hint after a failed download, and retries', async () => {
    const outcomes: ChromiumInstallOutcome[] = [
      {
        ok: false,
        error: 'Chromium does not start: libnss3.so is missing.',
        hint: 'sudo apt install libnss3 libatk-bridge2.0-0',
      },
      { ok: true, version: '131.0' },
    ]
    const installChromium = vi.fn(
      async () => outcomes.shift() as ChromiumInstallOutcome,
    )
    const { container } = mount(controller({ installChromium }))
    await act(async () => button('Download Chromium')?.click())
    const alert = container.querySelector('[role="alert"]')
    expect(alert?.textContent).toContain('libnss3.so is missing')
    expect(alert?.textContent).toContain('sudo apt install libnss3')
    await act(async () => button('Retry download')?.click())
    expect(installChromium).toHaveBeenCalledTimes(2)
    expect(container.querySelector('[role="alert"]')).toBeNull()
  })

  it('checks again after a manual install and says when it is still missing', async () => {
    const checkChromium = vi.fn(async () => MISSING)
    const { container } = mount(controller({ checkChromium }))
    await act(async () => button('Check again')?.click())
    expect(checkChromium).toHaveBeenCalledTimes(1)
    expect(container.textContent).toContain('Still no Chromium')
  })
})

describe('setup step numbering', () => {
  it('counts only the steps that set something up', () => {
    const withBrowser = [
      { id: 'welcome' as const },
      { id: 'models' as const },
      { id: 'browser' as const },
      { id: 'judge' as const },
      { id: 'ready' as const },
    ]
    expect(stepPosition(withBrowser, 'models')).toEqual({ index: 1, total: 3 })
    expect(stepPosition(withBrowser, 'browser')).toEqual({ index: 2, total: 3 })
    expect(stepPosition(withBrowser, 'judge')).toEqual({ index: 3, total: 3 })
    const without = withBrowser.filter((entry) => entry.id !== 'browser')
    expect(stepPosition(without, 'judge')).toEqual({ index: 2, total: 2 })
    expect(stepPosition(without, 'ready')).toBeUndefined()
  })
})

describe('ReadyStep after a Chromium download', () => {
  it('lists Chromium among what setup did', () => {
    const out = renderToStaticMarkup(
      <ReadyStep
        onboarding={controller({
          snapshot: { browser: READY },
          activity: [downloadEntry('done')],
        })}
        judges={[]}
        prompts={[]}
        agentNames={new Map()}
        onPrompt={noop}
        tour={{ kind: 'idle' }}
        onStartTour={noop}
        onStart={noop}
      />,
    )
    expect(out).toContain('Chromium is ready for agents')
    expect(out).not.toContain('131.0.6778.85')
    expect(out).not.toContain('~/.cache/iii/browser')
  })

  it('says nothing about Chromium when setup did not download it', () => {
    const out = renderToStaticMarkup(
      <ReadyStep
        onboarding={controller({ snapshot: { browser: READY } })}
        judges={[]}
        prompts={[]}
        agentNames={new Map()}
        onPrompt={noop}
        tour={{ kind: 'idle' }}
        onStartTour={noop}
        onStart={noop}
      />,
    )
    expect(out).not.toContain('Chromium')
  })
})

describe('chat: a browser call that failed for want of Chromium', () => {
  it('is recognised by the worker’s marker on browser calls only', () => {
    const output = {
      error: {
        kind: 'handler',
        message:
          'chromium_missing: no Chromium or Chrome found. Install it from the ADE (Set up the harness → Browser).',
      },
    }
    expect(isChromiumMissingCall('browser::sessions::start', output)).toBe(true)
    expect(isChromiumMissingCall('shell::exec', output)).toBe(false)
    expect(
      isChromiumMissingCall('browser::navigate', {
        error: { message: 'timeout' },
      }),
    ).toBe(false)
  })
})
