import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type {
  ConfigFormProps,
  ConsoleApi,
  SetupFn,
  UiAssetsPush,
} from '../types/injectable-ui'
import type { IiiClient } from './iii-client'
import { startUiLoader, UI_ASSETS_FN } from './ui-loader'
import {
  getExtConfigForm,
  getExtProviderConfigForm,
  getUiAssetsStatus,
  setUiAssetsStatus,
} from './ui-slots'

type UiModule = { default?: SetupFn }

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, reject, resolve }
}

function setupForm(label: string): UiModule {
  return {
    default(host) {
      host.configForms.register('llm-router', () => <p>{label}</p>)
      host.providerConfigForms.register('openai-codex', () => (
        <p>{label} provider</p>
      ))
    },
  }
}

function createHarness({
  importModule = vi.fn(async () => setupForm('default')),
  manifest = Promise.resolve({ disabled: false }),
}: {
  importModule?: (url: string) => Promise<UiModule>
  manifest?: Promise<{ disabled: boolean }>
} = {}) {
  let handler: ((payload: UiAssetsPush) => void | Promise<void>) | undefined
  const offHandler = vi.fn()
  const offTrigger = vi.fn()
  const client = {
    browserId: 'test-browser',
    trigger: vi.fn(() => manifest),
    on: vi.fn((functionId, nextHandler) => {
      expect(functionId).toBe(UI_ASSETS_FN)
      handler = nextHandler
      return offHandler
    }),
    registerTrigger: vi.fn(() => offTrigger),
  } as unknown as IiiClient
  const api = {
    iii: client,
    components: {},
    tokens: [],
    uiClasses: {} as ConsoleApi['uiClasses'],
    useTheme: () => 'light',
  } as ConsoleApi
  const stop = startUiLoader(client, api, {
    baseUrl: new URL('http://console.test/base/'),
    importModule,
  })
  return {
    emit(payload: UiAssetsPush) {
      if (!handler) throw new Error('asset handler was not registered')
      return handler(payload)
    },
    offHandler,
    offTrigger,
    stop,
  }
}

function renderCurrentForm(): string {
  const registration = getExtConfigForm('llm-router')
  if (!registration) return ''
  const props: ConfigFormProps = {
    id: 'llm-router',
    schema: null,
    value: {},
    onChange: vi.fn(),
  }
  return renderToStaticMarkup(<registration.component {...props} />)
}

afterEach(() => {
  setUiAssetsStatus('unavailable')
  vi.restoreAllMocks()
})

describe('injectable UI loader readiness', () => {
  it('stays loading until the initial sync is fully applied', async () => {
    const candidate = deferred<UiModule>()
    const harness = createHarness({ importModule: () => candidate.promise })

    expect(getUiAssetsStatus()).toBe('loading')
    harness.emit({
      event: 'sync',
      assets: [{ path: 'llm-router/page.js', kind: 'script', hash: 'one' }],
    })
    await vi.waitFor(() => expect(renderCurrentForm()).toBe(''))
    expect(getUiAssetsStatus()).toBe('loading')

    candidate.resolve(setupForm('custom form'))
    await vi.waitFor(() => {
      expect(getUiAssetsStatus()).toBe('ready')
      expect(renderCurrentForm()).toContain('custom form')
      expect(getExtProviderConfigForm('openai-codex')).toBeDefined()
    })

    harness.stop()
    expect(renderCurrentForm()).toBe('')
    expect(getExtProviderConfigForm('openai-codex')).toBeUndefined()
  })

  it('falls back when injectable UI is disabled', async () => {
    const harness = createHarness({
      manifest: Promise.resolve({ disabled: true }),
    })

    expect(getUiAssetsStatus()).toBe('loading')
    await vi.waitFor(() => expect(getUiAssetsStatus()).toBe('unavailable'))

    harness.stop()
  })
})

describe('injectable UI script updates', () => {
  it('keeps the last good form until its replacement finishes setup', async () => {
    const candidate = deferred<UiModule>()
    const importModule = vi
      .fn<(url: string) => Promise<UiModule>>()
      .mockResolvedValueOnce(setupForm('old form'))
      .mockImplementationOnce(() => candidate.promise)
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined)
    const harness = createHarness({ importModule })

    harness.emit({
      event: 'sync',
      assets: [{ path: 'llm-router/page.js', kind: 'script', hash: 'old' }],
    })
    await vi.waitFor(() => expect(renderCurrentForm()).toContain('old form'))

    harness.emit({
      event: 'set',
      path: 'llm-router/page.js',
      kind: 'script',
      hash: 'new',
    })
    await vi.waitFor(() => expect(importModule).toHaveBeenCalledTimes(2))
    expect(renderCurrentForm()).toContain('old form')

    candidate.resolve(setupForm('new form'))
    await vi.waitFor(() => expect(renderCurrentForm()).toContain('new form'))
    expect(warn).not.toHaveBeenCalled()

    harness.stop()
  })

  it('retains the last good form when a replacement fails', async () => {
    const importModule = vi
      .fn<(url: string) => Promise<UiModule>>()
      .mockResolvedValueOnce(setupForm('old form'))
      .mockRejectedValueOnce(new Error('broken module'))
    const error = vi.spyOn(console, 'error').mockImplementation(() => undefined)
    const harness = createHarness({ importModule })

    harness.emit({
      event: 'sync',
      assets: [{ path: 'llm-router/page.js', kind: 'script', hash: 'old' }],
    })
    await vi.waitFor(() => expect(renderCurrentForm()).toContain('old form'))

    harness.emit({
      event: 'set',
      path: 'llm-router/page.js',
      kind: 'script',
      hash: 'broken',
    })
    await vi.waitFor(() => expect(error).toHaveBeenCalledTimes(1))
    expect(renderCurrentForm()).toContain('old form')

    harness.stop()
  })
})
