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
  role: 'function-trigger',
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

  it('counts function-trigger input AND output JSON', () => {
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

  it('skips messages older than the most-recent compaction marker', () => {
    // After server-side compaction, the messages BEFORE the marker have
    // been summarised away on the server. Counting them would keep the
    // CTX bar pinned at pre-compaction levels and lie about reality.
    const compactionMarker: Message = {
      id: 'c',
      role: 'system',
      kind: 'compaction',
      content: 'compacted',
      summaryText: 'short summary',
      createdAt: 0,
    }
    const msgs: Message[] = [
      userMsg('this user msg is no longer in flat-state'),
      asstMsg('this assistant reply is also gone server-side'),
      compactionMarker,
      userMsg('post-compaction message'),
    ]
    const expected =
      Math.ceil('short summary'.length / 4) +
      Math.ceil('post-compaction message'.length / 4)
    expect(estimateConversationTokens(msgs)).toBe(expected)
  })

  it('honours only the LAST marker when several compactions have stacked', () => {
    const marker1: Message = {
      id: 'c1',
      role: 'system',
      kind: 'compaction',
      content: 'compacted',
      summaryText: 'first summary',
      createdAt: 0,
    }
    const marker2: Message = {
      id: 'c2',
      role: 'system',
      kind: 'compaction',
      content: 'compacted again',
      summaryText: 'second summary',
      createdAt: 0,
    }
    const msgs: Message[] = [
      userMsg('ancient'),
      marker1,
      userMsg('middle, also discarded after re-compact'),
      marker2,
      userMsg('current'),
    ]
    // Only marker2's summary + the trailing user message count.
    const expected =
      Math.ceil('second summary'.length / 4) + Math.ceil('current'.length / 4)
    expect(estimateConversationTokens(msgs)).toBe(expected)
  })

  it('counts every message when no compaction marker is present', () => {
    const msgs = [userMsg('a'.repeat(40)), asstMsg('b'.repeat(40))]
    expect(estimateConversationTokens(msgs)).toBe(Math.ceil(40 / 4) * 2)
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
