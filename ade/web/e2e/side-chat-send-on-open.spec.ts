import type { Page } from '@playwright/test'
import { expect, openSession, test } from './harness-stack'

/**
 * P31: a message sent in a side chat the moment it opens is sent, to that
 * side chat, and answered.
 *
 * `harness::side-chat::open` answers in milliseconds and the side chat's
 * view mounts at once, but its transcript is still being read
 * (`data-chat-session-hydrated="false"`). An Enter in that window did
 * nothing: no `harness::send`, the text left in the composer, and the empty
 * view under it reading "New session / Connect a model". The window is a few
 * tens of milliseconds, so the race showed in about one run in five. Here
 * the Console's engine socket goes through a proxy that holds every side
 * chat's transcript read back, so each iteration types and sends while the
 * side chat is still loading. Each round is one UI-007 stack (its scenario
 * runs 25 s and scripts one side-chat reply): the round's first side chat
 * is answered, and every later send must still go out, be accepted and
 * show as sent. Leaving one at once, still unread and empty, discards it
 * (`harness::side-chat::close`), where it used to stay behind.
 */

const REPLY = "Five attempts cover the provider's usual outage window."
const SIDE_REPLY = "Eight would outlast the delivery receipt's expiry."
const ROUNDS = 3
const PER_ROUND = 4
/** How long a side chat's transcript read is held back. */
const READ_DELAY_MS = 1200
const TRANSCRIPT_READS = new Set([
  'session::messages-tail',
  'session::messages',
])
const RECORDED = new Set([
  'harness::side-chat::open',
  'harness::side-chat::close',
  'harness::send',
])

interface Invocation {
  fn: string
  inv: string
  data: Record<string, unknown>
}

interface Frame {
  type?: string
  invocation_id?: string
  function_id?: string
  data?: Record<string, unknown>
  result?: Record<string, unknown>
}

function parse(message: string | Buffer): Frame | null {
  if (typeof message !== 'string') return null
  try {
    return JSON.parse(message) as Frame
  } catch {
    return null
  }
}

/**
 * Proxy the Console's engine socket: record the side-chat opens and sends
 * with their answers, and hold back the transcript reads of every session
 * but the conversation's.
 */
async function proxyEngine(page: Page, conversationId: string) {
  const sent: Invocation[] = []
  const results = new Map<string, Record<string, unknown>>()
  const heldReads = new Set<string>()
  await page.routeWebSocket(/.*/, (ws) => {
    const server = ws.connectToServer()
    ws.onMessage((message) => {
      const frame = parse(message)
      if (
        frame?.type === 'invokefunction' &&
        frame.invocation_id &&
        frame.function_id
      ) {
        if (
          TRANSCRIPT_READS.has(frame.function_id) &&
          frame.data?.session_id !== conversationId
        ) {
          heldReads.add(frame.invocation_id)
        }
        if (RECORDED.has(frame.function_id)) {
          sent.push({
            fn: frame.function_id,
            inv: frame.invocation_id,
            data: frame.data ?? {},
          })
        }
      }
      server.send(message)
    })
    server.onMessage((message) => {
      const frame = parse(message)
      if (frame?.type === 'invocationresult' && frame.invocation_id) {
        results.set(frame.invocation_id, frame.result ?? {})
        if (heldReads.delete(frame.invocation_id)) {
          setTimeout(() => ws.send(message), READ_DELAY_MS)
          return
        }
      }
      ws.send(message)
    })
  })
  return {
    lastOpen: () =>
      sent.filter((f) => f.fn === 'harness::side-chat::open').at(-1),
    sendOf: (message: string) =>
      sent.find((f) => f.fn === 'harness::send' && f.data.message === message),
    closeOf: (sessionId: string) =>
      sent.find(
        (f) =>
          f.fn === 'harness::side-chat::close' &&
          f.data.session_id === sessionId,
      ),
    resultOf: (call: Invocation | undefined) =>
      call ? results.get(call.inv) : undefined,
  }
}

test.describe('a side chat sent in as it opens', () => {
  test.use({ scenario: 'console-side-chat' })

  for (let round = 1; round <= ROUNDS; round += 1) {
    test(`round ${round}: ${PER_ROUND} side chats, each sent in at once`, async ({
      page,
      stack,
    }) => {
      page.setDefaultTimeout(10_000)
      const conversationId = stack.ready.session.id
      const engine = await proxyEngine(page, conversationId)
      const completed = stack.waitForTurnCompleted()
      await stack.send()
      expect(await completed).toMatchObject({ status: 'completed' })
      await page.setViewportSize({ width: 1440, height: 900 })
      await openSession(page, stack)
      const transcript = page.locator(
        `[data-chat-session-id="${conversationId}"]`,
      )
      await expect(transcript).toContainText(REPLY)
      const reply = transcript
        .locator('[data-message-role="assistant"]')
        .first()
      const view = page.locator('[data-side-chat-view]')
      const sideChat = view.locator('[data-chat-session-id]')
      const input = view.getByRole('textbox', { name: 'Ask aside…' })
      const paneButton = page.locator('[data-context-pane-button]')

      for (let i = 1; i <= PER_ROUND; i += 1) {
        const message = `Why five attempts and not eight? (${round}.${i})`
        await reply.hover()
        await reply.locator('[data-message-action="side-chat"]').click()
        await input.click()
        // The window the race lives in: the composer is there, the side
        // chat's transcript not read yet.
        await expect(sideChat).toHaveAttribute(
          'data-chat-session-hydrated',
          'false',
        )
        await page.keyboard.type(message)
        await page.keyboard.press('Enter')

        const open = engine.lastOpen()
        const opened = engine.resultOf(open) as { session_id?: string }
        expect(opened?.session_id).toMatch(/^s_/)
        const sideId = opened.session_id as string
        await expect(sideChat).toHaveAttribute('data-chat-session-id', sideId)
        // Held, not dropped: it goes once the side chat is ready, to it.
        await expect
          .poll(() => engine.sendOf(message)?.data.session_id)
          .toBe(sideId)
        await expect
          .poll(() => engine.resultOf(engine.sendOf(message))?.accepted)
          .toBe(true)
        await expect(sideChat).toHaveAttribute(
          'data-chat-session-hydrated',
          'true',
        )
        await expect(input).toHaveText('')
        await expect(
          view
            .locator('[data-message-role="user"]')
            .filter({ hasText: message }),
        ).toHaveCount(1)
        // The round's one scripted side-chat reply.
        if (i === 1) await expect(view).toContainText(SIDE_REPLY)
        await expect(view).not.toContainText('Connect a model')

        await page.getByRole('button', { name: 'Back to context' }).click()
        await expect(view).toHaveCount(0)
        if (
          (await paneButton.getAttribute('aria-label')) === 'Hide context pane'
        ) {
          await paneButton.click()
        }
      }

      if (round === 1) {
        // An empty side chat shows its fork strip over an empty transcript,
        // not the app's "New session / Connect a model".
        await reply.hover()
        await reply.locator('[data-message-action="side-chat"]').click()
        await expect(sideChat).toHaveAttribute(
          'data-chat-session-hydrated',
          'true',
        )
        await expect(view.locator('[data-side-chat-strip]')).toBeVisible()
        await expect(view).not.toContainText('Connect a model')
        await expect(view).not.toContainText('New session')
        await page.getByRole('button', { name: 'Back to context' }).click()
        await expect(view).toHaveCount(0)
        if (
          (await paneButton.getAttribute('aria-label')) === 'Hide context pane'
        ) {
          await paneButton.click()
        }
      }

      // Left at once, unread and empty: discarded all the same.
      await reply.hover()
      await reply.locator('[data-message-action="side-chat"]').click()
      await expect(sideChat).toHaveAttribute(
        'data-chat-session-hydrated',
        'false',
      )
      const left = engine.resultOf(engine.lastOpen()) as { session_id?: string }
      const leftId = left?.session_id as string
      expect(leftId).toMatch(/^s_/)
      await page.getByRole('button', { name: 'Back to context' }).click()
      await expect
        .poll(() => engine.resultOf(engine.closeOf(leftId))?.deleted)
        .toBe(true)
    })
  }
})
