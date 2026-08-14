import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { expect, test } from '@playwright/test'

const MODEL_ID = 'claude-sonnet-5'
const MODEL_LABEL = /claude[\s-]+sonnet[\s-]+5/i
const PROMPT = 'Reply with exactly QUICKSTART_OK and nothing else.'
const MARKER = 'QUICKSTART_OK'

function required(name: string): string {
  const value = process.env[name]
  if (!value) throw new Error(`${name} is required`)
  return value
}

test('completes and reloads the first Sonnet 5 conversation', async ({
  page,
}) => {
  const consoleUrl = required('HARNESS_QUICKSTART_CONSOLE_URL')
  const artifactsRoot = required('HARNESS_QUICKSTART_ARTIFACTS_DIR')

  await page.goto(consoleUrl)

  const chatTab = page.getByRole('tab', { name: /^chat \+ traces/i })
  await chatTab.click()
  await expect(chatTab).toHaveAttribute('aria-selected', 'true')

  const modelPicker = page.getByRole('button', { name: /^model(?::|\s|$)/i })
  await expect(modelPicker).toBeEnabled()
  await modelPicker.click()

  const anthropicGroup = page.getByRole('menuitem', {
    name: /^anthropic/i,
  })
  await expect(anthropicGroup).toBeVisible()
  if ((await anthropicGroup.getAttribute('aria-expanded')) !== 'true') {
    await anthropicGroup.click()
  }
  await expect(anthropicGroup).toHaveAttribute('aria-expanded', 'true')

  const sonnet = page.getByRole('menuitemradio', { name: MODEL_LABEL })
  await expect(sonnet).toHaveCount(1)
  await sonnet.click()

  const defaultEffort = page.getByRole('menuitemradio', {
    name: 'default',
    exact: true,
  })
  if (await defaultEffort.isVisible().catch(() => false)) {
    await defaultEffort.click()
  } else {
    await page.keyboard.press('Escape')
  }
  await expect(modelPicker).toHaveAccessibleName(
    /^model:\s*claude[\s-]+sonnet[\s-]+5,/i,
  )

  const composer = page.getByLabel('message composer')
  await composer.pressSequentially(PROMPT)
  await expect(composer).toHaveText(PROMPT)
  await page.getByRole('button', { name: 'send message' }).click()

  await expect(
    page.locator('[data-message-role="user"]', { hasText: PROMPT }),
  ).toHaveCount(1)
  const assistant = page.locator('[data-message-role="assistant"]', {
    hasText: MARKER,
  })
  await expect(assistant).toHaveCount(1)
  await expect(assistant.locator(':scope > div')).toHaveText(MARKER)

  const chat = page.locator('[data-chat-session-id]').first()
  const sessionId = await chat.getAttribute('data-chat-session-id')
  if (!sessionId)
    throw new Error('Console did not expose the persisted session id')
  await expect(chat).toHaveAttribute('data-chat-session-hydrated', 'true')

  await page.reload()
  await page
    .getByRole('button', {
      name: /^open reply with exactly quickstart_ok/i,
    })
    .first()
    .click()
  const reloaded = page.locator(`[data-chat-session-id="${sessionId}"]`)
  await expect(reloaded).toHaveAttribute('data-chat-session-hydrated', 'true')
  await expect(
    reloaded.locator('[data-message-role="user"]', { hasText: PROMPT }),
  ).toHaveCount(1)
  const reloadedAssistant = reloaded.locator(
    '[data-message-role="assistant"]',
    { hasText: MARKER },
  )
  await expect(reloadedAssistant).toHaveCount(1)
  await expect(reloadedAssistant.locator(':scope > div')).toHaveText(MARKER)

  await mkdir(artifactsRoot, { recursive: true })
  await writeFile(
    path.join(artifactsRoot, 'browser-evidence.json'),
    `${JSON.stringify(
      {
        schema_version: 1,
        provider: 'anthropic',
        model: MODEL_ID,
        session_id: sessionId,
        marker: MARKER,
        persisted_after_reload: true,
      },
      null,
      2,
    )}\n`,
  )
})
