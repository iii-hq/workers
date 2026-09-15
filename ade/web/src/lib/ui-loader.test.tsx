import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { PageHeader } from '@/components/ui/PageChrome'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type {
  ConfigFormProps,
  ConsoleApi,
  SetupFn,
  UiAssetsPush,
} from '../types/injectable-ui'
import type { IiiClient } from './iii-client'
import {
  type ConversationAdapter,
  startUiLoader,
  UI_ASSETS_FN,
} from './ui-loader'
import {
  getExtConfigForm,
  getExtPage,
  getExtProviderConfigForm,
  getExtTriggerActivityRenderers,
  getUiAssetsStatus,
  setUiAssetsStatus,
} from './ui-slots'

const wakeLockMocks = vi.hoisted(() => ({ acquire: vi.fn() }))
vi.mock('./screen-wake-lock', () => ({
  acquireScreenWakeLock: wakeLockMocks.acquire,
}))

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

function setupTriggerRenderer(label: string): UiModule {
  return {
    default(host) {
      host.triggerRenderers.register({
        id: `cron-${label}`,
        isMatch: (triggerType) => triggerType === 'cron',
        tryRender: () => <p>{label}</p>,
      })
    },
  }
}

function setupConfiguredPage(): UiModule {
  return {
    default(host) {
      host.pages.register({
        id: 'configured-page',
        title: 'Configured page',
        configurationId: 'browser',
        render: () => <PageHeader title="Configured page" />,
      })
    },
  }
}

function createHarness({
  conversationAdapter = {
    selectConversation: vi.fn(),
    openDraft: vi.fn(),
    composerModel: vi.fn(() => null),
  },
  importModule = vi.fn(async () => setupForm('default')),
  manifest = Promise.resolve({ disabled: false }),
}: {
  conversationAdapter?: ConversationAdapter
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
  const stop = startUiLoader(client, api, conversationAdapter, {
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
    client,
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
  wakeLockMocks.acquire.mockReset()
})

describe('extension screen leases', () => {
  it.each(['stop', 'delete', 'replace'] as const)(
    'blocks new leases before %s cleanup and drains registrations made during teardown',
    async (action) => {
      const order: string[] = []
      const release = vi.fn(() => {
        order.push('lease')
      })
      wakeLockMocks.acquire.mockReturnValue(release)
      const teardown = vi.fn()
      const harness = createHarness({
        importModule: vi
          .fn()
          .mockResolvedValueOnce({
            default: ((host) => {
              host.screen?.keepAwake()
              return () => {
                order.push('teardown')
                host.screen?.keepAwake()
                // Registrations added after the cleanup snapshot still need
                // their own cleanup pass; they must not outlive this script.
                host.configForms.register('teardown-only', () => (
                  <p>temporary</p>
                ))
                teardown()
              }
            }) satisfies SetupFn,
          })
          .mockResolvedValue({ default: () => undefined }),
      })
      harness.emit({
        event: 'sync',
        assets: [{ path: 'voice/page.js', kind: 'script', hash: 'one' }],
      })
      await vi.waitFor(() => expect(getUiAssetsStatus()).toBe('ready'))
      if (action === 'stop') harness.stop()
      else if (action === 'delete') {
        harness.emit({
          event: 'delete',
          path: 'voice/page.js',
          kind: 'script',
          hash: 'one',
        })
      } else {
        harness.emit({
          event: 'set',
          path: 'voice/page.js',
          kind: 'script',
          hash: 'two',
        })
      }
      await vi.waitFor(() => expect(teardown).toHaveBeenCalledTimes(1))
      expect(wakeLockMocks.acquire).toHaveBeenCalledTimes(1)
      expect(release).toHaveBeenCalledTimes(1)
      expect(order).toEqual(['teardown', 'lease'])
      expect(getExtConfigForm('teardown-only')).toBeUndefined()
      harness.stop()
    },
  )

  it('blocks reacquisition while cleaning up a failed setup', async () => {
    let keepAwake: (() => () => void) | undefined
    const release = vi.fn(() => {
      keepAwake?.()
    })
    wakeLockMocks.acquire.mockReturnValue(release)
    const error = vi.spyOn(console, 'error').mockImplementation(() => undefined)
    const harness = createHarness({
      importModule: async () => ({
        default(host) {
          keepAwake = host.screen?.keepAwake
          host.screen?.keepAwake()
          throw new Error('setup failed')
        },
      }),
    })
    harness.emit({
      event: 'sync',
      assets: [{ path: 'voice/page.js', kind: 'script', hash: 'one' }],
    })
    await vi.waitFor(() => expect(error).toHaveBeenCalledTimes(1))
    expect(release).toHaveBeenCalledTimes(1)
    keepAwake?.()()
    expect(wakeLockMocks.acquire).toHaveBeenCalledTimes(1)
    harness.stop()
  })

  it('releases unfinished activities on dispose without retaining finished leases', async () => {
    const finished = vi.fn()
    const unfinished = vi.fn()
    let keepAwake: (() => () => void) | undefined
    wakeLockMocks.acquire
      .mockReturnValueOnce(finished)
      .mockReturnValueOnce(unfinished)
    const harness = createHarness({
      importModule: async () => ({
        default(host) {
          keepAwake = host.screen?.keepAwake
          host.screen?.keepAwake()()
          host.screen?.keepAwake()
        },
      }),
    })
    harness.emit({
      event: 'sync',
      assets: [{ path: 'voice/page.js', kind: 'script', hash: 'one' }],
    })
    await vi.waitFor(() => expect(finished).toHaveBeenCalledTimes(1))
    expect(unfinished).not.toHaveBeenCalled()
    harness.stop()
    expect(finished).toHaveBeenCalledTimes(1)
    expect(unfinished).toHaveBeenCalledTimes(1)
    keepAwake?.()()
    expect(wakeLockMocks.acquire).toHaveBeenCalledTimes(2)
  })
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

describe('injectable page configuration metadata', () => {
  it('provides the page configuration id to console-owned header chrome', async () => {
    const harness = createHarness({
      importModule: vi.fn(async () => setupConfiguredPage()),
    })

    harness.emit({
      event: 'sync',
      assets: [{ path: 'browser/page.js', kind: 'script', hash: 'one' }],
    })
    await vi.waitFor(() => expect(getExtPage('configured-page')).toBeDefined())

    const page = getExtPage('configured-page')
    if (!page) throw new Error('configured page was not registered')
    const RegisteredPage = page.render
    const markup = renderToStaticMarkup(
      <TooltipProvider>
        <RegisteredPage panelSide="left" tabId="tab-1" paneId="tab-1:pane:0" />
      </TooltipProvider>,
    )
    expect(markup).toContain('aria-label="Configure worker"')

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

  it('hot-reloads trigger renderers atomically and cleans them up on stop', async () => {
    const candidate = deferred<UiModule>()
    const importModule = vi
      .fn<(url: string) => Promise<UiModule>>()
      .mockResolvedValueOnce(setupTriggerRenderer('old'))
      .mockImplementationOnce(() => candidate.promise)
    const harness = createHarness({ importModule })

    harness.emit({
      event: 'sync',
      assets: [{ path: 'cron/page.js', kind: 'script', hash: 'old' }],
    })
    await vi.waitFor(() =>
      expect(
        getExtTriggerActivityRenderers().map(({ renderer }) => renderer.id),
      ).toEqual(['cron-old']),
    )

    harness.emit({
      event: 'set',
      path: 'cron/page.js',
      kind: 'script',
      hash: 'new',
    })
    await vi.waitFor(() => expect(importModule).toHaveBeenCalledTimes(2))
    expect(
      getExtTriggerActivityRenderers().map(({ renderer }) => renderer.id),
    ).toEqual(['cron-old'])

    candidate.resolve(setupTriggerRenderer('new'))
    await vi.waitFor(() =>
      expect(
        getExtTriggerActivityRenderers().map(({ renderer }) => renderer.id),
      ).toEqual(['cron-new']),
    )

    harness.stop()
    expect(getExtTriggerActivityRenderers()).toEqual([])
  })
})

describe('injectable UI conversation adapters', () => {
  it('opens an editable investigation draft without selecting or sending a session', async () => {
    const openDraft = vi.fn()
    const selectConversation = vi.fn()
    const harness = createHarness({
      conversationAdapter: {
        openDraft,
        selectConversation,
        composerModel: () => null,
      },
      importModule: async () => ({
        default(host) {
          host.chat.openDraft({
            text: 'Investigate execution 123',
            title: 'Execution investigation',
          })
        },
      }),
    })

    harness.emit({
      event: 'sync',
      assets: [{ path: 'harness-e2e/page.js', kind: 'script', hash: 'one' }],
    })
    await vi.waitFor(() => expect(openDraft).toHaveBeenCalledOnce())
    expect(openDraft).toHaveBeenCalledWith({
      text: 'Investigate execution 123',
      title: 'Execution investigation',
    })
    expect(selectConversation).not.toHaveBeenCalled()
    expect(harness.client.trigger).not.toHaveBeenCalledWith(
      expect.stringMatching(/session::|harness::send/),
      expect.anything(),
    )
    harness.stop()
  })

  it('keeps concurrent loader hosts isolated through teardown and reload', async () => {
    const selectA = vi.fn()
    const selectB = vi.fn()
    const modelA = vi.fn(() => 'provider::model-a')
    const modelB = vi.fn(() => 'provider::model-b')
    const observed: string[] = []
    const moduleFor = (sessionId: string): UiModule => ({
      default(host) {
        host.chat.selectConversation?.(sessionId)
        observed.push(host.chat.composerModel?.('draft') ?? 'missing')
      },
    })
    const first = createHarness({
      conversationAdapter: {
        selectConversation: selectA,
        openDraft: vi.fn(),
        composerModel: modelA,
      },
      importModule: async () => moduleFor('session-a'),
    })
    const second = createHarness({
      conversationAdapter: {
        selectConversation: selectB,
        openDraft: vi.fn(),
        composerModel: modelB,
      },
      importModule: async () => moduleFor('session-b'),
    })

    first.emit({
      event: 'sync',
      assets: [{ path: 'first/page.js', kind: 'script', hash: 'one' }],
    })
    second.emit({
      event: 'sync',
      assets: [{ path: 'second/page.js', kind: 'script', hash: 'one' }],
    })
    await vi.waitFor(() => expect(observed).toHaveLength(2))
    expect(selectA).toHaveBeenCalledWith('session-a')
    expect(selectA).not.toHaveBeenCalledWith('session-b')
    expect(selectB).toHaveBeenCalledWith('session-b')
    expect(selectB).not.toHaveBeenCalledWith('session-a')
    expect(observed).toEqual(['provider::model-a', 'provider::model-b'])

    first.stop()
    second.emit({
      event: 'set',
      path: 'second/page.js',
      kind: 'script',
      hash: 'two',
    })
    await vi.waitFor(() => expect(selectB).toHaveBeenCalledTimes(2))
    expect(selectA).toHaveBeenCalledTimes(1)
    expect(modelB).toHaveBeenCalledTimes(2)
    second.stop()
  })
})
