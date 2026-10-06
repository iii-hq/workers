// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { ChatBackend } from '@/lib/backend'
import type { Message } from '@/types/chat'
import { useModelSwitch } from './use-model-switch'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true
const cleanup: Array<() => void> = []
afterEach(() => {
  for (const stop of cleanup.splice(0)) stop()
})
const button = (text: string) => {
  const found = [...document.querySelectorAll('button')].find(
    (b) => b.textContent === text,
  )
  if (!found) throw new Error(`Missing button: ${text}`)
  return found
}
const marker = (): Message => ({
  id: 'marker',
  role: 'system',
  kind: 'compaction',
  content: 'conversation compacted',
  createdAt: 2,
})
const external = (): Message => ({
  id: 'external',
  role: 'system',
  kind: 'notice',
  content: 'external change',
  createdAt: 2,
})

function setup(needsCompaction = true) {
  const preview = vi
    .fn()
    .mockResolvedValue({ needsCompaction, tokens: 200000, usable: 64000 })
  const compact = vi.fn().mockResolvedValue({
    status: 'ok',
    tokensBefore: 190000,
    summaryText: 'summary',
    compactionEntryId: 'marker',
  })
  const onSwitch = vi.fn()
  const onCompacted = vi.fn()
  const backend = {
    id: 'real',
    previewModelSwitch: preview,
    compactSession: compact,
  } as unknown as ChatBackend
  let api!: ReturnType<typeof useModelSwitch>
  let revision = 1
  let disabled = false
  let currentModel: 'large' | 'other' = 'large'
  let transcript: Message[] = []
  const root = createRoot(
    document.body.appendChild(document.createElement('div')),
  )
  function Probe() {
    api = useModelSwitch({
      backend,
      sessionId: 's',
      currentModel,
      models: [{ id: 'small', label: 'Small model' }],
      draft: false,
      disabled,
      revision,
      transcript,
      onSwitch,
      onCompacted,
    })
    return <>{api.dialog}</>
  }
  act(() => root.render(<Probe />))
  const stop = () => act(() => root.unmount())
  cleanup.push(stop)
  const request = async () => {
    let pending!: Promise<boolean>
    await act(async () => {
      pending = api.request('small', 'low')
    })
    return { pending }
  }
  return {
    preview,
    compact,
    onSwitch,
    onCompacted,
    request,
    get api() {
      return api
    },
    changeHistory: () =>
      act(() => {
        revision++
        root.render(<Probe />)
      }),
    setDisabled: (value: boolean) =>
      act(() => {
        disabled = value
        root.render(<Probe />)
      }),
    setCurrentModel: (value: 'large' | 'other') =>
      act(() => {
        currentModel = value
        root.render(<Probe />)
      }),
    setTranscript: (next: Message[]) =>
      act(() => {
        transcript = next
        revision++
        root.render(<Probe />)
      }),
    stop,
  }
}

describe('model switch confirmation', () => {
  it('shows English consent with Cancel focused and does not compact before acceptance', async () => {
    const s = setup()
    const { pending } = await s.request()
    expect(
      document.querySelector('[role="alertdialog"]')?.textContent,
    ).toContain('Compact this conversation to switch models?')
    expect(document.activeElement).toBe(button('Cancel switch'))
    expect(s.compact).not.toHaveBeenCalled()
    expect(s.onSwitch).not.toHaveBeenCalled()
    await act(async () => button('Cancel switch').click())
    await expect(pending).resolves.toBe(false)
    expect(s.compact).not.toHaveBeenCalled()
    expect(s.onSwitch).not.toHaveBeenCalled()
  })
  it('compacts with the destination model and only switches after successful persistence', async () => {
    const s = setup()
    let finish!: (value: unknown) => void
    s.compact.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve
      }),
    )
    const { pending } = await s.request()
    await act(async () => button('Compact and switch').click())
    expect(s.compact).toHaveBeenCalledWith('s', 'small')
    expect(s.onSwitch).not.toHaveBeenCalled()
    await act(async () => finish({ status: 'ok', compactionEntryId: 'marker' }))
    await expect(pending).resolves.toBe(true)
    expect(s.onCompacted).toHaveBeenCalledWith(true)
    expect(s.onSwitch).toHaveBeenCalledTimes(1)
    expect(s.onSwitch).toHaveBeenCalledWith('small')
  })
  it('switches without a popup or inference when history fits', async () => {
    const s = setup(false)
    const { pending } = await s.request()
    await expect(pending).resolves.toBe(true)
    expect(document.querySelector('[role="alertdialog"]')).toBeNull()
    expect(s.compact).not.toHaveBeenCalled()
    expect(s.onSwitch).toHaveBeenCalledTimes(1)
    expect(s.onSwitch).toHaveBeenCalledWith('small')
  })
  it('Escape cancels and preserves the old model', async () => {
    const s = setup()
    const { pending } = await s.request()
    await act(async () =>
      document.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }),
      ),
    )
    await expect(pending).resolves.toBe(false)
    expect(s.compact).not.toHaveBeenCalled()
  })
  it('does not switch if compaction fails', async () => {
    const s = setup()
    s.compact.mockResolvedValue({
      status: 'overflow',
      message: 'Cannot fit history',
    })
    const { pending } = await s.request()
    await act(async () => button('Compact and switch').click())
    await expect(pending).resolves.toBe(false)
    expect(s.onSwitch).not.toHaveBeenCalled()
    expect(s.api.error).toContain('Cannot fit history')
  })
  it('fails closed when preview cannot determine the budget', async () => {
    const s = setup()
    s.preview.mockRejectedValue(new Error('Router unavailable'))
    const { pending } = await s.request()
    await expect(pending).resolves.toBe(false)
    expect(s.api.error).toContain('Router unavailable')
    expect(s.compact).not.toHaveBeenCalled()
    expect(s.onSwitch).not.toHaveBeenCalled()
  })
  it('rejects duplicate selection and stale history before starting compaction', async () => {
    const s = setup()
    const { pending } = await s.request()
    await expect(s.api.request('other')).resolves.toBe(false)
    s.changeHistory()
    await act(async () => button('Compact and switch').click())
    await expect(pending).resolves.toBe(false)
    expect(s.api.error).toContain('conversation changed')
    expect(s.compact).not.toHaveBeenCalled()
  })
  it('ignores a preview response after the view unmounts', async () => {
    const s = setup()
    let finish!: (value: unknown) => void
    s.preview.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve
      }),
    )
    const { pending } = await s.request()
    s.stop()
    await act(async () => finish({ needsCompaction: false }))
    await expect(pending).resolves.toBe(false)
    expect(s.onSwitch).not.toHaveBeenCalled()
  })
  it('reports a conversation change while checking instead of silently dropping the request', async () => {
    const s = setup()
    let finish!: (value: unknown) => void
    s.preview.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve
      }),
    )
    const { pending } = await s.request()
    s.changeHistory()
    await act(async () =>
      finish({ needsCompaction: true, tokens: 200000, usable: 64000 }),
    )
    await expect(pending).resolves.toBe(false)
    expect(s.api.error).toContain('conversation changed while checking')
    expect(s.compact).not.toHaveBeenCalled()
    expect(s.onSwitch).not.toHaveBeenCalled()
  })
  it('reports persisted compaction when the current state disables the switch', async () => {
    const s = setup()
    let finish!: (value: unknown) => void
    s.compact.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve
      }),
    )
    const { pending } = await s.request()
    await act(async () => button('Compact and switch').click())
    s.setDisabled(true)
    await act(async () => finish({ status: 'ok', compactionEntryId: 'marker' }))
    await expect(pending).resolves.toBe(false)
    expect(s.onCompacted).toHaveBeenCalledWith(false)
    expect(s.onSwitch).not.toHaveBeenCalled()
    expect(s.api.error).toContain(
      'compacted, but the model switch was not applied',
    )
  })
  it('rejects a current-model change while compacting', async () => {
    const s = setup()
    let finish!: (value: unknown) => void
    s.compact.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve
      }),
    )
    const { pending } = await s.request()
    await act(async () => button('Compact and switch').click())
    s.setCurrentModel('other')
    await act(async () => finish({ status: 'ok', compactionEntryId: 'marker' }))
    await expect(pending).resolves.toBe(false)
    expect(s.onCompacted).toHaveBeenCalledWith(false)
    expect(s.onSwitch).not.toHaveBeenCalled()
  })
  it('does not treat its own persisted marker as an external conversation change', async () => {
    const s = setup()
    let finish!: (value: unknown) => void
    s.compact.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve
      }),
    )
    const { pending } = await s.request()
    await act(async () => button('Compact and switch').click())
    s.setTranscript([marker()])
    await act(async () => finish({ status: 'ok', compactionEntryId: 'marker' }))
    await expect(pending).resolves.toBe(true)
    expect(s.onSwitch).toHaveBeenCalledTimes(1)
  })
  it('rejects an external transcript change even when the own marker is present', async () => {
    const s = setup()
    let finish!: (value: unknown) => void
    s.compact.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve
      }),
    )
    const { pending } = await s.request()
    await act(async () => button('Compact and switch').click())
    s.setTranscript([external(), marker()])
    await act(async () => finish({ status: 'ok', compactionEntryId: 'marker' }))
    await expect(pending).resolves.toBe(false)
    expect(s.onCompacted).toHaveBeenCalledWith(false)
    expect(s.onSwitch).not.toHaveBeenCalled()
    expect(s.api.error).toContain('conversation changed')
  })
  it('does not call back again when the own marker arrives after the switch result', async () => {
    const s = setup()
    const { pending } = await s.request()
    await act(async () => button('Compact and switch').click())
    await expect(pending).resolves.toBe(true)
    s.setTranscript([marker()])
    expect(s.onSwitch).toHaveBeenCalledTimes(1)
  })
})
