// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { Composer } from './Composer'

vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
const roots: Root[] = []
afterEach(async () => {
  for (const root of roots.splice(0)) await act(async () => root.unmount())
  document.body.replaceChildren()
})

async function mount(initialText = '') {
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  roots.push(root)
  const onSubmit = vi.fn()
  await act(async () =>
    root.render(
      <Composer
        model="test-model"
        modelOptions={[]}
        permissionMode="manual"
        thinkingLevel="default"
        onModelChange={vi.fn()}
        onThinkingLevelChange={vi.fn()}
        onPermissionModeChange={vi.fn()}
        onSubmit={onSubmit}
        initialText={initialText}
        showSuggestions
        externalInput={false}
      />,
    ),
  )
  return { container, onSubmit }
}

describe('composer starter prompts', () => {
  it('fills only its own editor and waits for the user to send', async () => {
    const first = await mount()
    const second = await mount()
    const starter = Array.from(first.container.querySelectorAll('button')).find(
      (button) => button.textContent === 'Explain this project',
    )
    expect(starter).toBeDefined()
    const editor = first.container.querySelector('[contenteditable="true"]')
    await act(async () => starter?.click())
    expect(editor?.textContent).toContain("Explain this project's architecture")
    expect(
      second.container.querySelector('[contenteditable="true"]')?.textContent,
    ).toBe('')
    expect(first.onSubmit).not.toHaveBeenCalled()
    expect(second.onSubmit).not.toHaveBeenCalled()
    await act(async () =>
      first.container
        .querySelector<HTMLButtonElement>('[aria-label="send message"]')
        ?.click(),
    )
    expect(first.onSubmit).toHaveBeenCalledWith({
      text: "Explain this project's architecture and where I should start.",
      attachments: [],
    })
  })

  it('keeps an existing draft without offering replacement prompts', async () => {
    const { container } = await mount('Keep my unfinished request')
    expect(container.textContent).not.toContain('Explain this project')
    expect(
      container.querySelector('[contenteditable="true"]')?.textContent,
    ).toBe('Keep my unfinished request')
  })
})
