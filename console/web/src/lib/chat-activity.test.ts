import { describe, expect, it } from 'vitest'
import { getDockSignal } from '@/lib/chat-activity'
import type { Conversation, Message } from '@/types/chat'

function conv(messages: Message[]): Conversation {
  return {
    id: 'test',
    title: 't',
    model: 'openai::gpt-5',
    mode: 'agent',
    messages,
    createdAt: 0,
    updatedAt: 0,
  }
}

describe('getDockSignal', () => {
  it('returns null for missing or empty conversation', () => {
    expect(getDockSignal(null)).toBeNull()
    expect(getDockSignal(undefined)).toBeNull()
    expect(getDockSignal(conv([]))).toBeNull()
  })

  it("returns 'active' when the assistant is streaming the last message", () => {
    const m: Message = {
      id: '1',
      createdAt: 0,
      role: 'assistant',
      content: 'partial',
      streaming: true,
    }
    expect(getDockSignal(conv([m]))).toBe('active')
  })

  it("returns 'active' when the thought stream is in flight", () => {
    const m: Message = {
      id: '1',
      createdAt: 0,
      role: 'thought',
      content: 'thinking',
      durationMs: 0,
      streaming: true,
    }
    expect(getDockSignal(conv([m]))).toBe('active')
  })

  it("returns 'active' when a tool call is running", () => {
    const m: Message = {
      id: '1',
      createdAt: 0,
      role: 'function-trigger',
      functionId: 'shell::run',
      input: {},
      running: true,
    }
    expect(getDockSignal(conv([m]))).toBe('active')
  })

  it("returns 'error' when the last message is a system/error", () => {
    const m: Message = {
      id: '1',
      createdAt: 0,
      role: 'system',
      content: 'rate limited',
      tone: 'error',
    }
    expect(getDockSignal(conv([m]))).toBe('error')
  })

  it("returns 'attention' when the last message is a system/warn", () => {
    const m: Message = {
      id: '1',
      createdAt: 0,
      role: 'system',
      content: 'soft failure',
      tone: 'warn',
    }
    expect(getDockSignal(conv([m]))).toBe('attention')
  })

  it('returns null for a settled assistant message (no streaming flag)', () => {
    const m: Message = {
      id: '1',
      createdAt: 0,
      role: 'assistant',
      content: 'done',
    }
    expect(getDockSignal(conv([m]))).toBeNull()
  })

  it('pendency wins over freshness: pending approval anywhere overrides a later system error', () => {
    const pending: Message = {
      id: '1',
      createdAt: 0,
      role: 'function-trigger',
      functionId: 'shell::write',
      input: {},
      pendingApproval: true,
    }
    const errLater: Message = {
      id: '2',
      createdAt: 1,
      role: 'system',
      content: 'context compacted',
      tone: 'error',
    }
    expect(getDockSignal(conv([pending, errLater]))).toBe('active')
  })
})
