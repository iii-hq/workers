import type { FunctionTriggerMessage } from '@iii-dev/console-ui'
import { describe, expect, it, vi } from 'vitest'
import { createSearchTriggerRenderer } from './search-card'

vi.mock('@iii-dev/console-ui', () => ({
  ActionLine: () => null,
  Badge: () => null,
  Card: () => null,
  Chip: () => null,
  EmptyState: () => null,
  Eyebrow: () => null,
  MetaRow: () => null,
  TerminalCommandLine: () => null,
}))

/** Run every real function component in the tree so child bodies execute;
 * mocked components return null and stay transparent. Each node keeps its
 * type name and props (minus children) for assertions. */
function expand(node: unknown): unknown {
  if (Array.isArray(node)) return node.map(expand)
  if (!node || typeof node !== 'object' || !('type' in node)) return node
  const { type, props } = node as { type: unknown; props: Record<string, unknown> }
  const { children, ...rest } = props
  const rendered = typeof type === 'function' ? (type as (p: Record<string, unknown>) => unknown)(props) : null
  return {
    type: typeof type === 'function' ? type.name : String(type),
    props: rest,
    children: expand(rendered ?? children),
  }
}

const output = {
  guidance: 'Choose the smallest candidate set.',
  workers: [
    {
      namespace: 'browser',
      functions: [
        {
          function_id: 'browser::fetch',
          description: 'Fetch a web page.',
        },
      ],
    },
  ],
  latency_ms: 12,
}

describe('search trigger renderer', () => {
  it('keeps the custom result hidden until the host card expands', () => {
    expect(createSearchTriggerRenderer().metadata?.display).not.toBe(true)
  })

  it('renders submitted capabilities in the expanded result', () => {
    const rendered = createSearchTriggerRenderer().tryRender({
      functionId: 'directory::search_functions',
      input: {
        query: 'legacy query must stay hidden',
        capabilities: [
          'fetch webpage content from g1.globo.com',
          'extract latest news headlines',
        ],
      },
      output,
    } as FunctionTriggerMessage) as {
      type: (props: Record<string, unknown>) => unknown
      props: Record<string, unknown>
    }

    const card = rendered.type(rendered.props)
    const serialized = JSON.stringify(card)
    expect(serialized).toContain('fetch webpage content from g1.globo.com')
    expect(serialized).toContain('extract latest news headlines')
    expect(serialized).not.toContain('legacy query must stay hidden')
  })

  it('renders installed skills with their skills::get call, even without functions', () => {
    const rendered = createSearchTriggerRenderer().tryRender({
      functionId: 'directory::search_functions',
      input: { capabilities: ['schedule recurring function execution with cron'] },
      output: {
        guidance: 'The `skills` entries are installed how-to documents…',
        workers: [],
        skills: [{ id: 'cron', title: 'cron', description: 'Schedule any registered function.' }],
        latency_ms: 4256,
      },
    } as FunctionTriggerMessage) as {
      type: (props: Record<string, unknown>) => unknown
      props: Record<string, unknown>
    }

    const serialized = JSON.stringify(expand(rendered.type(rendered.props)))
    expect(serialized).toContain('installed skills')
    expect(serialized).toContain('"type":"SkillBlock"')
    expect(serialized).toContain(JSON.stringify('directory::skills::get { "id": "cron" }'))
    expect(serialized).toContain('Schedule any registered function.')
    // title === id: no separate title span
    expect(serialized).not.toContain('dir-ui-search-fn-desc')
    expect(serialized).not.toContain('No functions matched')
  })

  it('renders registered triggers with their functions::info call, even without functions', () => {
    const rendered = createSearchTriggerRenderer().tryRender({
      functionId: 'directory::search_functions',
      input: { capabilities: ['run a job every night'] },
      output: {
        guidance: 'The `triggers` entries are registered trigger bindings…',
        workers: [],
        triggers: [
          {
            id: 't-1',
            trigger_type: 'cron',
            function_id: 'harness::sweep-pending',
            worker_name: 'harness',
            config: { expression: '0 0 0 * * *' },
          },
        ],
        latency_ms: 900,
      },
    } as FunctionTriggerMessage) as {
      type: (props: Record<string, unknown>) => unknown
      props: Record<string, unknown>
    }

    const serialized = JSON.stringify(expand(rendered.type(rendered.props)))
    expect(serialized).toContain('registered triggers')
    expect(serialized).toContain('"type":"TriggerBlock"')
    expect(serialized).toContain(JSON.stringify('engine::functions::info { "function_id": "harness::sweep-pending" }'))
    expect(serialized).toContain(JSON.stringify(JSON.stringify({ expression: '0 0 0 * * *' })))
    expect(serialized).toContain('· harness')
    expect(serialized).not.toContain('No functions matched')
  })

  it('omits the config line for a trigger bound without config', () => {
    const rendered = createSearchTriggerRenderer().tryRender({
      functionId: 'directory::search_functions',
      input: { capabilities: ['react when the configuration changes'] },
      output: {
        guidance: 'The `triggers` entries are registered trigger bindings…',
        workers: [],
        triggers: [{ id: 't-2', trigger_type: 'configuration', function_id: 'directory::on-config-change', config: {} }],
        latency_ms: 90,
      },
    } as FunctionTriggerMessage) as {
      type: (props: Record<string, unknown>) => unknown
      props: Record<string, unknown>
    }

    const serialized = JSON.stringify(expand(rendered.type(rendered.props)))
    expect(serialized).toContain('"type":"TriggerBlock"')
    expect(serialized).toContain(JSON.stringify('engine::functions::info { "function_id": "directory::on-config-change" }'))
    expect(serialized).not.toContain('dir-ui-search-desc"')
  })

  it('labels the card with the search mode when present', () => {
    const rendered = createSearchTriggerRenderer().tryRender({
      functionId: 'directory::search_functions',
      input: { capabilities: ['x'] },
      output: {
        guidance: 'g',
        workers: [{ namespace: 'browser', functions: [{ function_id: 'browser::fetch', description: 'Fetch.' }] }],
        search_mode: 'jev',
        latency_ms: 10,
      },
    } as FunctionTriggerMessage) as {
      type: (props: Record<string, unknown>) => unknown
      props: Record<string, unknown>
    }
    const serialized = JSON.stringify(rendered.type(rendered.props))
    expect(serialized).toContain('jev')
    expect(serialized).not.toContain('>search<')
  })
})
