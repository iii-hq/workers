import { expect, expectPassingResult, openSession, test } from './harness-stack'

/*
 * chat-projects §2.4 / §2.8 (1–4), on the scripted stack of harness
 * integration UI-007 (`console-side-chat`, D96).
 *
 * The conversation runs one turn. The CONSOLE then opens a side chat with
 * "Side chat" on the reply (`harness::side-chat::open`) and sends in it
 * through `harness::send` with no model, provider or effort: the harness
 * runs it on the conversation's lock. The run's own check (its `verify`,
 * read here as the playground result) holds the cache claim: both router
 * requests carry the same system prompt, system sections and tools, the
 * side chat's history starts with the conversation's up to the reply,
 * nothing reaches the conversation, and exactly two generations ran.
 *
 * The Console shows what §2.3 asks: the fork strip, the locked model label
 * with its effort (D97), the textbox named "Ask aside…", the mark under the
 * forked reply that opens the side chat again, "Bring back" filling the
 * conversation's draft (and the server's), the Sidechats row with its
 * count, and "+ New" forking through the engine, discarded with `close`
 * when left empty.
 */
const MESSAGE =
  'The webhook retries five times before it gives up; explain why.'
const REPLY = "Five attempts cover the provider's usual outage window."
const SIDE_MESSAGE = 'Why five attempts and not eight?'
const SIDE_REPLY = "Eight would outlast the delivery receipt's expiry."
const EFFORTS = ['Default', 'Off', 'Minimal', 'Low', 'Medium', 'High', 'Xhigh']

interface SideChatItem {
  session_id: string
  title: string
  message_count: number
  from_entry_id?: string
  running: boolean
}

/** Every `harness::send` payload in a frame the page sent to the engine. */
function harnessSends(frame: string): Record<string, unknown>[] {
  let parsed: unknown
  try {
    parsed = JSON.parse(frame)
  } catch {
    return []
  }
  const found: Record<string, unknown>[] = []
  const walk = (value: unknown) => {
    if (!value || typeof value !== 'object') return
    const record = value as Record<string, unknown>
    if (record.function_id === 'harness::send') {
      const payload = record.data ?? record.payload ?? record.input
      if (payload && typeof payload === 'object') {
        found.push(payload as Record<string, unknown>)
      }
    }
    for (const child of Object.values(record)) walk(child)
  }
  walk(parsed)
  return found
}

test.describe('side chats from the engine', () => {
  test.use({ scenario: 'console-side-chat' })

  test('the Console forks at the reply and sends with no model or effort; the side chat runs on the prefix', async ({
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

    // What the page sends to the engine: the side chat's harness::send.
    const sends: Record<string, unknown>[] = []
    page.on('websocket', (ws) => {
      ws.on('framesent', ({ payload }) => {
        if (typeof payload === 'string') sends.push(...harnessSends(payload))
      })
    })

    const completed = stack.waitForTurnCompleted()
    await stack.send()
    expect(await completed).toMatchObject({ status: 'completed' })

    await page.setViewportSize({ width: 1440, height: 900 })
    await openSession(page, stack)
    const transcript = page.locator(`[data-chat-session-id="${session}"]`)
    await expect(transcript).toContainText(REPLY)

    // "Side chat" on the reply, no quote.
    await transcript
      .locator('[data-message-action="side-chat"]')
      .first()
      .click()
    const view = page.locator('[data-side-chat-view]')
    const strip = view.locator('[data-side-chat-strip]')
    await expect(strip).toContainText(
      "Knows the conversation up to the agent's reply at",
    )
    await expect(strip).toContainText(`Nothing here goes into ${title}.`)
    // The lock names the model and its effort (D97), with no picker.
    const locked = view.locator('[data-locked-model]:visible')
    await expect(locked).toBeVisible()
    const label = (await locked.innerText()).trim()
    expect(label).toMatch(new RegExp(` · (${EFFORTS.join('|')})$`))
    await locked.click()
    await expect(page.locator('[data-locked-model-hint]')).toHaveText(
      "A side chat keeps its conversation's model and effort, to reuse its cache.",
    )
    await page.keyboard.press('Escape')

    // The textbox is named for screen readers; type and send, nothing else.
    const input = view.getByRole('textbox', { name: 'Ask aside…' })
    await expect(input).toBeVisible()
    await input.click()
    await page.keyboard.type(SIDE_MESSAGE)
    await page.keyboard.press('Enter')
    await expect(view).toContainText(SIDE_REPLY)
    await expect(view).not.toContainText(MESSAGE)
    await expect(page.locator('[data-side-chat-header]')).toContainText(
      SIDE_MESSAGE,
    )
    // The send named no model, provider or effort.
    const sideSend = sends.find((send) =>
      JSON.stringify(send.message ?? '').includes(SIDE_MESSAGE),
    )
    expect(sideSend).toBeTruthy()
    const options = (sideSend?.options ?? {}) as Record<string, unknown>
    for (const key of ['model', 'provider']) {
      expect(sideSend).not.toHaveProperty(key)
    }
    for (const key of [
      'thinking_level',
      'provider_options',
      'reasoning',
      'functions',
      'skills',
      'system_prompt',
      'system_prompt_strategy',
      'agent',
    ]) {
      expect(options).not.toHaveProperty(key)
    }

    // Nothing of the side chat in the conversation.
    await expect(transcript).not.toContainText(SIDE_REPLY)

    // Bring back: into the conversation's composer, unsent, and its draft.
    await view.locator('[data-bring-back]').first().click()
    await expect(page.locator('[data-side-chat-toast]')).toContainText(
      `Added to the ${title} composer`,
    )
    const mainComposer = transcript.getByRole('textbox', {
      name: 'message composer',
    })
    await expect(mainComposer).toContainText(
      `From side chat “${SIDE_MESSAGE}”:`,
    )
    await expect(mainComposer).toContainText(SIDE_REPLY)
    await expect
      .poll(
        async () => {
          const read = (await stack.invoke('session::get', {
            session_id: session,
          })) as { meta?: { draft?: string | null } }
          return read.meta?.draft ?? ''
        },
        { timeout: 15_000 },
      )
      .toBe(`From side chat “${SIDE_MESSAGE}”:\n\n${SIDE_REPLY}`)

    // The engine lists it, with what was said.
    await expect
      .poll(
        async () => {
          const [row] = await sideChats()
          return row && !row.running ? row.message_count : 0
        },
        { timeout: 30_000 },
      )
      .toBeGreaterThanOrEqual(2)
    const [made] = await sideChats()
    expect(made.title).toBe(SIDE_MESSAGE)
    expect(made.from_entry_id).toBeTruthy()

    // Back to the list: the Sidechats row, and the mark under the reply.
    await page.getByRole('button', { name: 'Back to context' }).click()
    const section = page.getByRole('region', { name: 'Sidechats' })
    await expect(section).toBeVisible()
    const row = section.locator(`[data-side-chat-row="${made.session_id}"]`)
    await expect(row).toContainText(SIDE_MESSAGE)
    await expect(section).toContainText(`${made.message_count} messages ·`)
    const mark = transcript.locator(
      `[data-side-chat-mark="${made.session_id}"]`,
    )
    await expect(mark).toHaveText(`Sidechat from here · ${SIDE_MESSAGE}`)
    // Beside the traces pane the context pane is an overlay drawer (D48):
    // put it away to reach the transcript; the mark brings it back.
    const paneButton = page.locator('[data-context-pane-button]')
    if ((await paneButton.getAttribute('aria-label')) === 'Hide context pane') {
      await paneButton.click()
    }
    await mark.click()
    await expect(
      page.locator(`[data-side-chat-view="${made.session_id}"]`),
    ).toContainText(SIDE_REPLY)
    await page.getByRole('button', { name: 'Back to context' }).click()

    // "+ New" forks at the last reply through the engine; left empty, it
    // is discarded with `close`, without asking.
    await section.locator('[data-side-chat-new]').click()
    await expect(page.locator('[data-side-chat-header]')).toContainText(
      'Side chat',
    )
    await expect
      .poll(async () => (await sideChats()).length, { timeout: 15_000 })
      .toBe(2)
    await page.getByRole('button', { name: 'Back to context' }).click()
    await expect
      .poll(async () => (await sideChats()).length, { timeout: 30_000 })
      .toBe(1)
    await expect(section.locator('[data-side-chat-row]')).toHaveCount(1)

    // The run's own check: the same system prompt, sections and tools,
    // the history prefix, nothing written into the conversation.
    const result = await stack.finish()
    expectPassingResult(result)
    expect(result.evidence?.generations_consumed).toBe(2)
  })
})
