// @vitest-environment jsdom

import type { FunctionCallMessage, Host } from '@iii-dev/console-ui'
import type { HTMLAttributes, ReactNode } from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createWorkflowRenderer, registerWorkflowRenderer } from './index'

// The Console supplies these through its import map; the package throws when
// loaded directly, so only that boundary is substituted.
vi.mock('@iii-dev/console-ui', () => {
  const Span = ({ children, ...props }: HTMLAttributes<HTMLSpanElement>) => <span {...props}>{children}</span>
  return {
    Badge: Span,
    Chip: Span,
    Eyebrow: Span,
    MetaRow: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
    ActionLine: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
  }
})

const renderer = createWorkflowRenderer()
let root: Root
let container: HTMLDivElement

function message(overrides: Partial<FunctionCallMessage>): FunctionCallMessage {
  return {
    id: 'call-1',
    role: 'function-call',
    functionId: 'workflow::status',
    input: {},
    createdAt: 0,
    ...overrides,
  }
}

async function render(overrides: Partial<FunctionCallMessage>) {
  const node = renderer.tryRender(message(overrides))
  await act(async () => root.render(node))
  return node
}

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  act(() => root.unmount())
  container.remove()
})

describe('workflow cards', () => {
  it('start draws the DAG and the run id', async () => {
    await render({
      functionId: 'workflow::start',
      input: {
        nodes: { plan: { task: 'plan it', model: 'opus' }, write: { task: 'write it', reads: 'plan' } },
        output: 'write',
      },
      output: { run_id: 'run-42' },
    })
    const text = container.textContent ?? ''
    expect(text).toContain('launched')
    expect(text).toContain('run-42')
    expect(text).toContain('2 nodes')
    expect(text).toContain('← plan')
    expect(text).toContain('Output')
    expect(text).toContain('opus')
  })

  it('status orders fanned uids numerically and shows failures', async () => {
    await render({
      functionId: 'workflow::status',
      input: { run_id: 'run-42' },
      output: {
        run_id: 'run-42',
        status: 'failed',
        nodes: { 'read#10': 'done', 'read#2': 'failed', plan: 'done' },
        node_errors: { 'read#2': 'timed out' },
      },
    })
    const uids = [...container.querySelectorAll('.wf-node-id')].map((el) => el.textContent)
    expect(uids).toEqual(['plan', 'read#2', 'read#10'])
    expect(container.textContent).toContain('timed out')
    expect(container.textContent).toContain('2/3 done')
    expect(container.querySelector('[data-node-state="failed"]')).not.toBeNull()
  })

  it('node-result shows the stored result', async () => {
    await render({
      functionId: 'workflow::node-result',
      input: { run_id: 'run-42', node_uid: 'write' },
      output: { result: 'the essay' },
    })
    expect(container.querySelector('.wf-pre')?.textContent).toBe('the essay')
    expect(container.textContent).toContain('write')
  })

  it('stop shows the resulting status', async () => {
    await render({
      functionId: 'workflow::stop',
      input: { run_id: 'run-42' },
      output: { run_id: 'run-42', status: 'cancelled' },
    })
    expect(container.textContent).toContain('cancelled')
    expect(container.textContent).toContain('run-42')
  })

  it('leaves unparsable requests to the default card', () => {
    for (const functionId of ['workflow::start', 'workflow::node-result', 'workflow::stop']) {
      expect(renderer.tryRender(message({ functionId, input: { bogus: 1 } }))).toBeNull()
      expect(renderer.tryRender(message({ functionId, input: { bogus: 1 }, running: true }))).toBeNull()
    }
  })

  it('leaves errors, pending approvals and lifecycle ids to the console', () => {
    expect(renderer.tryRender(message({ input: { run_id: 'r' }, output: { error: 'boom' } }))).toBeNull()
    expect(renderer.tryRender(message({ input: { run_id: 'r' }, pendingApproval: true }))).toBeNull()
    expect(renderer.tryRender(message({ functionId: 'workflow::tick' }))).toBeNull()
    expect(renderer.isMatch('workflow::tick')).toBe(true)
    expect(renderer.isMatch('state::get')).toBe(false)
  })

  it('registers one renderer', () => {
    const remove = vi.fn()
    const register = vi.fn(() => remove)
    const host = { functionCalls: { register } } as unknown as Host
    expect(registerWorkflowRenderer(host)).toBe(remove)
    expect(register).toHaveBeenCalledTimes(1)
  })
})
