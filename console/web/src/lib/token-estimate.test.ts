import { describe, expect, it } from 'vitest'
import type { Message } from '@/types/chat'
import {
  estimateConversationTokens,
  estimateMessageTokens,
  formatTokenCount,
} from './token-estimate'

const userMsg = (content: string): Message => ({
  id: 'u',
  role: 'user',
  content,
  createdAt: 0,
})

const asstMsg = (content: string): Message => ({
  id: 'a',
  role: 'assistant',
  content,
  createdAt: 0,
})

const sysMsg = (content: string): Message => ({
  id: 's',
  role: 'system',
  content,
  createdAt: 0,
})

const fcallMsg = (input: unknown, output: unknown): Message => ({
  id: 'f',
  role: 'function-call',
  functionId: 'shell::run',
  input,
  output,
  createdAt: 0,
})

describe('estimateMessageTokens', () => {
  it('returns zero for empty strings', () => {
    expect(estimateMessageTokens(userMsg(''))).toBe(0)
  })

  it('scales chars/4 (ceiling) for user text', () => {
    expect(estimateMessageTokens(userMsg('abcd'))).toBe(1)
    expect(estimateMessageTokens(userMsg('abcde'))).toBe(2)
  })

  it('skips system notices entirely', () => {
    expect(estimateMessageTokens(sysMsg('compacted — 1k tokens'))).toBe(0)
  })

  it('counts compaction-marker summary text (it ships next turn)', () => {
    const marker: Message = {
      id: 'c',
      role: 'system',
      kind: 'compaction',
      content: 'compacted',
      tone: 'info',
      summaryText: 'a'.repeat(400),
      tokensBefore: 12_345,
      createdAt: 0,
    }
    // 400 chars / 4 = 100 tokens
    expect(estimateMessageTokens(marker)).toBe(100)
  })

  it('returns 0 for compaction-marker with no summaryText', () => {
    const marker: Message = {
      id: 'c',
      role: 'system',
      kind: 'compaction',
      content: 'compacted',
      tone: 'info',
      createdAt: 0,
    }
    expect(estimateMessageTokens(marker)).toBe(0)
  })

  it('counts function-call input AND output JSON', () => {
    const fc = fcallMsg({ command: 'ls' }, { stdout: 'a\nb\nc' })
    // input ≈ 18 chars, output ≈ 18 chars → ~9 tokens
    expect(estimateMessageTokens(fc)).toBeGreaterThan(5)
  })
})

describe('estimateConversationTokens', () => {
  it('sums every contributing message', () => {
    const msgs = [
      userMsg('hello world'),
      asstMsg('hi there'),
      sysMsg('this is chrome, skipped'),
    ]
    const expected = Math.ceil(11 / 4) + Math.ceil(8 / 4)
    expect(estimateConversationTokens(msgs)).toBe(expected)
  })

  it('returns 0 for an empty conversation', () => {
    expect(estimateConversationTokens([])).toBe(0)
  })
})

describe('formatTokenCount', () => {
  it('returns plain integers under 1k', () => {
    expect(formatTokenCount(0)).toBe('0')
    expect(formatTokenCount(42)).toBe('42')
    expect(formatTokenCount(999)).toBe('999')
  })

  it('formats thousands as k', () => {
    expect(formatTokenCount(1_000)).toBe('1.0k')
    expect(formatTokenCount(1_234)).toBe('1.2k')
    expect(formatTokenCount(12_345)).toBe('12k')
    expect(formatTokenCount(999_999)).toBe('1000k')
  })

  it('formats millions as M', () => {
    expect(formatTokenCount(1_000_000)).toBe('1.0M')
    expect(formatTokenCount(1_500_000)).toBe('1.5M')
    expect(formatTokenCount(10_000_000)).toBe('10M')
  })
})
