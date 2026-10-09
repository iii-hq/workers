// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { claimDraftSend, sendDraftWhenReady } from '@/lib/composer-insert'
import { Composer } from './Composer'

let root: Root | null = null

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  root = createRoot(document.body.appendChild(document.createElement('div')))
})

afterEach(async () => {
  if (root) await act(async () => root?.unmount())
  root = null
})

async function render(submitBlocked: boolean, onSubmit: () => void) {
  await act(async () =>
    root?.render(
      <Composer
        model={null}
        modelOptions={[]}
        permissionMode="manual"
        thinkingLevel="default"
        onModelChange={vi.fn()}
        onThinkingLevelChange={vi.fn()}
        onPermissionModeChange={vi.fn()}
        onSubmit={onSubmit}
        initialText="Build a link shortener."
        submitBlocked={submitBlocked}
        claimDraftSend={() => claimDraftSend('chat-1')}
      />,
    ),
  )
}

describe('a draft whose send was already decided', () => {
  it('waits until the composer can send, then sends it once', async () => {
    const onSubmit = vi.fn()
    sendDraftWhenReady('chat-1')

    await render(true, onSubmit)
    expect(onSubmit).not.toHaveBeenCalled()

    await render(false, onSubmit)
    expect(onSubmit).toHaveBeenCalledExactlyOnceWith({
      text: 'Build a link shortener.',
      attachments: [],
    })

    await render(true, onSubmit)
    await render(false, onSubmit)
    expect(onSubmit).toHaveBeenCalledOnce()
  })

  it('stays a draft when nobody asked to send it', async () => {
    const onSubmit = vi.fn()
    await render(false, onSubmit)
    expect(onSubmit).not.toHaveBeenCalled()
  })
})
