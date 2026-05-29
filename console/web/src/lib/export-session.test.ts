import { describe, expect, it } from 'vitest'
import type {
  AssistantMessage,
  Conversation,
  FunctionCallMessage,
  Message,
  SystemMessage,
  ThoughtMessage,
  UserMessage,
} from '@/types/chat'
import { buildExportFilename, conversationToMarkdown } from './export-session'

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
    expect(out).toContain('## Tool call — search')
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
    expect(out).toContain('## Tool call — broken')
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

  it('annotates a pending-approval tool call in the heading', () => {
    const fcall: FunctionCallMessage = {
      id: 'f1',
      role: 'function-call',
      functionId: 'delete_file',
      input: { path: '/tmp/x' },
      pendingApproval: true,
      createdAt: 1,
    }
    const out = conversationToMarkdown(baseConversation([fcall]))
    expect(out).toContain('## Tool call — delete_file (pending approval)')
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
