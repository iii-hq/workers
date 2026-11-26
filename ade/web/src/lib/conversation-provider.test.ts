import { describe, expect, it } from 'vitest'
import type { AssistantMessage, Message } from '@/types/chat'
import { effectiveModel, effectiveProvider } from './conversation-provider'

const CATALOG = [
  { id: 'anthropic::claude-opus-4-5' },
  { id: 'claude-code::claude-opus-4-5' },
  { id: 'openai::gpt-5.4' },
]

function reply(fields: Partial<AssistantMessage>): AssistantMessage {
  return {
    id: 'a1',
    role: 'assistant',
    content: 'Done',
    createdAt: 1,
    ...fields,
  }
}

const user: Message = { id: 'u1', role: 'user', content: 'Go', createdAt: 0 }

describe("a conversation's effective model and provider", () => {
  it('reads its own model first', () => {
    const c = {
      model: 'claude-code::claude-opus-4-5',
      messages: [reply({ model: 'gpt-5.4', provider: 'openai' })],
    }
    expect(effectiveModel(c, CATALOG)).toBe('claude-code::claude-opus-4-5')
    expect(effectiveProvider(c, { catalog: CATALOG })).toBe('claude-code')
  })

  // Turns another client sent through harness::send, sub-agents: no
  // metadata.model, a transcript reply naming its provider.
  it("falls back to the last reply's provider, which picks the catalog key", () => {
    const c = {
      model: null,
      messages: [
        user,
        reply({ model: 'claude-opus-4-5', provider: 'claude-code' }),
      ],
    }
    expect(effectiveProvider(c)).toBe('claude-code')
    expect(effectiveProvider(c, { catalog: CATALOG })).toBe('claude-code')
    // Not the first catalog key that ends in the model id (anthropic's).
    expect(effectiveModel(c, CATALOG)).toBe('claude-code::claude-opus-4-5')
    // A provider the catalog lacks keeps the plain id.
    expect(effectiveModel(c)).toBe('claude-opus-4-5')
  })

  it('reads a streamed reply’s composite model', () => {
    const c = {
      messages: [user, reply({ model: 'claude-code::claude-opus-4-5' })],
    }
    expect(effectiveProvider(c)).toBe('claude-code')
  })

  it('never guesses for an imported conversation, and falls back to a locked model', () => {
    const imported = {
      sessionMetadata: { external_source: 'claude-code' },
      messages: [reply({ model: 'claude-opus-4-5', provider: 'claude-code' })],
    }
    expect(effectiveModel(imported, CATALOG)).toBeNull()
    expect(effectiveProvider(imported)).toBeNull()
    expect(
      effectiveProvider(
        { messages: [] },
        { lockedModel: 'claude-code::claude-opus-4-5' },
      ),
    ).toBe('claude-code')
    expect(effectiveProvider({ messages: [user] })).toBeNull()
    // A plain id the catalog doesn't know names no provider.
    expect(effectiveProvider({ model: 'claude-opus-5-5' })).toBeNull()
  })
})
