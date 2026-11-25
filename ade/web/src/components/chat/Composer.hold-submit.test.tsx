// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Composer } from './Composer'

const PROMPT = 'Why five attempts and not eight?'

let root: Root | null = null

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  root = createRoot(document.body.appendChild(document.createElement('div')))
})

afterEach(async () => {
  if (root) await act(async () => root?.unmount())
  root = null
  document.body.innerHTML = ''
})

async function render(
  onSubmit: () => void,
  { submitBlocked = false, holdSubmit = false } = {},
) {
  await act(async () =>
    root?.render(
      <Composer
        model="anthropic::claude-sonnet-5-5"
        modelOptions={[]}
        permissionMode="manual"
        thinkingLevel="default"
        onModelChange={vi.fn()}
        onThinkingLevelChange={vi.fn()}
        onPermissionModeChange={vi.fn()}
        onSubmit={onSubmit}
        initialText={PROMPT}
        submitBlocked={submitBlocked}
        holdSubmit={holdSubmit}
      />,
    ),
  )
}

async function pressEnter() {
  const editor = document.querySelector<HTMLElement>('[contenteditable="true"]')
  expect(editor).not.toBeNull()
  await act(async () => {
    editor?.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'Enter',
        code: 'Enter',
        keyCode: 13,
        bubbles: true,
        cancelable: true,
      }),
    )
  })
}

describe('an Enter while the chat cannot send yet', () => {
  it('sends at once when nothing blocks it', async () => {
    const onSubmit = vi.fn()
    await render(onSubmit)
    await pressEnter()
    expect(onSubmit).toHaveBeenCalledExactlyOnceWith({
      text: PROMPT,
      attachments: [],
    })
  })

  it('is held while the transcript loads, then sent once', async () => {
    const onSubmit = vi.fn()
    await render(onSubmit, { submitBlocked: true, holdSubmit: true })
    await pressEnter()
    expect(onSubmit).not.toHaveBeenCalled()
    // The text stays in the editor until it goes.
    expect(document.body.textContent).toContain(PROMPT)

    await render(onSubmit)
    expect(onSubmit).toHaveBeenCalledExactlyOnceWith({
      text: PROMPT,
      attachments: [],
    })
    await render(onSubmit, { submitBlocked: true, holdSubmit: true })
    await render(onSubmit)
    expect(onSubmit).toHaveBeenCalledOnce()
  })

  it('is still ignored when the block is not one to wait out', async () => {
    const onSubmit = vi.fn()
    await render(onSubmit, { submitBlocked: true })
    await pressEnter()
    await render(onSubmit)
    expect(onSubmit).not.toHaveBeenCalled()
  })
})
