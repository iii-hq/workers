import type { FunctionTriggerMessage, Host } from '@iii-dev/console-ui'
import { act, type ButtonHTMLAttributes, type HTMLAttributes } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import setup from '../page'
import { createImageRenderer, GENERATE_ID, inlineImageUrl, parseImageDetails, READ_ID } from './image-renderer'

// The Console supplies these primitives through its runtime import map.
// Only that host boundary is substituted; renderer, hooks and formatters stay real.
vi.mock('@iii-dev/console-ui', () => ({
  IconButton: ({
    label,
    variant: _variant,
    ...props
  }: ButtonHTMLAttributes<HTMLButtonElement> & { label: string; variant?: string }) => (
    <button aria-label={label} {...props} />
  ),
  Skeleton: (props: HTMLAttributes<HTMLDivElement>) => <div {...props} />,
}))

const trigger = vi.fn<(id: string, payload: unknown, options?: unknown) => Promise<unknown>>()
const register = vi.fn()
const host = {
  iii: { trigger },
  functionTriggers: { register },
  providerConfigForms: { register: vi.fn() },
} as unknown as Host
const renderer = createImageRenderer(host)
const path = '/data/images/lighthouse.png'
const details = {
  path,
  model: 'gpt-image-2.5-flare',
  width: 1024,
  height: 1536,
  bytes: 2048,
  mime: 'image/png',
  revised_prompt: 'A lighthouse at dusk',
}
const preview = { content: [{ type: 'image', data: 'cHJldmlldw==', mime: 'image/jpeg' }] }
let root: Root
let container: HTMLDivElement

function message(overrides: Partial<FunctionTriggerMessage> = {}): FunctionTriggerMessage {
  return {
    id: 'image-1',
    role: 'function-trigger',
    functionId: GENERATE_ID,
    input: {},
    output: { details },
    createdAt: 0,
    ...overrides,
  }
}

async function render(overrides: Partial<FunctionTriggerMessage> = {}) {
  await act(async () => root.render(renderer.tryRender(message(overrides))))
}

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
  trigger.mockReset().mockResolvedValue(preview)
  register.mockReset()
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
})

describe('image chat renderer', () => {
  it('registers the renderer and claims only its image functions', () => {
    setup(host)
    expect(register).toHaveBeenCalledOnce()
    expect(register.mock.calls[0][0].isMatch(GENERATE_ID)).toBe(true)
    expect(renderer.isMatch(READ_ID)).toBe(true)
    expect(renderer.isMatch('other::image::generate')).toBe(false)
  })

  it('reads direct and harness-wrapped results without accepting errors', () => {
    for (const output of [details, { details }, { content: [], details: { content: [], details } }]) {
      expect(parseImageDetails(output)).toMatchObject({ path, model: details.model, width: 1024 })
    }
    for (const output of [null, [], {}, { error: 'failed', details }, { details: { path: '' } }]) {
      expect(parseImageDetails(output)).toBeNull()
    }
    expect(inlineImageUrl({ details: preview })).toBe('data:image/jpeg;base64,cHJldmlldw==')
  })

  it('fetches a preview and displays the image and saved file metadata', async () => {
    await render({ output: { content: [], details: { content: [], details } } })
    expect(trigger).toHaveBeenCalledWith(READ_ID, { path, variant: 'preview' }, { timeoutMs: 20_000 })
    expect(container.querySelector('img')?.alt).toBe('A lighthouse at dusk')
    expect(container.querySelector('img')?.src).toBe('data:image/jpeg;base64,cHJldmlldw==')
    expect(container.textContent).toContain('1024×1536')
    expect(container.textContent).toContain('lighthouse.png')
    expect(container.querySelector('button[aria-label="Copy path"]')).not.toBeNull()
  })

  it('uses an inline image without fetching it again', async () => {
    await render({ output: { ...preview, details } })
    expect(container.querySelector('img')).not.toBeNull()
    expect(trigger).not.toHaveBeenCalled()
  })

  it('shows pending generation without reading a file or bypassing approval', async () => {
    await act(async () =>
      root.render(
        renderer.tryRenderRunning?.(
          message({ running: true, input: { model: details.model, prompt: 'A lighthouse' } }),
        ),
      ),
    )
    expect(container.querySelector('[aria-busy="true"]')).not.toBeNull()
    expect(container.textContent).toContain('A lighthouse')
    expect(trigger).not.toHaveBeenCalled()
    expect(renderer.tryRenderRunning?.(message({ running: true, pendingApproval: true }))).toBeNull()
    expect(renderer.tryRender(message({ pendingApproval: true }))).toBeNull()
    expect(renderer.tryRender(message({ output: { error: 'denied' } }))).toBeNull()
  })

  it.each([
    ['unavailable', () => Promise.reject(new Error('Preview unavailable'))],
    ['empty', () => Promise.resolve({ content: [] })],
  ] as const)('shows a readable error for %s previews', async (_name, response) => {
    trigger.mockImplementation(response)
    await render()
    expect(container.querySelector('[role="status"]')?.textContent).toMatch(/unavailable|no preview/)
    expect(container.querySelector('img')).toBeNull()
    expect(container.textContent).toContain('lighthouse.png')
  })

  it('keeps the latest preview when an earlier file responds late', async () => {
    let resolveFirst!: (value: unknown) => void
    trigger.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveFirst = resolve
        }),
    )
    await render()
    expect(container.querySelector('[aria-label="Loading preview"]')).not.toBeNull()
    const next = { ...details, path: '/data/images/next.png' }
    await render({ output: { details: next } })
    await act(async () => resolveFirst({ content: [{ type: 'image', mime: 'image/png', data: 'b2xk' }] }))
    expect(container.querySelector('img')?.src).toBe('data:image/jpeg;base64,cHJldmlldw==')
    expect(container.textContent).toContain('next.png')
    expect(container.textContent).not.toContain('lighthouse.png')
  })
})
