// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { registerExtProviderConfigForm } from '@/lib/ui-slots'
import type {
  JsonSchema,
  JsonValue,
} from '@/pages/Configuration/tabs/WorkersTab/api'
import { configurationKeys } from '@/pages/Configuration/tabs/WorkersTab/hooks'
import type { ProviderConfigFormProps } from '@/types/injectable-ui'
import {
  mergeProviderConfigurationValue,
  ProviderConfigurationPanel,
  providerSliceFromValue,
} from './ProviderConfigurationPanel'

const transport = vi.hoisted(() => ({
  entries: [] as Array<Record<string, unknown>>,
  schemas: new Map<string, Record<string, unknown>>(),
  values: new Map<string, JsonValue>(),
  listError: null as unknown,
  schemaErrors: new Map<string, unknown>(),
  valueErrors: new Map<string, unknown>(),
  setError: null as unknown,
  pendingSet: null as { promise: Promise<unknown> } | null,
  pendingGets: new Map<string, { promise: Promise<unknown> }>(),
  trigger: vi.fn(),
}))

const conversationContext = vi.hoisted(() => ({
  current: null as {
    presentProviders: Array<Record<string, unknown>>
    modelOptions: Array<{ id: string }>
    refreshModels: () => Promise<void>
  } | null,
}))

vi.mock('@/lib/conversations-context', () => ({
  useConversationsCtxOptional: () => conversationContext.current,
}))

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: () => Promise.resolve({ trigger: transport.trigger }),
}))

const schemaFor = (providerIds: string[] = ['openai-codex']): JsonSchema => ({
  type: 'object',
  properties: {
    providers: {
      type: 'object',
      properties: Object.fromEntries(
        providerIds.map((providerId) => [
          providerId,
          {
            type: 'object',
            properties: {
              api_url: { type: 'string' },
              max_tokens: { type: 'integer', minimum: 1 },
            },
          },
        ]),
      ),
    },
  },
})

function configure({
  id = 'team-blue-router',
  value = {
    providers: {
      'openai-codex': { api_url: 'https://old.example', max_tokens: 3 },
    },
  },
  providerIds = ['openai-codex'],
}: {
  id?: string
  value?: JsonValue
  providerIds?: string[]
} = {}) {
  transport.entries = [{ id, metadata: { ui_form: 'llm-router' } }]
  transport.schemas = new Map([
    [
      id,
      {
        id,
        name: id,
        description: 'router',
        schema: schemaFor(providerIds),
        metadata: { ui_form: 'llm-router' },
      },
    ],
  ])
  transport.values = new Map([[id, value]])
}

function resetTransport() {
  configure()
  transport.listError = null
  transport.schemaErrors.clear()
  transport.valueErrors.clear()
  transport.setError = null
  transport.pendingSet = null
  transport.pendingGets.clear()
  transport.trigger.mockReset()
  transport.trigger.mockImplementation(
    async (functionId: string, payload: Record<string, unknown> = {}) => {
      if (functionId === 'configuration::list') {
        if (transport.listError) throw transport.listError
        return { configurations: transport.entries }
      }
      const id = String(payload.id ?? '')
      if (functionId === 'configuration::schema') {
        const error = transport.schemaErrors.get(id)
        if (error) throw error
        const response = transport.schemas.get(id)
        if (!response) throw { code: 'NOT_FOUND' }
        return response
      }
      if (functionId === 'configuration::get') {
        const error = transport.valueErrors.get(id)
        if (error) throw error
        const read = () => {
          const value = transport.values.get(id)
          if (value === undefined) throw { code: 'NOT_FOUND' }
          return { id, value }
        }
        const pending = transport.pendingGets.get(id)
        return pending ? pending.promise.then(read) : read()
      }
      if (functionId === 'configuration::set') {
        if (transport.setError) throw transport.setError
        const apply = () => {
          const oldValue = transport.values.get(id) ?? null
          transport.values.set(id, payload.value as JsonValue)
          return { old_value: oldValue, new_value: payload.value }
        }
        if (transport.pendingSet) {
          return transport.pendingSet.promise.then(apply)
        }
        return apply()
      }
      throw new Error(`unexpected trigger: ${functionId}`)
    },
  )
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

function Harness({ providerId = 'openai-codex' }: { providerId?: string }) {
  const [, setVersion] = useState(0)
  return (
    <QueryClientProvider client={queryClient}>
      <ProviderConfigurationPanel
        providerId={providerId}
        className="test-panel"
        onDirtyChange={() => setVersion((version) => version)}
      />
    </QueryClientProvider>
  )
}

let root: Root | undefined
let container: HTMLDivElement | undefined
let queryClient: QueryClient
let removeOverride: (() => void) | undefined
let observedOverrideProps: ProviderConfigFormProps | undefined

function TestProviderOverride(props: ProviderConfigFormProps) {
  observedOverrideProps = props
  return (
    <div data-testid="provider-override">
      <button
        type="button"
        onClick={() =>
          props.onChange({
            api_url: 'https://override.example',
            max_tokens: 3,
          })
        }
      >
        edit override
      </button>
      {props.errors?.get('/api_url') ? (
        <p role="alert">{props.errors.get('/api_url')}</p>
      ) : null}
    </div>
  )
}

function mount(providerId = 'openai-codex') {
  container = document.createElement('div')
  document.body.appendChild(container)
  root = createRoot(container)
  act(() => root?.render(<Harness providerId={providerId} />))
  return container
}

function renderProvider(providerId: string) {
  act(() => root?.render(<Harness providerId={providerId} />))
}

function button(label: string): HTMLButtonElement {
  const match = [...(container?.querySelectorAll('button') ?? [])].find(
    (candidate) =>
      candidate.textContent?.toLowerCase().includes(label.toLowerCase()),
  )
  expect(match, `button ${label}`).toBeTruthy()
  return match as HTMLButtonElement
}

function changeInput(input: HTMLInputElement, value: string) {
  act(() => {
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      'value',
    )?.set
    setter?.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
    input.dispatchEvent(new Event('change', { bubbles: true }))
  })
}

async function flush() {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
    await new Promise((resolve) => setTimeout(resolve, 0))
    await Promise.resolve()
  })
}

async function clickButton(label: string) {
  await act(async () => {
    button(label).click()
  })
  await flush()
}

async function waitForText(text: string, timeoutMs = 1000) {
  const start = Date.now()
  while (!container?.textContent?.includes(text)) {
    if (Date.now() - start > timeoutMs) {
      throw new Error(`Timed out waiting for ${text}`)
    }
    await flush()
  }
}

function callsFor(functionId: string) {
  return transport.trigger.mock.calls.filter(
    ([calledId]) => calledId === functionId,
  )
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        refetchOnWindowFocus: false,
        staleTime: 0,
      },
      mutations: { retry: false },
    },
  })
  resetTransport()
  conversationContext.current = null
  observedOverrideProps = undefined
})

afterEach(() => {
  act(() => root?.unmount())
  container?.remove()
  root = undefined
  container = undefined
  queryClient.clear()
  removeOverride?.()
  removeOverride = undefined
  conversationContext.current = null
  observedOverrideProps = undefined
  vi.unstubAllGlobals()
})

describe('ProviderConfigurationPanel value boundaries', () => {
  it('handles null safely and preserves opaque roots, siblings, credentials and templates', () => {
    expect(providerSliceFromValue(null, 'openai-codex')).toEqual({
      kind: 'ready',
      value: {},
    })
    expect(
      providerSliceFromValue(
        { providers: { 'openai-codex': null, sibling: { keep: true } } },
        'openai-codex',
      ),
    ).toEqual({ kind: 'ready', value: {} })
    expect(providerSliceFromValue('opaque', 'openai-codex').kind).toBe(
      'invalid',
    )

    const raw = {
      root_field: 'preserve',
      credentials: ['$', '{CODEX_KEY:template}'].join(''),
      providers: {
        sibling: { keep: true },
        'openai-codex': { old: true },
      },
    }
    expect(
      mergeProviderConfigurationValue(raw, 'openai-codex', { new: true }),
    ).toEqual({
      root_field: 'preserve',
      credentials: ['$', '{CODEX_KEY:template}'].join(''),
      providers: {
        sibling: { keep: true },
        'openai-codex': { new: true },
      },
    })
  })

  it.each(['default-llm-router', 'team-blue-router'])(
    'reads the resolved %s id',
    async (id) => {
      configure({ id })
      mount()
      await waitForText('Provider settings')
      expect(
        callsFor('configuration::schema').map(([_, payload]) => payload.id),
      ).toContain(id)
      expect(
        callsFor('configuration::get').map(([_, payload]) => payload.id),
      ).toContain(id)
      expect(
        callsFor('configuration::schema').some(
          ([_, payload]) => payload.id === 'llm-router',
        ),
      ).toBe(false)
    },
  )

  it('also supports the literal llm-router id without inventing an alias', async () => {
    configure({ id: 'llm-router' })
    const view = mount()
    await flush()
    expect(view.textContent).toContain('Provider settings')
    expect(callsFor('configuration::schema').at(-1)?.[1]).toEqual({
      id: 'llm-router',
    })
    expect(callsFor('configuration::get').at(-1)?.[1]).toEqual({
      id: 'llm-router',
      raw: true,
    })
  })
})

describe('ProviderConfigurationPanel lifecycle with real QueryClient hooks', () => {
  it('hydrates null without an automatic set', async () => {
    configure({ value: null })
    const view = mount()
    await flush()
    expect(view.querySelector('input[inputmode="url"]')).not.toBeNull()
    expect(callsFor('configuration::set')).toHaveLength(0)
  })

  it('does not read or save when the family is missing or ambiguous', async () => {
    transport.entries = []
    const missing = mount()
    await flush()
    expect(missing.textContent).toContain('No live router configuration')
    expect(callsFor('configuration::schema')).toHaveLength(0)
    expect(callsFor('configuration::get')).toHaveLength(0)

    act(() => root?.unmount())
    missing.remove()
    container = document.createElement('div')
    document.body.appendChild(container)
    root = createRoot(container)
    transport.entries = [
      { id: 'router-a', metadata: { ui_form: 'llm-router' } },
      { id: 'router-b', metadata: { ui_form: 'llm-router' } },
    ]
    queryClient.removeQueries({ queryKey: configurationKeys.list() })
    act(() => root?.render(<Harness />))
    await flush()
    expect(container.textContent).toContain('Open Settings')
    expect(container.textContent).toContain('router-a, router-b')
    expect(callsFor('configuration::schema')).toHaveLength(0)
  })

  it('blocks save when a live dirty draft loses a read prerequisite', async () => {
    const view = mount()
    await waitForText('Provider settings')
    const url = view.querySelector<HTMLInputElement>('input[inputmode="url"]')
    expect(url).not.toBeNull()
    if (!url) return
    changeInput(url, 'https://draft.example')
    await flush()

    transport.listError = new Error('registry offline')
    await act(async () => {
      await queryClient.invalidateQueries({
        queryKey: configurationKeys.list(),
      })
    })
    await flush()
    expect(view.textContent).toContain('Could not resolve router configuration')
    expect(button('save').disabled).toBe(true)
    expect(callsFor('configuration::set')).toHaveLength(0)
  })

  it('re-resolves the family on Retry without transferring the dirty draft', async () => {
    configure({
      value: {
        providers: {
          'openai-codex': {
            api_url: 'https://old.example',
            max_tokens: 3,
            api_key: 'A-SECRET',
          },
        },
      },
    })
    const view = mount()
    await waitForText('Provider settings')
    await flush()
    expect(view.textContent).toContain('Provider settings')
    const url = view.querySelector<HTMLInputElement>('input[inputmode="url"]')
    expect(url).not.toBeNull()
    if (!url) return
    changeInput(url, 'https://draft.example')
    await flush()

    transport.entries = [
      { id: 'renamed-router', metadata: { ui_form: 'llm-router' } },
    ]
    transport.schemas = new Map([
      [
        'renamed-router',
        {
          id: 'renamed-router',
          name: 'renamed-router',
          description: 'router',
          schema: schemaFor(),
        },
      ],
    ])
    transport.values = new Map([
      [
        'renamed-router',
        {
          providers: {
            'openai-codex': {
              api_url: 'https://server.example',
              max_tokens: 3,
              api_key: 'B-KEY',
            },
          },
        },
      ],
    ])
    transport.schemaErrors.set('team-blue-router', new Error('NOT_FOUND'))
    transport.valueErrors.set('team-blue-router', new Error('NOT_FOUND'))
    await act(async () => {
      await queryClient.invalidateQueries({
        queryKey: configurationKeys.schema('team-blue-router'),
      })
      await queryClient.invalidateQueries({
        queryKey: configurationKeys.rawValue('team-blue-router'),
      })
    })
    await flush()
    expect(view.textContent).toContain('Could not load provider configuration')

    transport.schemaErrors.delete('team-blue-router')
    transport.valueErrors.delete('team-blue-router')
    await clickButton('Retry')
    await flush()
    await flush()
    expect(
      view.querySelector<HTMLInputElement>('input[inputmode="url"]')?.value,
    ).toBe('https://server.example')
    expect(view.textContent).not.toContain('unsaved changes')
    // Retrying only rehydrates the destination. It must not write, and it
    // must not carry A's dirty data or secret into B.
    expect(callsFor('configuration::set')).toHaveLength(0)
    changeInput(
      view.querySelector<HTMLInputElement>(
        'input[inputmode="url"]',
      ) as HTMLInputElement,
      'https://b-saved.example',
    )
    await clickButton('save')
    const writes = callsFor('configuration::set')
    expect(writes).toHaveLength(1)
    expect(writes[0]?.[1]).toEqual({
      id: 'renamed-router',
      value: {
        providers: {
          'openai-codex': {
            api_url: 'https://b-saved.example',
            max_tokens: 3,
            api_key: 'B-KEY',
          },
        },
      },
    })
    expect(JSON.stringify(writes[0]?.[1])).not.toContain('A-SECRET')
    expect(JSON.stringify(writes[0]?.[1])).not.toContain(
      'https://draft.example',
    )
    expect(callsFor('configuration::list').length).toBeGreaterThanOrEqual(2)
    expect(
      callsFor('configuration::schema').some(
        ([_, payload]) => payload.id === 'renamed-router',
      ),
    ).toBe(true)
  })

  it('consumes a reset draft and does not resurrect it after A→B→A', async () => {
    configure({
      providerIds: ['openai-codex', 'openrouter'],
      value: {
        providers: {
          'openai-codex': { api_url: 'https://a.example', max_tokens: 3 },
          openrouter: { api_url: 'https://b.example', max_tokens: 4 },
        },
      },
    })
    const view = mount('openai-codex')
    await flush()
    const input = view.querySelector<HTMLInputElement>('input[inputmode="url"]')
    expect(input).not.toBeNull()
    if (!input) return
    changeInput(input, 'https://discarded.example')
    await flush()
    renderProvider('openrouter')
    await flush()
    renderProvider('openai-codex')
    await flush()
    expect(
      view.querySelector<HTMLInputElement>('input[inputmode="url"]')?.value,
    ).toBe('https://discarded.example')
    await clickButton('reset')
    expect(
      view.querySelector<HTMLInputElement>('input[inputmode="url"]')?.value,
    ).toBe('https://a.example')
    renderProvider('openrouter')
    await flush()
    renderProvider('openai-codex')
    await flush()
    expect(
      view.querySelector<HTMLInputElement>('input[inputmode="url"]')?.value,
    ).toBe('https://a.example')
    expect(view.textContent).not.toContain('unsaved changes')
  })

  it('uses the saved server value as the baseline after A→B→A', async () => {
    configure({
      providerIds: ['openai-codex', 'openrouter'],
      value: {
        providers: {
          'openai-codex': { api_url: 'https://a.example', max_tokens: 3 },
          openrouter: { api_url: 'https://b.example', max_tokens: 4 },
        },
      },
    })
    const view = mount('openai-codex')
    await flush()
    const input = view.querySelector<HTMLInputElement>('input[inputmode="url"]')
    expect(input).not.toBeNull()
    if (!input) return
    changeInput(input, 'https://saved.example')
    await flush()
    await clickButton('save')
    expect(transport.values.get('team-blue-router')).toEqual({
      providers: {
        'openai-codex': { api_url: 'https://saved.example', max_tokens: 3 },
        openrouter: { api_url: 'https://b.example', max_tokens: 4 },
      },
    })
    renderProvider('openrouter')
    await flush()
    renderProvider('openai-codex')
    await flush()
    expect(
      view.querySelector<HTMLInputElement>('input[inputmode="url"]')?.value,
    ).toBe('https://saved.example')
    expect(view.textContent).not.toContain('unsaved changes')
  })

  it('keeps the saved value visible while invalidation GET is pending', async () => {
    const view = mount()
    await waitForText('Provider settings')
    const pendingGet = deferred<unknown>()
    transport.pendingGets.set('team-blue-router', pendingGet)
    const url = view.querySelector<HTMLInputElement>('input[inputmode="url"]')
    expect(url).not.toBeNull()
    if (!url) return
    changeInput(url, 'https://saved.example')
    await flush()
    await clickButton('save')
    await flush()
    expect(
      view.querySelector<HTMLInputElement>('input[inputmode="url"]')?.value,
    ).toBe('https://saved.example')
    expect(view.textContent).not.toContain('Saved')
    expect(callsFor('configuration::set')).toHaveLength(1)
    await act(async () => pendingGet.resolve(undefined))
    await flush()
    expect(
      view.querySelector<HTMLInputElement>('input[inputmode="url"]')?.value,
    ).toBe('https://saved.example')
  })

  it('keeps an edit made during save and leaves the draft dirty', async () => {
    const pending = deferred<unknown>()
    transport.pendingSet = pending
    const view = mount()
    await waitForText('Provider settings')
    const url = view.querySelector<HTMLInputElement>('input[inputmode="url"]')
    expect(url).not.toBeNull()
    if (!url) return
    changeInput(url, 'https://first.example')
    await flush()
    await clickButton('save')
    await flush()
    expect(button('saving').disabled).toBe(true)
    changeInput(url, 'https://second.example')
    await act(async () => pending.resolve(undefined))
    await flush()
    expect(
      view.querySelector<HTMLInputElement>('input[inputmode="url"]')?.value,
    ).toBe('https://second.example')
    expect(view.textContent).toContain('unsaved changes')
  })

  it('isolates an old A save after switching A→B→A', async () => {
    configure({
      providerIds: ['openai-codex', 'openrouter'],
      value: {
        providers: {
          'openai-codex': { api_url: 'https://a.example', max_tokens: 3 },
          openrouter: { api_url: 'https://b.example', max_tokens: 4 },
        },
      },
    })
    const pending = deferred<unknown>()
    transport.pendingSet = pending
    const view = mount('openai-codex')
    await flush()
    const input = view.querySelector<HTMLInputElement>('input[inputmode="url"]')
    expect(input).not.toBeNull()
    if (!input) return
    changeInput(input, 'https://old-a.example')
    await flush()
    await clickButton('save')
    await flush()
    renderProvider('openrouter')
    await flush()
    expect(
      view.querySelector<HTMLInputElement>('input[inputmode="url"]')?.value,
    ).toBe('https://b.example')
    expect(view.textContent).not.toContain('https://old-a.example')
    await flush()
    renderProvider('openai-codex')
    await flush()
    const active = view.querySelector<HTMLInputElement>(
      'input[inputmode="url"]',
    )
    expect(active?.value).toBe('https://old-a.example')
    if (!active) return
    changeInput(active, 'https://new-a.example')
    await act(async () => pending.resolve(undefined))
    await flush()
    expect(
      view.querySelector<HTMLInputElement>('input[inputmode="url"]')?.value,
    ).toBe('https://new-a.example')
  })

  it('maps an absolute provider pointer to the visible field and keeps the generic error', async () => {
    const view = mount()
    await flush()
    await waitForText('Provider settings')
    const url = view.querySelector<HTMLInputElement>('input[inputmode="url"]')
    expect(url).not.toBeNull()
    if (!url) return
    changeInput(url, 'https://invalid.example')
    transport.setError = new Error(
      'invalid endpoint at /providers/openai-codex/api_url',
    )
    await flush()
    await clickButton('save')
    await flush()
    const alerts = [...view.querySelectorAll('[role="alert"]')].map(
      (node) => node.textContent,
    )
    expect(alerts.some((text) => text?.includes('invalid endpoint'))).toBe(true)
    expect(view.textContent).toContain('invalid endpoint')
  })

  it('renders an opaque value as not editable and supports reset', async () => {
    configure({ value: 'opaque' })
    const view = mount()
    await waitForText('not editable')
    expect(view.textContent).toContain('not editable')
    expect(view.querySelector('input')).toBeNull()

    configure({
      value: {
        providers: {
          'openai-codex': { api_url: 'https://server.example', max_tokens: 3 },
        },
      },
    })
    await act(async () => {
      await queryClient.invalidateQueries({
        queryKey: configurationKeys.rawValue('team-blue-router'),
      })
    })
    await waitForText('Provider settings')
    const url = view.querySelector<HTMLInputElement>('input[inputmode="url"]')
    expect(url).not.toBeNull()
    if (!url) return
    changeInput(url, 'https://draft.example')
    await flush()
    await clickButton('reset')
    await flush()
    expect(
      view.querySelector<HTMLInputElement>('input[inputmode="url"]')?.value,
    ).toBe('https://server.example')
  })

  it('R7(a): passes real override props and relative provider errors', async () => {
    removeOverride = registerExtProviderConfigForm({
      providerId: 'openai-codex',
      component: TestProviderOverride,
      scope: 'r7-test',
      path: 'ProviderConfigurationPanel.test.tsx',
    })
    const view = mount()
    await waitForText('edit override')
    expect(observedOverrideProps).toEqual(
      expect.objectContaining({
        providerId: 'openai-codex',
        value: { api_url: 'https://old.example', max_tokens: 3 },
        configured: undefined,
        available: undefined,
        modelCount: 0,
      }),
    )
    expect(observedOverrideProps?.schema).toEqual(
      expect.objectContaining({
        type: 'object',
        properties: expect.anything(),
      }),
    )
    expect(typeof observedOverrideProps?.onChange).toBe('function')
    expect(observedOverrideProps?.errors).toBeInstanceOf(Map)

    transport.setError = new Error(
      'invalid endpoint at /providers/openai-codex/api_url',
    )
    await clickButton('edit override')
    await clickButton('save')
    expect(view.querySelector('[role="alert"]')?.textContent).toBe(
      'invalid endpoint',
    )
    expect(observedOverrideProps?.errors?.get('/api_url')).toBe(
      'invalid endpoint',
    )
  })

  it('R7(b): treats null schema as terminal and Retry recovers the form', async () => {
    const response = transport.schemas.get('team-blue-router')
    expect(response).toBeTruthy()
    if (!response) return
    transport.schemas.set('team-blue-router', { ...response, schema: null })
    const view = mount()
    await waitForText('Provider configuration schema unavailable')
    expect(view.querySelector('input')).toBeNull()
    expect(view.textContent).not.toContain('unsaved changes')

    transport.schemas.set('team-blue-router', {
      ...response,
      schema: schemaFor(),
    })
    await clickButton('Retry')
    await waitForText('Provider settings')
    expect(view.querySelector('input[inputmode="url"]')).not.toBeNull()
  })

  it('R7(c): blocks Save for max_tokens=0 and preserves numeric zero', async () => {
    const view = mount()
    await waitForText('Provider settings')
    const tokens = view.querySelector<HTMLInputElement>(
      'input[inputmode="numeric"]',
    )
    expect(tokens).not.toBeNull()
    if (!tokens) return
    changeInput(tokens, '0')
    await flush()
    expect(tokens.value).toBe('0')
    expect(button('save').disabled).toBe(true)
    expect(callsFor('configuration::set')).toHaveLength(0)
  })

  it('R7(d): calls refreshModels exactly after a successful save', async () => {
    const refreshModels = vi.fn(async () => undefined)
    conversationContext.current = {
      presentProviders: [],
      modelOptions: [],
      refreshModels,
    }
    const view = mount()
    await waitForText('Provider settings')
    const url = view.querySelector<HTMLInputElement>('input[inputmode="url"]')
    expect(url).not.toBeNull()
    if (!url) return
    changeInput(url, 'https://saved.example')
    await flush()
    await clickButton('save')
    expect(callsFor('configuration::set')).toHaveLength(1)
    expect(refreshModels).toHaveBeenCalledTimes(1)
  })

  it('R7(e): blocks Save for a valid dirty draft after a value error and restores it on Retry', async () => {
    const view = mount()
    await waitForText('Provider settings')
    const url = view.querySelector<HTMLInputElement>('input[inputmode="url"]')
    expect(url).not.toBeNull()
    if (!url) return
    changeInput(url, 'https://draft.example')
    await flush()
    expect(button('save').disabled).toBe(false)

    transport.valueErrors.set(
      'team-blue-router',
      new Error('value unavailable'),
    )
    await act(async () => {
      await queryClient.invalidateQueries({
        queryKey: configurationKeys.rawValue('team-blue-router'),
      })
    })
    await flush()
    expect(view.textContent).toContain('Could not load provider configuration')
    expect(button('save').disabled).toBe(true)
    await clickButton('save')
    expect(callsFor('configuration::set')).toHaveLength(0)

    transport.valueErrors.delete('team-blue-router')
    await clickButton('Retry')
    await waitForText('Provider settings')
    expect(
      view.querySelector<HTMLInputElement>('input[inputmode="url"]')?.value,
    ).toBe('https://draft.example')
    expect(button('save').disabled).toBe(false)
  })

  it('R7(f): renders exact field alert plus the SaveBar error', async () => {
    const view = mount()
    await waitForText('Provider settings')
    const url = view.querySelector<HTMLInputElement>('input[inputmode="url"]')
    expect(url).not.toBeNull()
    if (!url) return
    changeInput(url, 'https://invalid.example')
    transport.setError = new Error(
      'invalid endpoint at /providers/openai-codex/api_url',
    )
    await flush()
    await clickButton('save')
    const alerts = [...view.querySelectorAll('[role="alert"]')].map(
      (node) => node.textContent,
    )
    expect(alerts).toContain('invalid endpoint')
    expect(alerts).toContain('error: invalid endpoint')
  })
})
