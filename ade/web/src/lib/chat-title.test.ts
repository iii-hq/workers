import { describe, expect, it, vi } from 'vitest'
import { cleanChatTitle, summarizeChatTitle } from './chat-title'

const answer = (text: string) => ({
  message: { role: 'assistant', content: [{ type: 'text', text }] },
})

describe('cleanChatTitle', () => {
  it('keeps a short heading as written', () => {
    expect(cleanChatTitle('Validate streaming in KAN-12')).toBe(
      'Validate streaming in KAN-12',
    )
  })

  it('strips quotes, a Title: prefix, emphasis and a closing period', () => {
    expect(cleanChatTitle('"Fix the sidebar rail."')).toBe(
      'Fix the sidebar rail',
    )
    expect(cleanChatTitle('Title: Check out the workers branch')).toBe(
      'Check out the workers branch',
    )
    expect(cleanChatTitle('**Redesign browser settings**')).toBe(
      'Redesign browser settings',
    )
    expect(cleanChatTitle('# Engine indicator\n\nmore')).toBe(
      'Engine indicator',
    )
  })

  it('rejects a reply that is not a heading', () => {
    expect(cleanChatTitle('')).toBeNull()
    expect(cleanChatTitle('   \n ')).toBeNull()
    expect(
      cleanChatTitle(
        'Sure, I can help you check out this branch in workers and then run the tests',
      ),
    ).toBeNull()
  })
})

describe('summarizeChatTitle', () => {
  it("asks the chat's own model through router::complete", async () => {
    const call = vi.fn().mockResolvedValue(answer('Check out the branch'))
    const title = await summarizeChatTitle(
      { call },
      {
        sessionId: 's_1',
        prompt: 'Checkout this branch in workers, then build and run it',
        model: 'claude-sonnet-5-5',
      },
    )
    expect(title).toBe('Check out the branch')
    expect(call).toHaveBeenCalledTimes(1)
    const [fn, payload, options] = call.mock.calls[0]
    expect(fn).toBe('router::complete')
    expect(payload).toMatchObject({
      model: 'claude-sonnet-5-5',
      provider: 'anthropic',
      messages: [
        {
          role: 'user',
          content: [
            {
              type: 'text',
              text: 'Checkout this branch in workers, then build and run it',
            },
          ],
        },
      ],
    })
    expect(options).toEqual({ timeoutMs: 30_000 })
  })

  it('resolves null without a model, on a failure, or on a non-heading', async () => {
    const call = vi.fn()
    expect(
      await summarizeChatTitle(
        { call },
        { sessionId: 's', prompt: 'x', model: null },
      ),
    ).toBeNull()
    expect(call).not.toHaveBeenCalled()

    call.mockRejectedValueOnce(new Error('router::complete not registered'))
    expect(
      await summarizeChatTitle(
        { call },
        { sessionId: 's', prompt: 'x', model: 'gpt-5' },
      ),
    ).toBeNull()

    call.mockResolvedValueOnce({ message: { content: [] } })
    expect(
      await summarizeChatTitle(
        { call },
        { sessionId: 's', prompt: 'x', model: 'gpt-5' },
      ),
    ).toBeNull()
  })
})
