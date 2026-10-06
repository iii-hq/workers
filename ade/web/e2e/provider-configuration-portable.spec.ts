import { readFile } from 'node:fs/promises'
import { expect, test } from '@playwright/test'
import { type ISdk, registerWorker } from 'iii-browser-sdk'

test.use({
  channel: process.env.CONSOLE_E2E_BROWSER_CHANNEL,
  video: 'off',
})

type ReadyManifest = {
  engine_url?: string
  console_url?: string
  configuration_id?: string
  configurationId?: string
  provider_id?: string
  providerId?: string
  configuration?: { id?: string }
  provider?: { id?: string }
}

type ConfigurationEntry = {
  id: string
  metadata?: Record<string, unknown>
}

type RawConfiguration = {
  id: string
  value: unknown
}

const readyFile = process.env.CONSOLE_E2E_READY_FILE ?? process.env.READY_FILE
const configuredProviderId = process.env.PROVIDER_ID
const aliasId = process.env.PROVIDER_E2E_ALIAS_ID ?? 'llm-router'

async function readReady(): Promise<ReadyManifest> {
  if (!readyFile) {
    throw new Error('CONSOLE_E2E_READY_FILE or READY_FILE is required')
  }
  return JSON.parse(await readFile(readyFile, 'utf8')) as ReadyManifest
}

async function triggerFromEngine(
  sdk: ISdk,
  functionId: string,
  payload: Record<string, unknown>,
): Promise<unknown> {
  return sdk.trigger({ function_id: functionId, payload })
}

async function runtime() {
  const ready = await readReady()
  const engineUrl = process.env.III_ENGINE_URL ?? ready.engine_url
  const consoleUrl = process.env.CONSOLE_E2E_URL ?? ready.console_url
  const configurationId =
    process.env.PROVIDER_CONFIGURATION_ID ??
    ready.configuration_id ??
    ready.configurationId ??
    ready.configuration?.id
  const providerId =
    configuredProviderId ??
    ready.provider_id ??
    ready.providerId ??
    ready.provider?.id
  if (!engineUrl || !consoleUrl) {
    throw new Error('ready manifest must provide engine_url and console_url')
  }
  if (!providerId) {
    throw new Error('PROVIDER_ID or provider id in ready manifest is required')
  }
  return { engineUrl, consoleUrl, configurationId, providerId }
}

async function listConfigurations(sdk: ISdk): Promise<ConfigurationEntry[]> {
  const response = (await triggerFromEngine(
    sdk,
    'configuration::list',
    {},
  )) as { configurations?: ConfigurationEntry[] }
  return response.configurations ?? []
}

function family(entry: ConfigurationEntry): string {
  const value = entry.metadata?.ui_form
  return typeof value === 'string' && value.trim() ? value.trim() : entry.id
}

async function resolveConfiguration(
  sdk: ISdk,
  requestedId?: string,
): Promise<string> {
  const entries = await listConfigurations(sdk)
  const matches = entries.filter(
    (entry) =>
      entry.id === requestedId ||
      entry.id === aliasId ||
      family(entry) === 'llm-router',
  )
  const unique = [
    ...new Map(matches.map((entry) => [entry.id, entry])).values(),
  ]
  if (unique.length !== 1) {
    throw new Error(
      'expected one live llm-router configuration, found ' +
        (unique.map((entry) => entry.id).join(', ') || 'none'),
    )
  }
  if (requestedId && unique[0].id !== requestedId) {
    throw new Error(
      'manifest configuration ' +
        requestedId +
        ' is not live id ' +
        unique[0].id,
    )
  }
  return unique[0].id
}

async function readRaw(sdk: ISdk, id: string): Promise<RawConfiguration> {
  return (await triggerFromEngine(sdk, 'configuration::get', {
    id,
    raw: true,
  })) as RawConfiguration
}

async function expectNotFound(
  sdk: ISdk,
  functionId: string,
  payload: Record<string, unknown>,
): Promise<void> {
  await expect(
    triggerFromEngine(sdk, functionId, payload),
  ).rejects.toMatchObject({ code: 'NOT_FOUND' })
}

function uniqueSaveUrl(): string {
  const configured =
    process.env.PROVIDER_E2E_SAVE_URL ?? 'https://provider-e2e.fixture.example'
  const suffix = `${Date.now()}-${process.pid}-${Math.random().toString(36).slice(2, 8)}`
  const separator = configured.includes('?') ? '&' : '?'
  return `${configured}${separator}provider_e2e_run=${suffix}`
}

async function openModelPicker(page: import('@playwright/test').Page) {
  const settings = page.getByRole('button', { name: 'chat settings' })
  if (await settings.count()) {
    await settings.click()
    await page.locator('[aria-labelledby="chat-settings-model"] button').click()
  } else {
    await page.locator('button[aria-label^="model"]:visible').first().click()
  }
}

test.describe('Provider configuration', () => {
  test('uses ModelPicker → Configure, saves and restores the namespaced raw value', async ({
    page,
  }) => {
    const live = await runtime()
    const sdk = registerWorker(live.engineUrl)
    let snapshot: RawConfiguration | undefined
    let cleanupError: unknown
    try {
      const liveId = await resolveConfiguration(sdk, live.configurationId)
      expect(liveId).not.toBe(aliasId)
      snapshot = await readRaw(sdk, liveId)
      const savedUrl = uniqueSaveUrl()

      await page.goto(live.consoleUrl)
      await expect(page).toHaveTitle(/iii Console/i)
      await openModelPicker(page)
      const providerGroup = page.locator(
        `[data-provider-group="${live.providerId}"]`,
      )
      await expect(providerGroup).toBeVisible()
      const configure = providerGroup.getByRole('button', {
        name: 'Configure',
        exact: true,
      })
      const configureProvider = providerGroup.getByRole('button', {
        name: 'Configure provider',
        exact: true,
      })
      if (await configure.count()) await configure.click()
      else await configureProvider.click()

      await expect(
        page.getByText('Credentials and provider-specific settings.', {
          exact: true,
        }),
      ).toBeVisible()
      await expect(
        page.getByRole('heading', { name: 'Provider settings', exact: true }),
      ).toBeVisible()
      const urlField = page.locator('input[inputmode="url"]')
      await expect(urlField).toBeVisible()
      await urlField.fill(savedUrl)
      const saveButton = page.getByRole('button', { name: /^save$/i })
      await expect(saveButton).toBeEnabled()
      await saveButton.click()
      await expect(saveButton).toHaveCount(0)

      const rawAfterSave = await readRaw(sdk, liveId)
      expect(rawAfterSave).toMatchObject({
        id: liveId,
        value: { providers: { [live.providerId]: { api_url: savedUrl } } },
      })
      await expectNotFound(sdk, 'configuration::get', {
        id: aliasId,
        raw: true,
      })
      const entriesAfterSave = await listConfigurations(sdk)
      expect(entriesAfterSave.map((entry) => entry.id)).toContain(liveId)
      expect(entriesAfterSave.map((entry) => entry.id)).not.toContain(aliasId)

      await page.reload()
      await openModelPicker(page)
      await page
        .locator(`[data-provider-group="${live.providerId}"]`)
        .getByRole('button', { name: /Configure/i })
        .first()
        .click()
      await expect(page.locator('input[inputmode="url"]')).toHaveValue(savedUrl)
    } catch (error) {
      cleanupError = error
    } finally {
      if (snapshot) {
        try {
          await triggerFromEngine(sdk, 'configuration::set', {
            id: snapshot.id,
            value: snapshot.value,
          })
          const restored = await readRaw(sdk, snapshot.id)
          expect(restored).toEqual(snapshot)
        } catch (error) {
          cleanupError ??= error
        }
      }
      try {
        await sdk.shutdown()
      } catch (error) {
        cleanupError ??= error
      }
    }
    if (cleanupError) throw cleanupError
  })
})
