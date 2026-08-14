import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { expect, type Page, test } from '@playwright/test'

const MODEL_ID = 'claude-sonnet-5'
const MODEL_LABEL = /claude[\s-]+sonnet[\s-]+5/i
const CAPABILITY_FUNCTION = 'shell::exec'
const CAPABILITY_OUTPUT_MARKER = 'HARNESS_FIRST_CAPABILITY_OUTPUT'
const RESPONSE_MARKER = 'HARNESS_FIRST_CAPABILITY_OK'
const PROMPT =
  `Use ${CAPABILITY_FUNCTION} exactly once to run printf with arguments ` +
  `["%s", "${CAPABILITY_OUTPUT_MARKER}"]. After the function succeeds, ` +
  `reply with exactly ${RESPONSE_MARKER} and nothing else.`

function required(name: string): string {
  const value = process.env[name]
  if (!value) throw new Error(`${name} is required`)
  return value
}

async function selectSonnet(page: Page) {
  const modelPicker = page.getByRole('button', { name: /^model(?::|\s|$)/i })
  await expect(modelPicker).toBeEnabled()
  await modelPicker.click()

  const anthropicGroup = page.getByRole('menuitem', { name: /^anthropic/i })
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
  await expect(modelPicker).toHaveAccessibleName(/^model:\s*claude[\s-]+sonnet[\s-]+5,/i)
}

test('completes and reloads the first real Harness capability', async ({ page }) => {
  const consoleUrl = required('HARNESS_QUICKSTART_CONSOLE_URL')
  const artifactsRoot = required('HARNESS_QUICKSTART_ARTIFACTS_DIR')

  await page.goto(consoleUrl)

  const chatTab = page.getByRole('tab', { name: /^chat \+ traces/i })
  await chatTab.click()
  await expect(chatTab).toHaveAttribute('aria-selected', 'true')

  await selectSonnet(page)

  const composer = page.getByLabel('message composer')
  await composer.pressSequentially(PROMPT)
  await expect(composer).toHaveText(PROMPT)
  await page.getByRole('button', { name: 'send message' }).click()

  const userMessage = page.locator('[data-message-role="user"]', { hasText: PROMPT })
  await expect(userMessage).toHaveCount(1)
  const chat = userMessage.locator('xpath=ancestor::section[@data-chat-session-id][1]')
  const sessionId = await chat.getAttribute('data-chat-session-id')
  if (!sessionId) throw new Error('Console did not expose the capability session id')
  const functionCard = chat.locator(`[data-function-id="${CAPABILITY_FUNCTION}"]`)
  await expect(functionCard).toHaveCount(1, { timeout: 90_000 })
  await expect(functionCard).toHaveAttribute('data-function-status', 'done', { timeout: 90_000 })

  const assistant = chat.locator('[data-message-role="assistant"]', {
    hasText: RESPONSE_MARKER,
  })
  await expect(assistant).toHaveCount(1, { timeout: 90_000 })
  await expect(assistant.locator(':scope > div')).toHaveText(RESPONSE_MARKER)
  await expect(assistant).toContainText(MODEL_ID)
  await expect(chat).toHaveAttribute('data-chat-session-hydrated', 'true')

  await page.reload()
  await page
    .getByRole('button', { name: /^open use shell::exec exactly once/i })
    .first()
    .click()
  const reloaded = page.locator(`[data-chat-session-id="${sessionId}"]`)
  await expect(reloaded).toHaveAttribute('data-chat-session-hydrated', 'true')
  await expect(reloaded.locator('[data-message-role="user"]', { hasText: PROMPT })).toHaveCount(1)
  const reloadedFunctionCard = reloaded.locator(`[data-function-id="${CAPABILITY_FUNCTION}"]`)
  await expect(reloadedFunctionCard).toHaveCount(1)
  await expect(reloadedFunctionCard).toHaveAttribute('data-function-status', 'done')
  await reloadedFunctionCard.locator('button[aria-expanded]').first().click()
  await expect(reloadedFunctionCard.locator('[data-function-pane="response"] code, .shui-pre.out code')).toContainText(
    CAPABILITY_OUTPUT_MARKER,
  )
  const reloadedAssistant = reloaded.locator('[data-message-role="assistant"]', { hasText: RESPONSE_MARKER })
  await expect(reloadedAssistant).toHaveCount(1)
  await expect(reloadedAssistant.locator(':scope > div')).toHaveText(RESPONSE_MARKER)
  await expect(reloadedAssistant).toContainText(MODEL_ID)

  await mkdir(artifactsRoot, { recursive: true })
  await writeFile(
    path.join(artifactsRoot, 'first-capability-browser-evidence.json'),
    `${JSON.stringify(
      {
        schema_version: 1,
        provider: 'anthropic',
        model: MODEL_ID,
        session_id: sessionId,
        prompt: PROMPT,
        response_marker: RESPONSE_MARKER,
        capability_function: CAPABILITY_FUNCTION,
        capability_output_marker: CAPABILITY_OUTPUT_MARKER,
        function_rendered: true,
        output_rendered_after_reload: true,
        persisted_after_reload: true,
      },
      null,
      2,
    )}\n`,
  )
})
