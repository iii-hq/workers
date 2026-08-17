import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { expect, type Locator, type Page, test } from '@playwright/test'

const ANTHROPIC_MODEL_ID = 'claude-sonnet-5'
const ANTHROPIC_MODEL_LABEL = /claude[\s-]+sonnet[\s-]+5/i
const OPENAI_MODEL_ID = 'gpt-5.6-luna'
const OPENAI_MODEL_LABEL = /gpt[\s-]*5\.6[\s-]*luna/i

const ANTHROPIC_PROMPT =
  'Reply with exactly QUICKSTART_ANTHROPIC_OK and nothing else.'
const ANTHROPIC_MARKER = 'QUICKSTART_ANTHROPIC_OK'
const OPENAI_SWITCH_PROMPT =
  'Reply with exactly QUICKSTART_OPENAI_SWITCH_OK and nothing else.'
const OPENAI_SWITCH_MARKER = 'QUICKSTART_OPENAI_SWITCH_OK'
const OPENAI_NEW_CHAT_PROMPT =
  'Reply with exactly QUICKSTART_OPENAI_NEW_CHAT_OK and nothing else.'
const OPENAI_NEW_CHAT_MARKER = 'QUICKSTART_OPENAI_NEW_CHAT_OK'

function required(name: string): string {
  const value = process.env[name]
  if (!value) throw new Error(`${name} is required`)
  return value
}

async function selectModel(
  page: Page,
  provider: 'anthropic' | 'openai',
  modelLabel: RegExp,
  pickerLabel: RegExp,
) {
  const modelPicker = page.getByRole('button', { name: /^model(?::|\s|$)/i })
  await expect(modelPicker).toBeEnabled()
  await modelPicker.click()

  const providerGroup = page.getByRole('menuitem', {
    name: new RegExp(`^${provider}`, 'i'),
  })
  await expect(providerGroup).toBeVisible()
  if ((await providerGroup.getAttribute('aria-expanded')) !== 'true') {
    await providerGroup.click()
  }
  await expect(providerGroup).toHaveAttribute('aria-expanded', 'true')

  const model = page.getByRole('menuitemradio', { name: modelLabel })
  await expect(model).toHaveCount(1)
  await model.click()

  const defaultEffort = page.getByRole('menuitemradio', {
    name: 'default',
    exact: true,
  })
  if (await defaultEffort.isVisible().catch(() => false)) {
    await defaultEffort.click()
  } else {
    await page.keyboard.press('Escape')
  }
  await expect(modelPicker).toHaveAccessibleName(pickerLabel)
}

async function sendExactMessage(
  page: Page,
  chat: Locator,
  prompt: string,
  marker: string,
  modelId: string,
) {
  const composer = page.getByLabel('message composer')
  await composer.pressSequentially(prompt)
  await expect(composer).toHaveText(prompt)
  await page.getByRole('button', { name: 'send message' }).click()

  await expect(
    chat.locator('[data-message-role="user"]', { hasText: prompt }),
  ).toHaveCount(1)
  const assistant = chat.locator('[data-message-role="assistant"]', {
    hasText: marker,
  })
  await expect(assistant).toHaveCount(1)
  await expect(assistant.locator(':scope > div')).toHaveText(marker)
  await expect(assistant).toContainText(modelId)
}

async function persistedChat(
  page: Page,
  titlePrefix: RegExp,
  sessionId: string,
) {
  await page.getByRole('button', { name: titlePrefix }).first().click()
  const chat = page.locator(`[data-chat-session-id="${sessionId}"]`)
  await expect(chat).toHaveAttribute('data-chat-session-hydrated', 'true')
  return chat
}

test('switches from Sonnet 5 to Luna and starts a new Luna chat', async ({
  page,
}) => {
  const consoleUrl = required('HARNESS_QUICKSTART_CONSOLE_URL')
  const artifactsRoot = required('HARNESS_QUICKSTART_ARTIFACTS_DIR')

  await page.goto(consoleUrl)

  const chatTab = page.getByRole('tab', { name: /^chat \+ traces/i })
  await chatTab.click()
  await expect(chatTab).toHaveAttribute('aria-selected', 'true')

  await selectModel(
    page,
    'anthropic',
    ANTHROPIC_MODEL_LABEL,
    /^model:\s*claude[\s-]+sonnet[\s-]+5,/i,
  )

  const firstChat = page.locator('[data-chat-session-id]').first()
  const firstSessionId = await firstChat.getAttribute('data-chat-session-id')
  if (!firstSessionId)
    throw new Error('Console did not expose the persisted session id')
  await sendExactMessage(
    page,
    firstChat,
    ANTHROPIC_PROMPT,
    ANTHROPIC_MARKER,
    ANTHROPIC_MODEL_ID,
  )
  await expect(firstChat).toHaveAttribute('data-chat-session-hydrated', 'true')

  await selectModel(
    page,
    'openai',
    OPENAI_MODEL_LABEL,
    /^model:\s*gpt[\s-]*5\.6[\s-]*luna,/i,
  )
  await sendExactMessage(
    page,
    firstChat,
    OPENAI_SWITCH_PROMPT,
    OPENAI_SWITCH_MARKER,
    OPENAI_MODEL_ID,
  )
  await expect(firstChat).toHaveAttribute(
    'data-chat-session-id',
    firstSessionId,
  )

  await page.getByRole('button', { name: /^new chat$/i }).click()
  const secondChat = page.locator('[data-chat-session-id]').first()
  const secondSessionId = await secondChat.getAttribute('data-chat-session-id')
  if (!secondSessionId)
    throw new Error('Console did not expose the new persisted session id')
  expect(secondSessionId).not.toBe(firstSessionId)

  await selectModel(
    page,
    'openai',
    OPENAI_MODEL_LABEL,
    /^model:\s*gpt[\s-]*5\.6[\s-]*luna,/i,
  )
  await sendExactMessage(
    page,
    secondChat,
    OPENAI_NEW_CHAT_PROMPT,
    OPENAI_NEW_CHAT_MARKER,
    OPENAI_MODEL_ID,
  )
  await expect(secondChat).toHaveAttribute('data-chat-session-hydrated', 'true')

  await page.reload()
  const reloadedFirst = await persistedChat(
    page,
    /^open reply with exactly quickstart_an/i,
    firstSessionId,
  )
  await expect(
    reloadedFirst.locator('[data-message-role="user"]', {
      hasText: ANTHROPIC_PROMPT,
    }),
  ).toHaveCount(1)
  const reloadedAnthropic = reloadedFirst.locator(
    '[data-message-role="assistant"]',
    { hasText: ANTHROPIC_MARKER },
  )
  await expect(reloadedAnthropic).toHaveCount(1)
  await expect(reloadedAnthropic).toContainText(ANTHROPIC_MODEL_ID)
  await expect(
    reloadedFirst.locator('[data-message-role="user"]', {
      hasText: OPENAI_SWITCH_PROMPT,
    }),
  ).toHaveCount(1)
  const reloadedSwitch = reloadedFirst.locator(
    '[data-message-role="assistant"]',
    { hasText: OPENAI_SWITCH_MARKER },
  )
  await expect(reloadedSwitch).toHaveCount(1)
  await expect(reloadedSwitch).toContainText(OPENAI_MODEL_ID)

  const reloadedSecond = await persistedChat(
    page,
    /^open reply with exactly quickstart_op/i,
    secondSessionId,
  )
  await expect(
    reloadedSecond.locator('[data-message-role="user"]', {
      hasText: OPENAI_NEW_CHAT_PROMPT,
    }),
  ).toHaveCount(1)
  const reloadedNewChat = reloadedSecond.locator(
    '[data-message-role="assistant"]',
    { hasText: OPENAI_NEW_CHAT_MARKER },
  )
  await expect(reloadedNewChat).toHaveCount(1)
  await expect(reloadedNewChat).toContainText(OPENAI_MODEL_ID)

  await mkdir(artifactsRoot, { recursive: true })
  await writeFile(
    path.join(artifactsRoot, 'browser-evidence.json'),
    `${JSON.stringify(
      {
        schema_version: 2,
        provider: 'openai',
        model: OPENAI_MODEL_ID,
        marker: OPENAI_NEW_CHAT_MARKER,
        providers: ['anthropic', 'openai'],
        models: [ANTHROPIC_MODEL_ID, OPENAI_MODEL_ID],
        scenarios: {
          model_switch_same_chat: true,
          openai_new_chat: true,
        },
        sessions: [
          {
            session_id: firstSessionId,
            turns: [
              {
                provider: 'anthropic',
                model: ANTHROPIC_MODEL_ID,
                marker: ANTHROPIC_MARKER,
              },
              {
                provider: 'openai',
                model: OPENAI_MODEL_ID,
                marker: OPENAI_SWITCH_MARKER,
              },
            ],
          },
          {
            session_id: secondSessionId,
            turns: [
              {
                provider: 'openai',
                model: OPENAI_MODEL_ID,
                marker: OPENAI_NEW_CHAT_MARKER,
              },
            ],
          },
        ],
        persisted_after_reload: true,
      },
      null,
      2,
    )}\n`,
  )
})
