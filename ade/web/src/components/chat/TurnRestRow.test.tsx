// @vitest-environment jsdom
import { act, type ComponentProps, useLayoutEffect } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import type { TurnRest } from '@/lib/backend/harness-send'
import { TurnRestRow } from './TurnRestRow'

let root: Root
let host: HTMLDivElement
const turnRest = vi.fn<(sessionId: string) => Promise<TurnRest | null>>()
const onContinue = vi.fn()

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  turnRest.mockReset()
  onContinue.mockReset()
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})

afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  vi.unstubAllGlobals()
})

async function render(props: Partial<ComponentProps<typeof TurnRestRow>> = {}) {
  await act(async () =>
    root.render(
      <TurnRestRow
        sessionId="s-1"
        atRest
        turnRest={turnRest}
        onContinue={onContinue}
        {...props}
      />,
    ),
  )
}

/** A read that answers when the test says so. */
function pendingRead() {
  let answer!: (rest: TurnRest | null) => void
  turnRest.mockReturnValueOnce(
    new Promise((resolve) => {
      answer = resolve
    }),
  )
  return (rest: TurnRest | null) => act(async () => answer(rest))
}

const button = () => host.querySelector('button')

it('shows nothing until the read answers, then offers Continue on an ended turn', async () => {
  const answer = pendingRead()
  await render()
  expect(turnRest).toHaveBeenCalledWith('s-1')
  expect(host.innerHTML).toBe('')
  await answer('ended')
  expect(host.querySelector('[role=status]')?.textContent).toBe('Turn ended.')
  expect(button()?.textContent).toBe('Continue')
})

it('says a waiting turn waits, with no button', async () => {
  turnRest.mockResolvedValue('waiting')
  await render()
  expect(host.textContent).toBe('Waiting for a notification.')
  expect(button()).toBeNull()
})

it('shows nothing when the state is unknown or the read fails', async () => {
  turnRest.mockResolvedValueOnce(null)
  await render()
  expect(host.innerHTML).toBe('')

  turnRest.mockRejectedValueOnce(new Error('status failed'))
  await render({ sessionId: 's-2' })
  expect(turnRest).toHaveBeenCalledTimes(2)
  expect(host.innerHTML).toBe('')
})

it('does not read while a turn is live, and reads once each time it ends', async () => {
  turnRest.mockResolvedValue('waiting')
  await render({ atRest: false })
  expect(turnRest).not.toHaveBeenCalled()
  await render()
  await render()
  expect(turnRest).toHaveBeenCalledTimes(1)
  expect(host.textContent).toBe('Waiting for a notification.')

  // A turn starts: the row goes, and the old answer does not come back
  // before the next read does.
  await render({ atRest: false })
  expect(host.innerHTML).toBe('')
  const answer = pendingRead()
  await render()
  expect(turnRest).toHaveBeenCalledTimes(2)
  expect(host.innerHTML).toBe('')
  await answer('ended')
  expect(host.textContent).toContain('Turn ended.')
})

it('hides on the click, so a second click cannot send again', async () => {
  turnRest.mockResolvedValue('ended')
  await render()
  const clicked = button()
  await act(async () => clicked?.click())
  expect(onContinue).toHaveBeenCalledTimes(1)
  expect(host.innerHTML).toBe('')
  // The chat is still at rest while the send is being prepared.
  await render()
  await act(async () => clicked?.click())
  expect(onContinue).toHaveBeenCalledTimes(1)
  expect(host.innerHTML).toBe('')
  expect(turnRest).toHaveBeenCalledTimes(1)
})

it('offers Continue again after the turn the click started', async () => {
  turnRest.mockResolvedValue('ended')
  await render()
  await act(async () => button()?.click())
  await render({ atRest: false })
  await render()
  expect(turnRest).toHaveBeenCalledTimes(2)
  expect(button()?.textContent).toBe('Continue')
})

it('is gone in the same commit that a turn starts in', async () => {
  turnRest.mockResolvedValue('ended')
  const committed: string[] = []
  function Probe({ atRest }: { atRest: boolean }) {
    useLayoutEffect(() => {
      if (!atRest) committed.push(host.innerHTML)
    })
    return (
      <TurnRestRow
        sessionId="s-1"
        atRest={atRest}
        turnRest={turnRest}
        onContinue={onContinue}
      />
    )
  }
  await act(async () => root.render(<Probe atRest />))
  expect(button()).not.toBeNull()
  await act(async () => root.render(<Probe atRest={false} />))
  expect(committed).toEqual([''])
})

it('ignores an answer that arrives after a turn started', async () => {
  const answer = pendingRead()
  await render()
  await render({ atRest: false })
  await answer('ended')
  expect(host.innerHTML).toBe('')
  const next = pendingRead()
  await render()
  expect(host.innerHTML).toBe('')
  await next('waiting')
  expect(host.textContent).toBe('Waiting for a notification.')
})

it('drops the row of a session that is no longer shown', async () => {
  turnRest.mockResolvedValueOnce('ended')
  await render()
  expect(button()).not.toBeNull()
  pendingRead()
  await render({ sessionId: 's-2' })
  expect(host.innerHTML).toBe('')
})

it('ignores an answer for a session that is no longer shown', async () => {
  const answer = pendingRead()
  await render()
  const next = pendingRead()
  await render({ sessionId: 's-2' })
  await answer('ended')
  expect(host.innerHTML).toBe('')
  await next('waiting')
  expect(turnRest).toHaveBeenLastCalledWith('s-2')
  expect(host.textContent).toBe('Waiting for a notification.')
})
