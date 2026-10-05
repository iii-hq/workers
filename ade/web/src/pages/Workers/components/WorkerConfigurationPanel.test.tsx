// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ConfigFormProps } from '@/types/injectable-ui'
import { WorkerConfigurationPanel } from './WorkerConfigurationPanel'

const harness = vi.hoisted(() => ({
  schema: {
    data: undefined as unknown,
    error: null as Error | null,
    refetch: vi.fn(),
  },
  value: { model: 'jev-latest' } as unknown,
  mutate: vi.fn(),
}))

vi.mock('@/pages/Configuration/tabs/WorkersTab/hooks', () => ({
  useConfigurationSchema: () => harness.schema,
  useConfigurationValue: () => ({
    data: harness.value,
    isLoading: false,
    isError: false,
  }),
  useSetConfiguration: () => ({ mutate: harness.mutate }),
}))

/** The worker's own form, as its UI script registers it. */
function ModelForm({ value, onChange }: ConfigFormProps) {
  const model = (value as { model?: string } | null)?.model ?? ''
  return (
    <input
      aria-label="model"
      value={model}
      onChange={(event) => onChange({ model: event.target.value })}
    />
  )
}

vi.mock('@/lib/ui-slots', () => ({
  useExtConfigForm: (id: string) =>
    id === 'judge-typesafe'
      ? { configurationId: id, component: ModelForm, layout: 'contained' }
      : undefined,
  useUiAssetsStatus: () => 'ready',
  isExtConfigFormPending: () => false,
}))

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  harness.schema = { data: undefined, error: null, refetch: vi.fn() }
  harness.value = { model: 'jev-latest' }
  harness.mutate.mockReset()
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
})

const ENTRY = {
  id: 'judge-typesafe',
  name: 'judge-typesafe',
  description: 'TypeSafe judge',
  schema: { type: 'object' },
}

async function render(
  props: Partial<Parameters<typeof WorkerConfigurationPanel>[0]> = {},
) {
  await act(async () => {
    root.render(
      <WorkerConfigurationPanel configurationId="judge-typesafe" {...props} />,
    )
  })
}

function setInput(input: HTMLInputElement, text: string) {
  const setValue = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'value',
  )?.set
  setValue?.call(input, text)
  input.dispatchEvent(new Event('input', { bubbles: true }))
}

describe('WorkerConfigurationPanel', () => {
  it("renders the worker's form without the Settings header and saves through the host", async () => {
    harness.schema.data = ENTRY
    const onDirtyChange = vi.fn()
    const onSaved = vi.fn()
    harness.mutate.mockImplementation((_payload, options) =>
      options.onSuccess({ new_value: { model: 'jev-2' }, old_value: null }),
    )
    await render({ onDirtyChange, onSaved })
    // The surface around it titles the page: no Settings header here.
    expect(container.querySelector('header')).toBeNull()
    const input = container.querySelector<HTMLInputElement>(
      'input[aria-label="model"]',
    )
    expect(input?.value).toBe('jev-latest')
    await act(async () => setInput(input as HTMLInputElement, 'jev-2'))
    expect(onDirtyChange).toHaveBeenLastCalledWith(true)
    const save = [...container.querySelectorAll('button')].find(
      (button) => button.textContent === 'save',
    )
    await act(async () => save?.click())
    expect(harness.mutate.mock.calls[0]?.[0]).toEqual({
      id: 'judge-typesafe',
      value: { model: 'jev-2' },
    })
    expect(onSaved).toHaveBeenCalledWith({ model: 'jev-2' })
  })

  it('shows a skeleton until the entry loads, and a retry when it cannot', async () => {
    await render()
    expect(
      container.querySelector('[aria-label="Loading settings"]'),
    ).not.toBeNull()
    harness.schema.error = new Error('configuration worker is down')
    await render()
    expect(container.textContent).toContain('configuration worker is down')
    const retry = [...container.querySelectorAll('button')].find((button) =>
      button.textContent?.includes('Retry'),
    )
    await act(async () => retry?.click())
    expect(harness.schema.refetch).toHaveBeenCalled()
  })

  it('renders nothing without a configuration id', async () => {
    await render({ configurationId: null })
    expect(container.innerHTML).toBe('')
  })
})
