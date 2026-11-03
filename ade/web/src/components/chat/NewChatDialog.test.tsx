// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { registerExtComposerAction } from '@/lib/ui-slots'
import { NewChatDialog } from './NewChatDialog'

vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)

let root: Root | null = null
const offs: Array<() => void> = []
afterEach(async () => {
  await act(async () => root?.unmount())
  root = null
  for (const off of offs.splice(0)) off()
  document.body.replaceChildren()
})

async function mount() {
  const onCreate = vi.fn()
  const onOpenFull = vi.fn()
  const container = document.body.appendChild(document.createElement('div'))
  root = createRoot(container)
  await act(async () =>
    root?.render(
      <NewChatDialog
        open
        onOpenChange={vi.fn()}
        modelOptions={[]}
        catalogLoading={false}
        onCreate={onCreate}
        onOpenFull={onOpenFull}
      />,
    ),
  )
  return { onCreate, onOpenFull }
}

const byLabel = (label: string) =>
  document.body.querySelector<HTMLElement>(`[aria-label="${label}"]`)

async function type(text: string) {
  const field = document.body.querySelector('textarea') as HTMLTextAreaElement
  const setter = Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    'value',
  )?.set
  await act(async () => {
    setter?.call(field, text)
    field.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

describe('NewChatDialog', () => {
  it('offers attach and the injected mic beside Create', async () => {
    offs.push(
      registerExtComposerAction({
        id: 'voice-dictate',
        scope: 'voice',
        path: 'voice/page.js',
        render: () => <button type="button" aria-label="Dictate" />,
      }),
    )
    await mount()
    expect(byLabel('attach files')).not.toBeNull()
    expect(byLabel('Dictate')).not.toBeNull()
  })

  it('carries the draft into a full chat without sending it', async () => {
    const { onCreate, onOpenFull } = await mount()
    await type('Add tags to bookmarks')
    await act(async () => byLabel('Open as a full chat')?.click())
    expect(onCreate).not.toHaveBeenCalled()
    expect(onOpenFull).toHaveBeenCalledWith(
      expect.objectContaining({
        text: 'Add tags to bookmarks',
        attachments: [],
      }),
    )
  })
})
