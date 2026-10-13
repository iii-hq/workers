// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { claimDraftSend, sendDraftWhenReady } from '@/lib/composer-insert'
import { Composer } from './Composer'

const PROMPT = 'Build a link shortener.'

let root: Root | null = null

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  root = createRoot(document.body.appendChild(document.createElement('div')))
})

afterEach(async () => {
  if (root) await act(async () => root?.unmount())
  root = null
})

async function render(
  onSubmit: () => void,
  {
    submitBlocked = false,
    model = 'anthropic::claude-sonnet-5-5' as string | null,
  } = {},
) {
  await act(async () =>
    root?.render(
      <Composer
        model={model}
        modelOptions={[]}
        permissionMode="manual"
        thinkingLevel="default"
        onModelChange={vi.fn()}
        onThinkingLevelChange={vi.fn()}
        onPermissionModeChange={vi.fn()}
        onSubmit={onSubmit}
        initialText={PROMPT}
        submitBlocked={submitBlocked}
        claimDraftSend={() => claimDraftSend('chat-1')}
      />,
    ),
  )
}

describe('a draft whose send was already decided', () => {
  it('waits until the composer can send, then sends it once', async () => {
    const onSubmit = vi.fn()
    sendDraftWhenReady('chat-1', PROMPT)

    await render(onSubmit, { submitBlocked: true })
    expect(onSubmit).not.toHaveBeenCalled()

    await render(onSubmit)
    expect(onSubmit).toHaveBeenCalledExactlyOnceWith({
      text: PROMPT,
      attachments: [],
    })

    await render(onSubmit, { submitBlocked: true })
    await render(onSubmit)
    expect(onSubmit).toHaveBeenCalledOnce()
  })

  it('waits for a model, which a send without one would refuse', async () => {
    const onSubmit = vi.fn()
    sendDraftWhenReady('chat-1', PROMPT)

    await render(onSubmit, { model: null })
    expect(onSubmit).not.toHaveBeenCalled()

    // The claim was left for the model: it goes once one is chosen.
    await render(onSubmit)
    expect(onSubmit).toHaveBeenCalledOnce()
  })

  it('leaves a draft edited since the send was decided to the user', async () => {
    const onSubmit = vi.fn()
    sendDraftWhenReady('chat-1', 'Build an expense tracker.')

    await render(onSubmit)
    expect(onSubmit).not.toHaveBeenCalled()
    // Claimed on the first chance to send, so it cannot fire later.
    expect(claimDraftSend('chat-1')).toBeUndefined()
  })

  it('stays a draft when nobody asked to send it', async () => {
    const onSubmit = vi.fn()
    await render(onSubmit)
    expect(onSubmit).not.toHaveBeenCalled()
  })
})
