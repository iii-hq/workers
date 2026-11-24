import { expect, expectPassingResult, openSession, test } from './harness-stack'

/*
 * chat-projects §2.4 / §2.8 (1–4): side chats from the engine, on the
 * scripted stack of harness integration INT-046 (`side-chat-same-prefix`).
 *
 * The conversation runs one turn. The fixture's probe then forks a side chat
 * at the reply (`harness::side-chat::open`) and sends in it, with no model
 * and no option: the harness runs it on the conversation's lock. The run's
 * own check (its `verify`, read here as the playground result) holds the
 * cache claim: both router requests carry the same system prompt, system
 * sections and tools, the side chat's history starts with the
 * conversation's up to the reply, and nothing reaches the conversation.
 *
 * The Console then shows what §2.3 asks: the mark under the forked reply,
 * the side-chat view with its fork strip, locked model and "Ask aside…",
 * "Bring back" filling the conversation's draft (and the server's), the
 * Sidechats section with "N messages", and "+ New" forking through the
 * engine, discarded with `close` when left empty.
 */
const MESSAGE =
  'The webhook retries five times before it gives up; explain why.'
const REPLY = "Five attempts cover the provider's usual outage window."
const SIDE_MESSAGE = 'Why five attempts and not eight?'
const SIDE_REPLY = "Eight would outlast the delivery receipt's expiry."

interface SideChatItem {
  session_id: string
  title: string
  message_count: number
  from_entry_id?: string
  running: boolean
}

test.describe('side chats from the engine', () => {
  test.use({ scenario: 'side-chat-same-prefix' })

  test('fork, send, the same prefix, and the side chat in the Console', async ({
    page,
    stack,
  }) => {
    const session = stack.ready.session.id
    const title = stack.ready.session.title
    const sideChats = async () =>
      (
        (await stack.invoke('harness::context-items', {
          session_id: session,
        })) as { side_chats?: SideChatItem[] }
      ).side_chats ?? []

    const completed = stack.waitForTurnCompleted()
    await stack.send()
    expect(await completed).toMatchObject({ status: 'completed' })
    // The probe's side chat: forked at the reply, sent in, answered.
    await expect
      .poll(
        async () => {
          const [row] = await sideChats()
          return row && !row.running ? row.message_count : 0
        },
        { timeout: 60_000 },
      )
      .toBeGreaterThanOrEqual(2)
    const [probe] = await sideChats()
    expect(probe.title).toBe(SIDE_MESSAGE)
    expect(probe.from_entry_id).toBeTruthy()

    await page.setViewportSize({ width: 1440, height: 900 })
    await openSession(page, stack)
    const transcript = page.locator(`[data-chat-session-id="${session}"]`)
    await expect(transcript).toContainText(REPLY)
    // Nothing of the side chat in the conversation.
    await expect(transcript).not.toContainText(SIDE_REPLY)

    // The fork point mark under the reply opens the side chat.
    const mark = transcript.locator(
      `[data-side-chat-mark="${probe.session_id}"]`,
    )
    await expect(mark).toHaveText(`Sidechat from here · ${SIDE_MESSAGE}`)
    await mark.click()
    const view = page.locator(`[data-side-chat-view="${probe.session_id}"]`)
    await expect(view).toBeVisible()
    await expect(page.locator('[data-side-chat-header]')).toContainText(
      SIDE_MESSAGE,
    )
    await expect(view.locator('[data-side-chat-strip]')).toContainText(
      "Knows the conversation up to the agent's reply at",
    )
    await expect(view.locator('[data-side-chat-strip]')).toContainText(
      `Nothing here goes into ${title}.`,
    )
    // Its own messages only; the copied history stays in the conversation.
    await expect(view).toContainText(SIDE_MESSAGE)
    await expect(view).toContainText(SIDE_REPLY)
    await expect(view).not.toContainText(MESSAGE)
    // The lock: a label, no picker, and why on a tap.
    const locked = view.locator('[data-locked-model]:visible')
    await expect(locked).toBeVisible()
    await locked.click()
    await expect(page.locator('[data-locked-model-hint]')).toHaveText(
      "A side chat keeps its conversation's model and effort, to reuse its cache.",
    )
    await page.keyboard.press('Escape')
    await expect(view.locator('[contenteditable="true"]').first()).toBeVisible()
    await expect(view).toContainText('Ask aside…')

    // Bring back: into the conversation's composer, unsent, and its draft.
    await view.locator('[data-bring-back]').first().click()
    await expect(page.locator('[data-side-chat-toast]')).toContainText(
      `Added to the ${title} composer`,
    )
    const mainComposer = transcript.locator('[contenteditable="true"]').first()
    await expect(mainComposer).toContainText(
      `From side chat “${SIDE_MESSAGE}”:`,
    )
    await expect(mainComposer).toContainText(SIDE_REPLY)
    await expect
      .poll(
        async () => {
          const meta = (await stack.invoke('session::get', {
            session_id: session,
          })) as {
            draft?: string | null
            session?: { draft?: string | null }
            meta?: { draft?: string | null }
          }
          return meta.draft ?? meta.session?.draft ?? meta.meta?.draft ?? ''
        },
        { timeout: 15_000 },
      )
      .toContain(`From side chat “${SIDE_MESSAGE}”:`)

    // Back to the list: the Sidechats section, with what was said.
    await page.getByRole('button', { name: 'Back to context' }).click()
    const section = page.getByRole('region', { name: 'Sidechats' })
    await expect(section).toBeVisible()
    const row = section.locator(`[data-side-chat-row="${probe.session_id}"]`)
    await expect(row).toContainText(SIDE_MESSAGE)
    await expect(section).toContainText(`${probe.message_count} messages ·`)

    // "+ New" forks at the last reply through the engine; left empty, it
    // is discarded with `close`, without asking.
    await section.locator('[data-side-chat-new]').click()
    await expect(page.locator('[data-side-chat-header]')).toContainText(
      'Side chat',
    )
    await expect
      .poll(async () => (await sideChats()).length, { timeout: 15_000 })
      .toBe(2)
    await expect(
      page.locator('[data-side-chat-view] [data-locked-model]:visible'),
    ).toBeVisible()
    await page.getByRole('button', { name: 'Back to context' }).click()
    await expect
      .poll(async () => (await sideChats()).length, { timeout: 30_000 })
      .toBe(1)
    await expect(section.locator('[data-side-chat-row]')).toHaveCount(1)

    // The run's own check: the same system prompt, sections and tools,
    // the history prefix, and nothing written into the conversation.
    const result = await stack.finish()
    expectPassingResult(result)
    expect(result.evidence?.generations_consumed).toBe(2)
  })
})
