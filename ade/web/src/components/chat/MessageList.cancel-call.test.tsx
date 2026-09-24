// @vitest-environment jsdom

import { act, StrictMode } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { FunctionTriggerMessage } from '@/types/chat'
import { MessageList } from './MessageList'

/**
 * The per-call stop button: `MessageList` → `Message` → `FunctionTriggerCard`
 * wiring of `onCancelCall` (`harness::function::cancel`), and the card's
 * cancelling / refused / settled states.
 */

const running: FunctionTriggerMessage = {
  id: 'e_assistant:1',
  role: 'function-trigger',
  functionTriggerId: 'call_a',
  sessionId: 's-1',
  functionId: 'shell::exec',
  input: { command: 'make', args: ['-j8'] },
  running: true,
  createdAt: 0,
}
const cancelledOutput = {
  error: {
    kind: 'function_error',
    message: 'shell::exec was cancelled by the user before it returned.',
    details: {
      error: 'cancelled',
      cancelled_by: 'user',
      function_id: 'shell::exec',
    },
  },
}
const cancel = '[data-message-action="cancel"]'
const cancelError = '[data-function-trigger-cancel-error]'

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(() => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
})

/** Click the stop button and let its handler's promise settle inside act. */
async function clickCancel() {
  await act(async () => {
    container.querySelector<HTMLButtonElement>(cancel)?.click()
    await new Promise<void>((resolve) => setTimeout(resolve, 0))
  })
}

async function render(
  messages: FunctionTriggerMessage[],
  onCancelCall?: (
    sessionId: string,
    functionTriggerId: string,
  ) => Promise<void>,
) {
  await act(() => {
    root.render(
      <StrictMode>
        <MessageList messages={messages} onCancelCall={onCancelCall} />
      </StrictMode>,
    )
  })
}

function cancelButton() {
  return container.querySelector<HTMLButtonElement>(cancel)
}

describe('per-call cancel button', () => {
  it('renders only for a running, addressable call when the backend can cancel', async () => {
    // No handler: backends without per-call cancellation show no button.
    await render([running])
    expect(cancelButton()).toBeNull()

    await render(
      [running],
      vi.fn(async () => {}),
    )
    expect(cancelButton()).not.toBeNull()
    expect(cancelButton()?.getAttribute('aria-label')).toBe(
      'cancel shell::exec',
    )

    // A parked call is denied, not cancelled; a settled one is history.
    await render(
      [{ ...running, pendingApproval: true }],
      vi.fn(async () => {}),
    )
    expect(cancelButton()).toBeNull()
    await render(
      [{ ...running, running: false, output: { ok: true }, durationMs: 5 }],
      vi.fn(async () => {}),
    )
    expect(cancelButton()).toBeNull()

    // Without the harness ids there is nothing to address.
    await render(
      [{ ...running, functionTriggerId: undefined }],
      vi.fn(async () => {}),
    )
    expect(cancelButton()).toBeNull()
  })

  it('calls the backend with the session and call ids, then waits for the result to settle the card', async () => {
    const onCancelCall = vi.fn(async () => {})
    await render([running], onCancelCall)

    await clickCancel()
    expect(onCancelCall).toHaveBeenCalledExactlyOnceWith('s-1', 'call_a')
    // The RPC resolved but the call has not settled: the button stays busy
    // (the harness reports the cancelled result through the transcript).
    expect(cancelButton()?.disabled).toBe(true)
    expect(cancelButton()?.getAttribute('aria-label')).toBe(
      'cancelling shell::exec',
    )
    // A repeat click while busy is a no-op.
    await clickCancel()
    expect(onCancelCall).toHaveBeenCalledTimes(1)

    // The cancelled result pairs in: the card settles as "Cancelled", not
    // "Failed", and the button is gone.
    await render(
      [
        {
          ...running,
          running: false,
          output: cancelledOutput,
          durationMs: 1200,
        },
      ],
      onCancelCall,
    )
    expect(cancelButton()).toBeNull()
    const card = container.querySelector('[data-message-role="function-call"]')
    expect(card?.getAttribute('data-function-status')).toBe('error')
    const errorCopy = container.querySelector(
      '[data-function-status-text="error"]',
    )
    expect(errorCopy?.textContent).toContain('Cancelled')
    expect(errorCopy?.textContent).not.toContain('Failed')
  })

  it('re-enables the button and explains when the harness refuses the cancel', async () => {
    const onCancelCall = vi.fn(async () => {
      throw new Error('nothing to cancel — the call already finished')
    })
    await render([running], onCancelCall)

    await clickCancel()
    expect(cancelButton()?.disabled).toBe(false)
    expect(container.querySelector(cancelError)?.textContent).toContain(
      'nothing to cancel',
    )

    // A retry is possible after a refusal.
    await clickCancel()
    expect(onCancelCall).toHaveBeenCalledTimes(2)
  })

  it('clears a stale refusal once the call settles', async () => {
    const onCancelCall = vi.fn(async () => {
      throw new Error('boom')
    })
    await render([running], onCancelCall)
    await clickCancel()
    expect(container.querySelector(cancelError)).not.toBeNull()

    await render(
      [{ ...running, running: false, output: { ok: true }, durationMs: 5 }],
      onCancelCall,
    )
    expect(container.querySelector(cancelError)).toBeNull()
  })
})
