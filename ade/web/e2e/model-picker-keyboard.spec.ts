import { readFile } from 'node:fs/promises'
import { expect, type Page, test } from '@playwright/test'

test.use({
  channel: process.env.CONSOLE_E2E_BROWSER_CHANNEL,
  video: 'off',
})
type Ready = {
  console_url?: string
  engine_url?: string
  configuration_id?: string
  provider_id?: string
}

const providerId = process.env.PROVIDER_ID ?? 'openai-codex'

async function runtime() {
  const readyPath = process.env.CONSOLE_E2E_READY_FILE ?? process.env.READY_FILE
  const ready = readyPath
    ? (JSON.parse(await readFile(readyPath, 'utf8')) as Ready)
    : {}
  const consoleUrl = process.env.CONSOLE_E2E_URL ?? ready.console_url
  const configurationId =
    process.env.PROVIDER_CONFIGURATION_ID ?? ready.configuration_id
  if (!consoleUrl || !configurationId) {
    throw new Error(
      'CONSOLE_E2E_URL/CONSOLE_E2E_READY_FILE and configuration id are required',
    )
  }
  return { consoleUrl, configurationId }
}

async function openModelPicker(page: Page) {
  const settings = page.getByRole('button', { name: 'chat settings' })
  if (await settings.count()) {
    await settings.click()
    await page.locator('[aria-labelledby="chat-settings-model"] button').click()
  } else {
    await page.locator('button[aria-label^="model"]:visible').first().click()
  }
}

async function openProvider(
  page: Page,
  options: { expectHealthy?: boolean } = {},
) {
  await openModelPicker(page)
  const group = page.locator(`[data-provider-group="${providerId}"]`)
  await expect(group).toBeVisible()
  await group
    .getByRole('button', { name: /Configure/i })
    .first()
    .click()
  if (options.expectHealthy !== false) {
    await expect(
      page.getByText('Credentials and provider-specific settings.', {
        exact: true,
      }),
    ).toBeVisible()
    await expect(page.locator('input[inputmode="url"]')).toBeVisible()
  }
  return group
}

async function installSchemaFailure(page: Page, configurationId: string) {
  await page.evaluate(async (id) => {
    const target = window as Window & {
      __providerConfigurationSchemaFailure?: boolean
      __providerConfigurationClientState?: {
        client: {
          trigger: (
            functionId: string,
            payload?: Record<string, unknown>,
            options?: { timeoutMs?: number; namespace?: string },
          ) => Promise<unknown>
        }
        original: (
          functionId: string,
          payload?: Record<string, unknown>,
          options?: { timeoutMs?: number; namespace?: string },
        ) => Promise<unknown>
      }
    }
    const modulePath = '/src/lib/iii-client.ts'
    const { getIiiClient } = await import(/* @vite-ignore */ modulePath)
    const client = await getIiiClient()
    if (!target.__providerConfigurationClientState) {
      type Trigger = (
        functionId: string,
        payload?: Record<string, unknown>,
        options?: { timeoutMs?: number; namespace?: string },
      ) => Promise<unknown>
      const original: Trigger = client.trigger.bind(client) as Trigger
      client.trigger = async (
        functionId: string,
        payload?: Record<string, unknown>,
        options?: { timeoutMs?: number; namespace?: string },
      ) => {
        if (
          target.__providerConfigurationSchemaFailure &&
          functionId === 'configuration::schema' &&
          payload?.id === id
        ) {
          throw new Error(`isolated schema failure for ${id}`)
        }
        return original(functionId, payload, options)
      }
      target.__providerConfigurationClientState = { client, original }
    }
    target.__providerConfigurationSchemaFailure = true
  }, configurationId)
}

async function restoreSchemaFailure(page: Page) {
  await page.evaluate(() => {
    const target = window as Window & {
      __providerConfigurationSchemaFailure?: boolean
      __providerConfigurationClientState?: {
        client: { trigger: (...args: never[]) => Promise<unknown> }
        original: (...args: never[]) => Promise<unknown>
      }
    }
    target.__providerConfigurationSchemaFailure = false
    if (target.__providerConfigurationClientState) {
      target.__providerConfigurationClientState.client.trigger =
        target.__providerConfigurationClientState.original
    }
  })
}

async function activeElement(page: Page) {
  return page.evaluate(() => {
    const element = document.activeElement as HTMLElement | null
    return element
      ? {
          tag: element.tagName,
          role: element.getAttribute('role'),
          ariaLabel: element.getAttribute('aria-label'),
          inputMode: element.getAttribute('inputmode'),
          value: 'value' in element ? String(element.value) : '',
        }
      : null
  })
}

test.describe('Provider configuration ModelPicker keyboard journeys', () => {
  test('desktop keeps native Tab traversal, typing, Escape, dirty guard, and Back', async ({
    page,
  }) => {
    test.skip(
      !process.env.CONSOLE_E2E_URL && !process.env.CONSOLE_E2E_READY_FILE,
    )
    const live = await runtime()
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto(live.consoleUrl)
    await expect(page).toHaveTitle(/iii Console/i)
    const group = await openProvider(page)

    const afterConfigure = await activeElement(page)
    expect(afterConfigure?.ariaLabel).toBe('back to models')
    await page.keyboard.press('Tab')
    await expect(page.locator('input[inputmode="url"]')).toBeFocused()
    await page.keyboard.press('Tab')
    await expect(page.locator('input[inputmode="numeric"]')).toBeFocused()
    await page.keyboard.press('Shift+Tab')
    await expect(page.locator('input[inputmode="url"]')).toBeFocused()

    await page.keyboard.press('Control+A')
    await page.keyboard.type('https://typed-by-keyboard.example')
    await expect(page.locator('input[inputmode="url"]')).toHaveValue(
      'https://typed-by-keyboard.example',
    )
    expect((await activeElement(page))?.inputMode).toBe('url')

    await page.getByRole('button', { name: /^reset$/i }).click()
    await page.keyboard.press('Escape')
    await expect(
      page.getByText('Credentials and provider-specific settings.', {
        exact: true,
      }),
    ).toHaveCount(0)

    await openProvider(page)
    const url = page.locator('input[inputmode="url"]')
    await url.fill('https://dirty.example')
    page.once('dialog', async (dialog) => {
      expect(dialog.message()).toContain('discard unsaved changes')
      await dialog.dismiss()
    })
    await page.keyboard.press('Escape')
    await expect(url).toBeVisible()

    await page.getByRole('button', { name: /^reset$/i }).click()
    await page.getByRole('button', { name: 'back to models' }).click()
    await expect(group).toBeVisible()
  })

  test('desktop Retry is reachable with Tab+Enter after selective schema failure', async ({
    page,
  }) => {
    test.skip(
      !process.env.CONSOLE_E2E_URL && !process.env.CONSOLE_E2E_READY_FILE,
    )
    const live = await runtime()
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto(live.consoleUrl)
    await expect(page).toHaveTitle(/iii Console/i)
    await installSchemaFailure(page, live.configurationId)
    try {
      await openProvider(page, { expectHealthy: false })
      await expect(
        page.getByText('Could not load provider configuration', {
          exact: true,
        }),
      ).toBeVisible()
      await page.evaluate(() => {
        ;(
          window as Window & { __providerConfigurationSchemaFailure?: boolean }
        ).__providerConfigurationSchemaFailure = false
      })
      const retry = page.getByRole('button', { name: 'Retry', exact: true })
      let reached = false
      for (let index = 0; index < 8; index += 1) {
        if (
          await retry.evaluate((element) => element === document.activeElement)
        ) {
          reached = true
          break
        }
        await page.keyboard.press('Tab')
      }
      expect(reached).toBe(true)
      await page.keyboard.press('Enter')
      await expect(
        page.getByRole('heading', { name: 'Provider settings' }),
      ).toBeVisible()
      await expect(page.locator('input[inputmode="url"]')).toBeVisible()
    } finally {
      await restoreSchemaFailure(page)
    }
  })

  test('phone BottomSheet keeps Configure, Back, and Escape usable', async ({
    page,
  }) => {
    test.skip(
      !process.env.CONSOLE_E2E_URL && !process.env.CONSOLE_E2E_READY_FILE,
    )
    const live = await runtime()
    await page.setViewportSize({ width: 390, height: 844 })
    await page.goto(live.consoleUrl)
    await expect(page).toHaveTitle(/iii Console/i)
    const group = await openProvider(page)
    await expect(
      page.getByRole('button', { name: 'back to models' }),
    ).toBeFocused()
    await page.getByRole('button', { name: 'back to models' }).click()
    await expect(group).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(group).toBeHidden()
  })
})
