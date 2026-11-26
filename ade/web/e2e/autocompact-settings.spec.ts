import { expect, type Page, test } from '@playwright/test'

/*
 * chat-projects phase 3 in a browser, over the auto-compact harness's
 * in-memory engine (e2e/autocompact-harness): Settings › Chat ›
 * Auto-compact (§3.3.1), the meter's tick and label (§3.3.3) and the
 * transcript's notice and markers (§3.3.5). Run with
 * `pnpm test:e2e:autocompact`; nothing here reaches a live engine.
 */

declare global {
  interface Window {
    __AUTOCOMPACT_HARNESS__: {
      calls: { functionId: string; input: Record<string, unknown> }[]
      opened: string[]
      saved: { id: string; draft: Record<string, unknown> }[]
      rejectNext: { code: string; message: string } | null
    }
  }
}

const URL_PATH = '/e2e/autocompact-harness/index.html'

let sockets: string[] = []

test.beforeEach(async ({ page }) => {
  sockets = []
  page.on('websocket', (socket) => sockets.push(socket.url()))
})

test.afterEach(() => {
  // The harness has no engine: nothing may open a socket.
  expect(sockets).toEqual([])
})

const dialog = (page: Page) => page.locator('[data-autocompact-dialog]')

/** The patches written through `harness::chat-settings::set`, in order. */
async function settingsWrites(page: Page) {
  return page.evaluate(() =>
    window.__AUTOCOMPACT_HARNESS__.calls
      .filter((c) => c.functionId === 'harness::chat-settings::set')
      .map((c) => c.input.patch),
  )
}

async function openHarness(page: Page, query = '') {
  await page.goto(`${URL_PATH}${query}`)
  await expect(dialog(page)).toBeVisible()
  // The models and the log are read on open.
  await expect(dialog(page).locator('[data-autocompact-model]')).toHaveCount(2)
}

test('the transcript shows the notice verbatim and builds each marker from its record', async ({
  page,
}) => {
  await page.goto(`${URL_PATH}?closed`)
  const transcript = page.locator('[data-harness-transcript]')
  await expect(transcript).toContainText('Auto-compacting at 160k…')
  const markers = transcript.locator('[data-compaction-marker]')
  await expect(markers).toHaveText([
    'Auto-compacted at 160k · 161k → 34k tokens · acme-web rule',
    'Auto-compacted during the turn at 160k · 183k → 31k tokens · acme-web rule',
    "Compacted at the model's limit · 116k → 30k tokens",
  ])
  // /compact and a record from before phase 3 read as today.
  const today = transcript.locator(
    'article:not([data-compaction-trigger]):has(details)',
  )
  await expect(today).toHaveCount(2)
  await expect(today.nth(0)).toContainText('1,200 tokens')
  await expect(today.nth(1)).toContainText('98,000 tokens')
  // The summary stays one click away.
  await markers.first().click()
  await expect(transcript).toContainText(
    'The parser fix, the open TODOs and PR 412.',
  )
})

test('the meter carries the tick and "auto 400k"', async ({ page }) => {
  await page.goto(`${URL_PATH}?closed`)
  const meter = page.locator('[data-harness-meter] button').first()
  await expect(meter.locator('[data-ring-tick]')).toHaveCount(1)
  await expect(meter.locator('[data-autocompact-label]')).toHaveText(
    '·auto 400k',
  )
  await meter.click()
  await expect(page.locator('[data-autocompact-line]')).toHaveText(
    'Auto-compact at 400k · global rule',
  )
  await expect(page.locator('[data-slot="fact-meter-tick"]')).toBeVisible()
})

test('Settings › Chat › Auto-compact reads the engine and writes at once', async ({
  page,
}) => {
  await openHarness(page)
  const d = dialog(page)
  await expect(d).toContainText(
    "When a conversation's context passes this size at the end of a turn, the harness compacts it",
  )
  await expect(d.getByRole('switch')).toBeChecked()
  const field = d.locator('[name="autocompact-tokens"]')
  await expect(field).toHaveValue('400k')
  await expect(d.locator('[data-autocompact-model]').first()).toContainText(
    '200k window · 116k input budget',
  )
  await expect(d.locator('[data-autocompact-model]').first()).toContainText(
    'At its own limit, as today',
  )
  await expect(d.locator('[data-autocompact-model]').nth(1)).toContainText(
    'At 400k',
  )
  // A grey tick at each model's limit.
  await expect(d.locator('[data-slider-tick]')).toHaveCount(2)

  // A typed size, written on Enter.
  await field.fill('160k')
  await field.press('Enter')
  await expect
    .poll(() => settingsWrites(page))
    .toEqual([
      {
        autocompact: {
          mode: 'tokens',
          tokens: 160_000,
          instructions: 'Keep file paths and the PR numbers.',
        },
      },
    ])
  // 160k is past Opus's 116k budget, under Sonnet's 916k.
  await expect(d.locator('[data-autocompact-outcome]')).toHaveText([
    'At its own limit, as today',
    'At 160k',
  ])

  // Out of range: the field's error, nothing written.
  await field.fill('3M')
  await field.press('Enter')
  await expect(d.getByRole('alert')).toHaveText('Between 20k and 2M tokens')
  expect(await settingsWrites(page)).toHaveLength(1)

  // The slider, by keyboard: one step up from the shown size.
  await field.fill('160k')
  await field.press('Enter')
  await d.getByRole('slider').focus()
  await page.keyboard.press('ArrowRight')
  await expect
    .poll(async () => (await settingsWrites(page)).at(-1))
    .toEqual({
      autocompact: {
        mode: 'tokens',
        tokens: 170_000,
        instructions: 'Keep file paths and the PR numbers.',
      },
    })

  // Off keeps the size for the next time it is on.
  await d.getByRole('switch').click()
  await expect
    .poll(async () => (await settingsWrites(page)).at(-1))
    .toEqual({
      autocompact: {
        mode: 'off',
        tokens: 170_000,
        instructions: 'Keep file paths and the PR numbers.',
      },
    })
  await expect(field).toBeDisabled()
})

test("shows the harness's typed rejections under their fields", async ({
  page,
}) => {
  await openHarness(page)
  const d = dialog(page)
  await page.evaluate(() => {
    window.__AUTOCOMPACT_HARNESS__.rejectNext = {
      code: 'bad_instructions',
      message: 'Up to 1,000 characters',
    }
  })
  const summary = d.locator('[name="autocompact-instructions"]')
  await summary.fill('Keep the migration plan.')
  await d.getByRole('button', { name: 'Save' }).click()
  await expect(d.getByRole('alert')).toHaveText('Up to 1,000 characters')
  // The same text saves once the harness takes it.
  await d.getByRole('button', { name: 'Save' }).click()
  await expect
    .poll(async () => (await settingsWrites(page)).at(-1))
    .toEqual({
      autocompact: {
        mode: 'tokens',
        tokens: 400_000,
        instructions: 'Keep the migration plan.',
      },
    })
})

test('lists projects and sessions with their effective size, and opens them', async ({
  page,
}) => {
  await openHarness(page)
  const d = dialog(page)
  const rows = d.locator('tr[data-autocompact-override]')
  await expect(rows).toHaveCount(4)
  await expect(rows.nth(0)).toContainText('acme-web')
  await expect(rows.nth(0)).toContainText(
    '160k on Claude Sonnet 5.5 (Claude Code) · Claude Opus 5.5 (Claude Code) at its own limit',
  )
  await expect(rows.nth(1)).toContainText('Global (400k)')
  await expect(rows.nth(2)).toContainText("The model's limit only")
  await expect(rows.nth(3)).toContainText(
    'Ledger reconciliation · session in billing-api',
  )
  await expect(d.locator('[data-autocompact-sessions]')).toHaveText(
    '1 session sets their own value',
  )

  // A project opens its dialog on the Auto-compact section; its write is
  // the project's patch.
  await rows.nth(1).click()
  const section = page.locator('[data-project-autocompact]')
  await expect(section).toBeVisible()
  await section.locator('[data-autocompact-preset="200000"]').click()
  await page.getByRole('button', { name: 'Save' }).last().click()
  await expect(section).toBeHidden()
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__AUTOCOMPACT_HARNESS__.calls
          .filter((c) => c.functionId === 'harness::projects::update')
          .map((c) => c.input),
      ),
    )
    .toEqual([
      { id: 'prj_billing', autocompact: { mode: 'tokens', tokens: 200_000 } },
    ])
})

// The live acceptance at 1440×900 (env-p3): the long Effective text ran
// past the dialog's edge. Where and Effective wrap; nothing is clipped.
for (const viewport of [
  { width: 1440, height: 900 },
  { width: 700, height: 900 },
]) {
  test(`the overrides table fits the dialog at ${viewport.width}×${viewport.height}`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport)
    await openHarness(page)
    const d = dialog(page)
    await expect(d.locator('tr[data-autocompact-override]')).toHaveCount(4)
    const fit = await d
      .locator('[data-slot="table-frame"]')
      .evaluate((frame) => {
        const table = frame.querySelector('table')
        const box = frame.getBoundingClientRect()
        const cells = [
          ...frame.querySelectorAll(
            '[data-autocompact-effective], [data-autocompact-where]',
          ),
        ].map((cell) => cell.getBoundingClientRect())
        return {
          overflow: (table?.scrollWidth ?? 0) - frame.clientWidth,
          outside: cells.filter(
            (cell) => cell.left < box.left || cell.right > box.right + 0.5,
          ).length,
          wrapped: cells.some((cell) => cell.height > 24),
        }
      })
    expect(fit.overflow).toBeLessThanOrEqual(0)
    expect(fit.outside).toBe(0)
    // The long names wrap rather than run on.
    expect(fit.wrapped).toBe(true)
  })
}

test('a history row opens the conversation', async ({ page }) => {
  await openHarness(page)
  const d = dialog(page)
  const entry = d.locator('[data-autocompact-log="s-webhooks"]')
  await expect(entry).toContainText('Webhook retries')
  await expect(entry).toContainText('183k → 31k')
  await entry.click()
  await expect(d).toBeHidden()
  expect(
    await page.evaluate(() => window.__AUTOCOMPACT_HARNESS__.opened),
  ).toEqual(['s-webhooks'])
})

test('on a phone the overrides are a list and the dialog scrolls inside', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await openHarness(page)
  const d = dialog(page)
  await expect(d.locator('table')).toHaveCount(0)
  await expect(d.locator('button[data-autocompact-override]')).toHaveCount(4)
  const box = await d.boundingBox()
  expect(box?.width).toBeLessThanOrEqual(390)
  // The page body never scrolls sideways.
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - window.innerWidth,
  )
  expect(overflow).toBeLessThanOrEqual(0)
})
