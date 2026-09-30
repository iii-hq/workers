import { readFile } from 'node:fs/promises'
import { expect, type Page, test } from '@playwright/test'

test.use({
  channel: process.env.CONSOLE_E2E_BROWSER_CHANNEL,
  video: 'off',
})

const providerId = process.env.PROVIDER_ID ?? 'openai-codex'
const visualDir = process.env.PROVIDER_E2E_VISUAL_DIR

type Ready = { console_url?: string; configuration_id?: string }

async function runtime() {
  const readyPath = process.env.CONSOLE_E2E_READY_FILE ?? process.env.READY_FILE
  const ready = readyPath
    ? (JSON.parse(await readFile(readyPath, 'utf8')) as Ready)
    : {}
  const consoleUrl = process.env.CONSOLE_E2E_URL ?? ready.console_url
  const configurationId =
    process.env.PROVIDER_CONFIGURATION_ID ?? ready.configuration_id
  if (!consoleUrl || !configurationId) {
    throw new Error('Console URL and live configuration id are required')
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

async function installLongSchemaFailure(page: Page, configurationId: string) {
  await page.evaluate(async (id) => {
    const longId = `missing-router-${'x'.repeat(180)}`
    const target = window as Window & {
      __providerConfigurationSchemaFailure?: boolean
      __providerConfigurationClientState?: {
        client: {
          trigger: (...args: never[]) => Promise<unknown>
        }
        original: (...args: never[]) => Promise<unknown>
      }
      __providerConfigurationLongId?: string
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
          throw new Error(`${longId}: isolated schema failure`)
        }
        return original(functionId, payload, options)
      }
      target.__providerConfigurationClientState = { client, original }
    }
    target.__providerConfigurationLongId = longId
    target.__providerConfigurationSchemaFailure = true
  }, configurationId)
}

async function restore(page: Page) {
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

async function openErrorPanel(
  page: Page,
  live: Awaited<ReturnType<typeof runtime>>,
  pickerSurface: 'sheet' | 'menu',
) {
  await installLongSchemaFailure(page, live.configurationId)
  await openModelPicker(page)
  if (pickerSurface === 'menu') {
    await expect(page.getByRole('menu')).toBeVisible()
  } else {
    await expect(page.locator('[role="dialog"]')).toBeVisible()
  }
  const group = page.locator(`[data-provider-group="${providerId}"]`)
  await expect(group).toBeVisible()
  await group
    .getByRole('button', { name: /Configure/i })
    .first()
    .click()
  await expect(
    page.getByText('Could not load provider configuration', { exact: true }),
  ).toBeVisible()
  await expect(
    page.getByRole('button', { name: 'Retry', exact: true }),
  ).toBeVisible()
}

async function assertScopedOverflow(page: Page) {
  const result = await page.evaluate(() => {
    const surfaces = [
      ...document.querySelectorAll<HTMLElement>('.configuration-surface'),
    ]
    const panels = surfaces.flatMap((surface) => [
      ...surface.querySelectorAll<HTMLElement>('[role="alert"]'),
    ])
    const roots = [...new Set([...surfaces, ...panels])]
    const offenders = roots.flatMap((root) =>
      [root, ...root.querySelectorAll<HTMLElement>('*')]
        .filter((element) => element.scrollWidth > element.clientWidth + 1)
        .map((element) => ({
          root: root.className,
          tag: element.tagName,
          className:
            typeof element.className === 'string' ? element.className : '',
          scrollWidth: element.scrollWidth,
          clientWidth: element.clientWidth,
        })),
    )
    return {
      surfaceCount: surfaces.length,
      statusPanelCount: panels.length,
      offenders,
    }
  })
  expect(result.surfaceCount).toBeGreaterThan(0)
  expect(result.statusPanelCount).toBeGreaterThan(0)
  expect(result.offenders, JSON.stringify(result)).toEqual([])
  return result
}

const visualCases: Array<{
  name: string
  width: number
  height: number
  theme: 'light' | 'dark'
  pickerSurface: 'sheet' | 'menu'
}> = [
  {
    name: 'phone-light',
    width: 390,
    height: 844,
    theme: 'light',
    pickerSurface: 'sheet',
  },
  {
    name: 'phone-dark',
    width: 390,
    height: 844,
    theme: 'dark',
    pickerSurface: 'sheet',
  },
  {
    name: 'desktop-app-sheet-light',
    width: 700,
    height: 900,
    theme: 'light',
    pickerSurface: 'sheet',
  },
  {
    name: 'desktop-app-sheet-dark',
    width: 700,
    height: 900,
    theme: 'dark',
    pickerSurface: 'sheet',
  },
  {
    name: 'desktop-wide-menu-light',
    width: 1440,
    height: 900,
    theme: 'light',
    pickerSurface: 'menu',
  },
  {
    name: 'desktop-wide-menu-dark',
    width: 1440,
    height: 900,
    theme: 'dark',
    pickerSurface: 'menu',
  },
]

for (const item of visualCases) {
  test(`${item.name}: long schema error fits panel and StatusPanel`, async ({
    page,
  }, testInfo) => {
    test.skip(
      !process.env.CONSOLE_E2E_URL && !process.env.CONSOLE_E2E_READY_FILE,
    )
    const live = await runtime()
    await page.setViewportSize({ width: item.width, height: item.height })
    await page.addInitScript((theme) => {
      localStorage.setItem('iii-theme', theme)
    }, item.theme)
    await page.goto(live.consoleUrl)
    await expect(page).toHaveTitle(/iii Console/i)
    try {
      await openErrorPanel(page, live, item.pickerSurface)
      await expect(page.locator('html')).toHaveAttribute(
        'data-theme',
        item.theme,
      )
      const overflow = await assertScopedOverflow(page)
      const output = visualDir ?? testInfo.outputDir
      await page.screenshot({
        path: `${output}/${item.name}-long-schema-error.png`,
        fullPage: true,
      })
      await testInfo.attach(`${item.name}-overflow.json`, {
        body: JSON.stringify(overflow, null, 2),
        contentType: 'application/json',
      })
    } finally {
      await restore(page)
    }
  })
}
