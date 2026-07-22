import { beforeEach, describe, expect, it, vi } from 'vitest'

const mockTrigger = vi.hoisted(() => vi.fn())
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: () => Promise.resolve({ trigger: mockTrigger }),
}))

import type {
  AssistantMessage,
  Conversation,
  FunctionCallMessage,
  Message,
  SystemMessage,
  ThoughtMessage,
  UserMessage,
} from '@/types/chat'
import {
  buildExportFilename,
  conversationToMarkdown,
  fetchWorkerVersions,
  messagesToMarkdown,
  formatTimestamp,
} from './export-session'

function baseConversation(messages: Message[] = []): Conversation {
  return {
    id: 'conv-12345678-abcd',
    title: 'Test session',
    model: 'openai::gpt-5',
    mode: 'agent',
    messages,
    createdAt: Date.UTC(2025, 0, 1, 12, 0, 0),
    updatedAt: Date.UTC(2025, 0, 1, 12, 30, 0),
  }
}

describe('conversationToMarkdown', () => {
  it('renders a header for an empty conversation with no-messages marker', () => {
    const out = conversationToMarkdown(baseConversation())
    expect(out).toMatch(/^# Session: Test session/)
    expect(out).toContain('- ID: `conv-12345678-abcd`')
    expect(out).toContain('- Model: `openai::gpt-5`')
    expect(out).toContain('- Mode: `agent`')
    expect(out).toContain('- Message count: 0')
    expect(out).toContain('_(no messages)_')
  })

  it('renders one of each message type with correct headings', () => {
    const user: UserMessage = {
      id: 'u1',
      role: 'user',
      content: 'hello',
      createdAt: 1,
    }
    const assistant: AssistantMessage = {
      id: 'a1',
      role: 'assistant',
      content: 'world',
      model: 'openai::gpt-5',
      mode: 'agent',
      createdAt: 2,
    }
    const thought: ThoughtMessage = {
      id: 't1',
      role: 'thought',
      content: 'thinking…',
      durationMs: 100,
      createdAt: 3,
    }
    const fcallNoOutput: FunctionCallMessage = {
      id: 'f1',
      role: 'function-call',
      functionId: 'search',
      input: { query: 'foo' },
      createdAt: 4,
    }
    const fcallWithOutput: FunctionCallMessage = {
      id: 'f2',
      role: 'function-call',
      functionId: 'search',
      input: { query: 'bar' },
      output: { hits: 3 },
      createdAt: 5,
    }
    const sys: SystemMessage = {
      id: 's1',
      role: 'system',
      content: 'compacted',
      tone: 'info',
      kind: 'compaction',
      createdAt: 6,
    }

    const out = conversationToMarkdown(
      baseConversation([
        user,
        assistant,
        thought,
        fcallNoOutput,
        fcallWithOutput,
        sys,
      ]),
    )

    expect(out).toContain('## User\nhello')
    expect(out).toContain('## Assistant (openai::gpt-5, agent)\nworld')
    expect(out).toContain('## Thought\nthinking…')
    expect(out).toContain('## Trigger — search')
    expect(out).toContain('"query": "foo"')
    expect(out).toContain('"query": "bar"')
    expect(out).toContain('"hits": 3')
    expect(out).toContain('## System — info (compaction)\ncompacted')
  })

  it('omits the Output block when output is undefined', () => {
    const fcall: FunctionCallMessage = {
      id: 'f1',
      role: 'function-call',
      functionId: 'search',
      input: { q: 'x' },
      createdAt: 1,
    }
    const out = conversationToMarkdown(baseConversation([fcall]))
    expect(out).toContain('**Input:**')
    expect(out).not.toContain('**Output:**')
  })

  it('lists attachments by metadata without leaking dataUrl base64', () => {
    const user: UserMessage = {
      id: 'u1',
      role: 'user',
      content: 'see image',
      createdAt: 1,
      attachments: [
        {
          id: 'att1',
          name: 'screenshot.png',
          size: 2048,
          type: 'image/png',
          dataUrl: 'data:image/png;base64,AAAABBBBCCCCDDDD',
        },
      ],
    }
    const out = conversationToMarkdown(baseConversation([user]))
    expect(out).toContain(
      '**Attachments:** `screenshot.png` (image/png, 2.0 KB)',
    )
    expect(out).not.toContain('AAAABBBBCCCCDDDD')
    expect(out).not.toContain('data:image/png')
  })

  it('falls back to String(value) for non-serialisable tool input', () => {
    const circular: { self?: unknown } = {}
    circular.self = circular
    const fcall: FunctionCallMessage = {
      id: 'f1',
      role: 'function-call',
      functionId: 'broken',
      input: circular,
      createdAt: 1,
    }
    const out = conversationToMarkdown(baseConversation([fcall]))
    expect(out).toContain('## Trigger — broken')
    // Either '[object Object]' from the toString fallback or the markdown
    // simply didn't throw — both signal graceful handling.
    expect(out).toContain('[object Object]')
  })

  it('preserves message order', () => {
    const m1: UserMessage = {
      id: 'u1',
      role: 'user',
      content: 'first',
      createdAt: 1,
    }
    const m2: AssistantMessage = {
      id: 'a1',
      role: 'assistant',
      content: 'second',
      createdAt: 2,
    }
    const m3: UserMessage = {
      id: 'u2',
      role: 'user',
      content: 'third',
      createdAt: 3,
    }
    const out = conversationToMarkdown(baseConversation([m1, m2, m3]))
    const firstIdx = out.indexOf('first')
    const secondIdx = out.indexOf('second')
    const thirdIdx = out.indexOf('third')
    expect(firstIdx).toBeGreaterThan(-1)
    expect(secondIdx).toBeGreaterThan(firstIdx)
    expect(thirdIdx).toBeGreaterThan(secondIdx)
  })

  it('annotates a pending-approval trigger in the heading', () => {
    const fcall: FunctionCallMessage = {
      id: 'f1',
      role: 'function-call',
      functionId: 'delete_file',
      input: { path: '/tmp/x' },
      pendingApproval: true,
      createdAt: 1,
    }
    const out = conversationToMarkdown(baseConversation([fcall]))
    expect(out).toContain('## Trigger — delete_file (pending approval)')
  })
})

describe('conversationToMarkdown — workers block', () => {
  it('renders sorted name: version bullets after the message count', () => {
    const out = conversationToMarkdown(baseConversation(), [
      { name: 'llm-router', version: '1.3.3' },
      { name: 'console', version: '1.7.2' },
      { name: 'harness', version: '1.5.2' },
    ])
    const idx = out.indexOf('- Workers:')
    expect(idx).toBeGreaterThan(out.indexOf('- Message count: 0'))
    expect(out).toContain(
      '- Workers:\n  - console: 1.7.2\n  - harness: 1.5.2\n  - llm-router: 1.3.3',
    )
  })

  it('collapses exact name: version duplicates', () => {
    const out = conversationToMarkdown(baseConversation(), [
      { name: 'harness', version: '1.5.2' },
      { name: 'harness', version: '1.5.2' },
      { name: 'harness', version: '1.5.1' },
    ])
    expect(out).toContain('- Workers:\n  - harness: 1.5.1\n  - harness: 1.5.2')
    expect(out.match(/ {2}- harness: 1\.5\.2/g)).toHaveLength(1)
  })

  it('renders (no version) when a worker has no version', () => {
    const out = conversationToMarkdown(baseConversation(), [
      { name: 'scrapling' },
    ])
    expect(out).toContain('- Workers:\n  - scrapling: (no version)')
  })

  it('renders _(unavailable)_ when workers is null', () => {
    const out = conversationToMarkdown(baseConversation(), null)
    expect(out).toContain('- Workers: _(unavailable)_')
  })

  it('renders _(none connected)_ when workers is an empty array', () => {
    const out = conversationToMarkdown(baseConversation(), [])
    expect(out).toContain('- Workers: _(none connected)_')
  })

  it('omits the Workers line entirely when workers is undefined', () => {
    const out = conversationToMarkdown(baseConversation())
    expect(out).not.toContain('- Workers')
  })
})

describe('buildExportFilename', () => {
  it('uses the first 8 chars of the conversation id', () => {
    const filename = buildExportFilename(baseConversation())
    expect(filename).toMatch(/^iii-session-conv-123-\d{8}-\d{4}\.md$/)
  })

  it('falls back to `session` when the id is empty', () => {
    const conv = { ...baseConversation(), id: '' }
    const filename = buildExportFilename(conv)
    expect(filename).toMatch(/^iii-session-session-\d{8}-\d{4}\.md$/)
  })
})

describe('fetchWorkerVersions', () => {
  beforeEach(() => {
    mockTrigger.mockReset()
  })

  it('calls engine::workers::list with a 2s timeout and maps name/version', async () => {
    mockTrigger.mockResolvedValue({
      workers: [
        { id: 'w1', name: 'harness', version: '1.5.2', status: 'connected' },
        { id: 'w2', name: null, version: '0.2.4', status: 'connected' },
        { id: 'w3', name: 'scrapling', version: null, status: 'connected' },
      ],
    })
    const out = await fetchWorkerVersions()
    expect(mockTrigger).toHaveBeenCalledWith(
      'engine::workers::list',
      {},
      { timeoutMs: 2000 },
    )
    expect(out).toEqual([
      { name: 'harness', version: '1.5.2' },
      { name: 'w2', version: '0.2.4' },
      { name: 'scrapling', version: undefined },
    ])
  })

  it('returns null when the trigger rejects (timeout or error)', async () => {
    mockTrigger.mockRejectedValue(new Error('timeout'))
    await expect(fetchWorkerVersions()).resolves.toBeNull()
  })

  it('returns null when the response has no workers array', async () => {
    mockTrigger.mockResolvedValue({ nope: true })
    await expect(fetchWorkerVersions()).resolves.toBeNull()
  })

  it('skips malformed entries but keeps valid ones', async () => {
    mockTrigger.mockResolvedValue({
      workers: [null, 42, { version: '9.9.9' }, { id: 'ok', version: '1.0.0' }],
    })
    await expect(fetchWorkerVersions()).resolves.toEqual([
      { name: 'ok', version: '1.0.0' },
    ])
  })
})


describe('buildExportFilename with suffix', () => {
  it('inserts the suffix between id and timestamp', () => {
    const filename = buildExportFilename(baseConversation(), 'full')
    expect(filename).toMatch(/^iii-session-conv-123-full-\d{8}-\d{4}\.md$/)
  })
})

describe('conversationToMarkdown — sub-agents bullet', () => {
  it('renders the count after the workers block', () => {
    const out = conversationToMarkdown(baseConversation(), [], 3)
    const workersIdx = out.indexOf('- Workers:')
    const subIdx = out.indexOf('- Sub-agents: 3')
    expect(workersIdx).toBeGreaterThan(-1)
    expect(subIdx).toBeGreaterThan(workersIdx)
  })

  it('renders _(unavailable)_ when discovery failed (null)', () => {
    const out = conversationToMarkdown(baseConversation(), [], null)
    expect(out).toContain('- Sub-agents: _(unavailable)_')
  })

  it('omits the bullet entirely when undefined', () => {
    const out = conversationToMarkdown(baseConversation(), [])
    expect(out).not.toContain('- Sub-agents')
  })
})

describe('messagesToMarkdown', () => {
  it('renders messages identically to the conversation body', () => {
    const user: UserMessage = {
      id: 'u1',
      role: 'user',
      content: 'hello',
      createdAt: 1,
    }
    expect(messagesToMarkdown([user])).toBe('## User\nhello')
  })
})

describe('formatTimestamp', () => {
  it('renders ISO timestamps', () => {
    expect(formatTimestamp(Date.UTC(2025, 0, 1, 12, 0, 0))).toBe(
      '2025-01-01T12:00:00.000Z',
    )
  })
})