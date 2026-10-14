import { expect, type Locator, type Page, test } from '@playwright/test'

declare global {
  interface Window {
    __CLIPBOARD_HARNESS__?: {
      mode: 'success' | 'failure'
      writes: string[]
      fallbackCalls: string[]
    }
    __COLD_TOUCH_EVENTS__?: Record<string, unknown>[]
    __WORKERS_HARNESS__: {
      calls: {
        functionId: string
        payload: Record<string, unknown>
        options?: { namespace: string }
      }[]
      dirty: boolean | string
      saveMode: 'success' | 'error' | 'restart-error' | 'reload-error'
      saveDelay: number
      loadFailures: number
      snapshotMode: 'success' | 'error'
      restartMode: 'success' | 'error'
      readDelay: number
      savedCount: number
      commands: () => { id: string; bindings: string[]; enabled: boolean }[]
      runCommand: (id: string) => void
      externalEdit: (
        name: string,
        patch: {
          run?: string
          config_override?: string
          start_after?: string[]
          environment?: { key: string; value: string | null; secret: boolean }[]
        },
      ) => void
      readsCompleted: number
      configurationReferenceMode: 'none' | 'identity' | 'catalog' | 'error'
      configurationReferenceFailures: number
      editorSnapshot: () => Promise<{
        value: string
        position: { lineNumber: number; column: number } | null
        selection: {
          startLineNumber: number
          startColumn: number
          endLineNumber: number
          endColumn: number
        } | null
        scrollTop: number
        focused: boolean
      }>
    }
  }
}

const fixture = '/e2e/workers-harness/index.html'
const editedCommand = 'cargo run --locked --bin ade'
let browserErrors: string[] = []
let websocketUrls: string[] = []

test.beforeEach(async ({ page }) => {
  browserErrors = []
  websocketUrls = []
  page.on('pageerror', (error) => browserErrors.push(error.message))
  page.on('websocket', (socket) => websocketUrls.push(socket.url()))
  await page.addInitScript(() => window.localStorage.clear())
})

test.afterEach(() => {
  expect(
    browserErrors,
    'the real Workers component must render without browser errors',
  ).toEqual([])
  expect(
    websocketUrls,
    'the isolated fixture must never connect to a live engine',
  ).toEqual([])
})

async function selectWorker(page: Page, name = 'ade') {
  const worker = page
    .locator('button')
    .filter({
      has: page.locator('span', { hasText: new RegExp(`^${name}$`) }),
    })
    .first()
  if (!(await worker.isVisible())) {
    await page.getByRole('button', { name: 'Workers', exact: true }).click()
  }
  await worker.click()
}

async function openSettings(page: Page, parameters = '') {
  await page.goto(`${fixture}${parameters}`)
  await page
    .locator('[aria-current="page"]')
    .filter({ hasText: 'Project' })
    .waitFor()
  await selectWorker(page)
  await page.getByRole('tab', { name: /^Settings/ }).click()
  const command = page.locator('.wk-form input').first()
  await expect(command).toHaveValue('./target/debug/ade')
  return command
}

async function editCalls(page: Page) {
  return page.evaluate(() =>
    window.__WORKERS_HARNESS__.calls.filter(
      (call) => call.functionId === 'console::compose::edit',
    ),
  )
}

async function openPreviousSettings(page: Page) {
  await page.evaluate(
    () =>
      new Promise((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(resolve)),
      ),
  )
  const summary = page
    .locator('summary')
    .filter({ hasText: /^Previous saved settings$/ })
  const open = await summary.evaluateAll((elements) => {
    const details = elements[0]?.closest('details')
    return details?.hasAttribute('open') ?? null
  })
  if (open !== false) return
  const details = page.locator('details').filter({ has: summary })
  await summary.press('Enter')
  await expect(details).toHaveAttribute('open')
}

async function replaceYaml(page: Page, value: string) {
  const editor = page.getByRole('textbox', {
    name: /^(Configuration override YAML|config_override)$/,
  })
  await editor.focus()
  await editor.press('ControlOrMeta+A')
  await page.keyboard.insertText(value)
}

async function scrollSettingsBottom(page: Page) {
  await page.locator('.wk-main').evaluate((element) => {
    element.scrollTop = element.scrollHeight
  })
}

async function scrollRenderedLineIntoView(line: Locator) {
  // Monaco can replace its line node while applying a loaded model/highlight.
  // Reacquire it for this read-only scroll; the physical click still occurs once.
  await expect(async () => {
    await line.scrollIntoViewIfNeeded()
  }).toPass({ timeout: 8000 })
  await expect(line).toBeInViewport()
}

async function assertFieldUnobscured(page: Page) {
  const command = page.getByRole('textbox', { name: 'Command', exact: true })
  await expect(command).toBeInViewport()
  const field = await command.boundingBox()
  const bar = await page
    .getByRole('region', { name: 'Apply worker settings', exact: true })
    .boundingBox()
  if (!field || !bar)
    throw new Error('Command and its apply bar must be visible')
  expect(field.y).toBeGreaterThanOrEqual(bar.y + bar.height - 1)
}

async function textPresentation(locator: Locator) {
  return locator.evaluate(async (element) => {
    const parent = (node: Element): Element | null => {
      if (node.parentElement) return node.parentElement
      const root = node.getRootNode()
      return root instanceof ShadowRoot ? root.host : null
    }
    const animations: Animation[] = []
    for (let node: Element | null = element; node; node = parent(node)) {
      animations.push(
        ...node
          .getAnimations()
          .filter((animation) =>
            Number.isFinite(animation.effect?.getComputedTiming().endTime),
          ),
      )
    }
    await Promise.allSettled(animations.map((animation) => animation.finished))
    const box = element.getBoundingClientRect()
    const range = document.createRange()
    range.selectNodeContents(element)
    const textBounds = [...range.getClientRects()].map((rect) => ({
      left: rect.left,
      right: rect.right,
      top: rect.top,
      bottom: rect.bottom,
    }))
    const canvas = document.createElement('canvas')
    canvas.width = canvas.height = 1
    const context = canvas.getContext('2d')
    if (!context) throw new Error('Color measurements require a canvas')
    const color = (value: string): number[] => {
      context.clearRect(0, 0, 1, 1)
      context.fillStyle = value
      context.fillRect(0, 0, 1, 1)
      return [...context.getImageData(0, 0, 1, 1).data].map(
        (channel) => channel / 255,
      )
    }
    const blend = (front: number[], back: number[]): number[] => {
      const alpha = front[3] + back[3] * (1 - front[3])
      return [
        ...[0, 1, 2].map((index) =>
          alpha
            ? (front[index] * front[3] +
                back[index] * back[3] * (1 - front[3])) /
              alpha
            : 0,
        ),
        alpha,
      ]
    }
    const style = getComputedStyle(element)
    let foreground = color(style.color)
    let background = [0, 0, 0, 0]
    let opacity = 1
    for (let node: Element | null = element; node; node = parent(node)) {
      const current = getComputedStyle(node)
      const behind = color(current.backgroundColor)
      foreground = blend(foreground, behind)
      background = blend(background, behind)
      const alpha = Number.parseFloat(current.opacity)
      foreground[3] *= alpha
      background[3] *= alpha
      opacity *= alpha
    }
    foreground = blend(foreground, [1, 1, 1, 1])
    background = blend(background, [1, 1, 1, 1])
    const luminance = (rgb: number[]) => {
      const linear = rgb
        .slice(0, 3)
        .map((channel) =>
          channel <= 0.04045
            ? channel / 12.92
            : ((channel + 0.055) / 1.055) ** 2.4,
        )
      return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722
    }
    const light = luminance(foreground)
    const dark = luminance(background)
    return {
      connected: element.isConnected,
      width: box.width,
      height: box.height,
      left: box.left,
      right: box.right,
      top: box.top,
      bottom: box.bottom,
      clientWidth: element.clientWidth,
      scrollWidth: element.scrollWidth,
      clientHeight: element.clientHeight,
      scrollHeight: element.scrollHeight,
      textBounds,
      opacity,
      foreground,
      background,
      contrast: (Math.max(light, dark) + 0.05) / (Math.min(light, dark) + 0.05),
    }
  })
}

async function paintedTextLines(locator: Locator) {
  return locator.evaluate(async (element) => {
    await document.fonts.ready
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT)
    const painted: { center: number; text: string }[] = []
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const text = node.textContent ?? ''
      for (let index = 0; index < text.length; index++) {
        const range = document.createRange()
        range.setStart(node, index)
        range.setEnd(node, index + 1)
        const rect = range.getBoundingClientRect()
        if (!rect.width || !rect.height) continue
        const center = rect.top + rect.height / 2
        const line = painted.find(
          (candidate) => Math.abs(candidate.center - center) < 5,
        )
        if (line) line.text += text[index]
        else painted.push({ center, text: text[index] })
      }
    }
    return painted
      .sort((a, b) => a.center - b.center)
      .map((line) => line.text.trim().replace(/\s+/g, ' '))
  })
}

test('keeps the engine namespace readable at normal-text AA contrast in both themes', async ({
  page,
}) => {
  for (const theme of ['dark', 'light']) {
    for (const pane of [null, 720]) {
      await page.setViewportSize({ width: 1656, height: 1314 })
      const command = await openSettings(
        page,
        `?theme=${theme}${pane ? `&pane=${pane}` : ''}`,
      )
      await command.fill(editedCommand)
      const namespace = page
        .getByRole('banner')
        .getByText('iii-local', { exact: true })
      await expect(namespace).toBeVisible()
      const presentation = await textPresentation(namespace)
      expect
        .soft(
          presentation.contrast,
          `${theme} engine namespace effective text contrast`,
        )
        .toBeGreaterThanOrEqual(4.5)
      if (pane)
        await page.getByRole('button', { name: 'Workers', exact: true }).click()
      const selected = page
        .getByRole('complementary', { name: 'Workers', exact: true })
        .locator('[aria-current="page"]')
      await expect(selected).toContainText('ade')
      const samples = [
        {
          name: 'description',
          locator: selected.locator('.iii-ui-list-item__description'),
        },
        {
          name: 'unsaved',
          locator: selected.getByText('unsaved', { exact: true }),
        },
      ]
      for (const hover of [false, true]) {
        if (hover) await selected.hover()
        else await page.mouse.move(0, 0)
        const measured = []
        for (const sample of samples) {
          await expect(sample.locator).toBeVisible()
          const paint = await textPresentation(sample.locator)
          measured.push({ sample: sample.name, paint })
          expect
            .soft(
              paint.contrast,
              `${theme}/${pane ?? 'desktop'}/${hover ? 'selected-hover' : 'selected'} ${sample.name}`,
            )
            .toBeGreaterThanOrEqual(4.5)
        }
        const id = `selected-worker-${theme}-${pane ?? 'desktop'}-${hover ? 'hover' : 'rest'}`
        await test.info().attach(id, {
          body: JSON.stringify({ theme, pane, hover, measured }, null, 2),
          contentType: 'application/json',
        })
        const screenshot = test.info().outputPath(`${id}.png`)
        await page.screenshot({ path: screenshot })
        await test
          .info()
          .attach(id, { path: screenshot, contentType: 'image/png' })
      }
      if (theme === 'light' && pane === null) {
        await selectWorker(page, 'agent')
        await page.mouse.move(0, 0)
        const unselected = page
          .getByRole('complementary', { name: 'Workers', exact: true })
          .getByRole('button')
          .filter({ has: page.locator('span').filter({ hasText: /^ade$/ }) })
        await expect(unselected).not.toHaveAttribute('aria-current')
        const badge = unselected.getByText('unsaved', { exact: true })
        await expect(badge).toBeVisible()
        const paint = await textPresentation(badge)
        expect(paint.contrast).toBeGreaterThanOrEqual(4.5)
        await test.info().attach('unselected-worker-light-desktop', {
          body: JSON.stringify({ paint }, null, 2),
          contentType: 'application/json',
        })
      }
      expect(await editCalls(page)).toEqual([])
    }
  }
})

test('uses full-width drill-in settings in a fresh 720px pane while preserving navigation drafts and desktop resizing', async ({
  page,
}) => {
  test.setTimeout(90_000)
  const command = await openSettings(page, '?pane=720&commands=1')
  const settings = page.locator('.wk-settings')
  expect(
    (await textPresentation(settings)).width,
    '720px host must give Settings useful full-pane width',
  ).toBeGreaterThanOrEqual(650)
  await expect(
    page.getByRole('complementary', { name: 'Workers', exact: true }),
  ).toHaveCount(0)
  const back = page.getByRole('button', { name: 'Workers', exact: true })
  await expect(back).toBeVisible()
  await command.fill(editedCommand)
  await back.click()
  await expect(
    page.getByRole('searchbox', { name: 'Filter workers', exact: true }),
  ).toBeVisible()
  await selectWorker(page, 'agent')
  await selectWorker(page, 'ade')
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await expect(command).toHaveValue(editedCommand)
  expect(await editCalls(page)).toEqual([])

  await openSettings(page, '?commands=1')
  const sidebar = page.getByRole('complementary', {
    name: 'Workers',
    exact: true,
  })
  await expect(sidebar).toBeVisible()
  const before = await sidebar.boundingBox()
  const main = await page.locator('.wk-main').boundingBox()
  if (!before || !main)
    throw new Error('Desktop list and main pane must have visible geometry')
  expect(before.x + before.width).toBeLessThanOrEqual(main.x + 1)
  const resize = page.getByRole('separator', {
    name: 'resize Workers',
    exact: true,
  })
  await resize.focus()
  await resize.press('ArrowRight')
  await expect
    .poll(async () => (await sidebar.boundingBox())?.width)
    .toBeGreaterThan(before.width)
  await expect(
    page.getByRole('button', { name: 'Workers', exact: true }),
  ).toHaveCount(0)

  for (const width of [390, 450]) {
    for (const theme of ['dark', 'light']) {
      const height = width === 390 ? 844 : 900
      await page.setViewportSize({ width, height })
      const phoneCommand = await openSettings(page, `?theme=${theme}`)
      await page.evaluate(async () => {
        await document.fonts.ready
        await new Promise((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(resolve)),
        )
      })
      const summary = page
        .locator('summary')
        .filter({ hasText: /^Worker context$/ })
      const commandBox = await phoneCommand.boundingBox()
      if (!commandBox)
        throw new Error('The first native Settings input must render')
      await expect(phoneCommand).toHaveAccessibleName('Command')
      await expect(phoneCommand).toHaveAccessibleDescription('scripts.run')
      const commandRow = page.locator('.iii-ui-settings-row').filter({
        has: page.getByRole('textbox', { name: 'Command', exact: true }),
      })
      const commandLabel = commandRow
        .locator('label')
        .filter({ hasText: /^Command$/ })
      const commandDescription = commandRow.getByText('scripts.run', {
        exact: true,
      })
      const commandRowBox = await commandRow.boundingBox()
      if (!commandRowBox) throw new Error('The Command row must be measurable')
      const commandText = {
        label: await textPresentation(commandLabel),
        description: await textPresentation(commandDescription),
      }
      for (const [kind, paint] of Object.entries(commandText)) {
        expect(
          paint.connected,
          `${width}/${theme} Command ${kind} is connected`,
        ).toBe(true)
        expect(
          paint.height,
          `${width}/${theme} Command ${kind} is painted`,
        ).toBeGreaterThan(0)
        for (const bounds of paint.textBounds) {
          expect(
            bounds.left,
            `${width}/${theme} Command ${kind} fits its row`,
          ).toBeGreaterThanOrEqual(commandRowBox.x)
          expect(
            bounds.right,
            `${width}/${theme} Command ${kind} fits its row`,
          ).toBeLessThanOrEqual(commandRowBox.x + commandRowBox.width)
        }
      }
      expect(commandBox.height).toBeGreaterThanOrEqual(48)
      const commandCopy = await commandRow
        .getByRole('button', { name: 'Copy run command', exact: true })
        .boundingBox()
      expect(commandCopy?.width).toBeGreaterThanOrEqual(48)
      expect(commandCopy?.height).toBeGreaterThanOrEqual(48)
      const entrance = await page.evaluate(() => ({
        documentOverflow: document.documentElement.scrollWidth - innerWidth,
        scrollY,
        headerY: document.querySelector('.wk-header')?.getBoundingClientRect()
          .y,
        paneScroll: document.querySelector('.wk-main')?.scrollTop,
      }))
      await test.info().attach(`settings-entrance-${width}-${theme}`, {
        body: JSON.stringify(
          {
            commandBox,
            commandRowBox,
            commandText,
            commandCopy,
            viewport: { width, height },
            contextSummaries: await summary.count(),
            entrance,
          },
          null,
          2,
        ),
        contentType: 'application/json',
      })
      const screenshot = test
        .info()
        .outputPath(`settings-entrance-${width}-${theme}.png`)
      await page.screenshot({ path: screenshot })
      await test.info().attach(`settings-entrance-${width}-${theme}`, {
        path: screenshot,
        contentType: 'image/png',
      })
      expect
        .soft(
          await summary.count(),
          `${width}/${theme} Settings needs native Worker context`,
        )
        .toBe(1)
      expect
        .soft(
          commandBox.y,
          `${width}/${theme} first Settings field should enter before the bottom third`,
        )
        .toBeLessThanOrEqual((height * 2) / 3)
      expect(commandBox.y + commandBox.height).toBeLessThanOrEqual(height)
      expect(entrance).toMatchObject({
        documentOverflow: 0,
        scrollY: 0,
        headerY: 0,
        paneScroll: 0,
      })
      if ((await summary.count()) !== 1) continue
      const details = page.locator('details').filter({ has: summary })
      await expect(details).not.toHaveAttribute('open')
      await expect(page.locator('.wk-title')).toHaveText('ade')
      await expect(
        page.locator('.wk-title-row').getByText('ready', { exact: true }),
      ).toBeVisible()
      for (const action of ['Restart', 'Stop', 'More actions for ade'])
        await expect(
          page
            .locator('.wk-actions')
            .getByRole('button', { name: action, exact: true }),
        ).toBeInViewport()
      await expect(page.locator('.wk-meta')).toBeHidden()
      await expect(page.locator('.wk-relations')).toBeHidden()
      const size = await summary.boundingBox()
      if (!size) throw new Error('Native Worker context must be measurable')
      expect(size.height).toBeGreaterThanOrEqual(48)
      const summaryPresentation = await textPresentation(summary)
      expect(summaryPresentation.contrast).toBeGreaterThanOrEqual(4.5)
      await phoneCommand.fill(editedCommand)
      // Traverse from the actual input so the summary is reached in keyboard mode.
      for (
        let step = 0;
        step < 12 &&
        !(await summary.evaluate(
          (element) => document.activeElement === element,
        ));
        step++
      )
        await page.keyboard.press('Shift+Tab')
      await expect(summary).toBeFocused()
      const focus = await summary.evaluate((element) => {
        const rect = element.getBoundingClientRect()
        const hit = document.elementFromPoint(
          rect.x + rect.width / 2,
          rect.y + rect.height / 2,
        )
        const style = getComputedStyle(element)
        return {
          focused: document.activeElement === element,
          visible: element.matches(':focus-visible'),
          painted: hit === element || element.contains(hit),
          indicator:
            (parseFloat(style.outlineWidth) > 0 &&
              style.outlineStyle !== 'none') ||
            style.boxShadow !== 'none',
        }
      })
      expect(focus).toMatchObject({
        focused: true,
        visible: true,
        painted: true,
        indicator: true,
      })
      await test.info().attach(`settings-context-focus-${width}-${theme}`, {
        body: JSON.stringify({ size, summaryPresentation, focus }, null, 2),
        contentType: 'application/json',
      })
      await summary.press('Enter')
      await expect(details).toHaveAttribute('open')
      await expect(page.locator('.wk-meta')).toBeVisible()
      await expect(page.locator('.wk-relations')).toBeVisible()
      await summary.press('Space')
      await expect(details).not.toHaveAttribute('open')
      await expect(phoneCommand).toHaveValue(editedCommand)
      await page.getByRole('tab', { name: 'Log', exact: true }).click()
      await expect(summary).toHaveCount(0)
      await expect(page.locator('.wk-meta')).toBeVisible()
      await expect(page.locator('.wk-relations')).toBeVisible()
      await page.getByRole('tab', { name: /^Settings/ }).click()
      await expect(details).not.toHaveAttribute('open')
      await expect(phoneCommand).toHaveValue(editedCommand)
      await summary.focus()
      await summary.press('Enter')
      await page
        .locator('.wk-relations')
        .getByRole('button')
        .filter({ hasText: /^state/ })
        .click()
      await expect(page.locator('.wk-title')).toHaveText('state')
      await selectWorker(page, 'ade')
      await page.getByRole('tab', { name: /^Settings/ }).click()
      await expect(details).not.toHaveAttribute('open')
      await expect(phoneCommand).toHaveValue(editedCommand)
      await page.setViewportSize({ width: 1656, height: 1314 })
      await expect(summary).toHaveCount(0)
      await expect(page.locator('.wk-meta')).toBeVisible()
      await expect(page.locator('.wk-relations')).toBeVisible()
      await expect(phoneCommand).toHaveValue(editedCommand)
      await page.setViewportSize({ width, height })
      await expect(details).not.toHaveAttribute('open')
      await expect(phoneCommand).toHaveValue(editedCommand)
      expect(
        await page.evaluate(() =>
          window.__WORKERS_HARNESS__.calls.filter((call) =>
            [
              'console::compose::edit',
              'compose::restart',
              'compose::start',
              'compose::stop',
            ].includes(call.functionId),
          ),
        ),
      ).toEqual([])
      expect(await page.evaluate(() => scrollY)).toBe(0)
      const save = page.getByRole('button', {
        name: 'Save and restart',
        exact: true,
      })
      await save.click()
      await expect(
        page.getByText('Settings saved for ade', { exact: true }).first(),
      ).toBeVisible()
      await expect(save).toBeDisabled()
      await page.locator('.wk-main').evaluate((element) => {
        element.scrollTop = 0
      })
      await page.evaluate(async () => {
        window.scrollTo(0, 0)
        await document.fonts.ready
        await new Promise((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(resolve)),
        )
      })
      const savedCommandBox = await phoneCommand.boundingBox()
      if (!savedCommandBox)
        throw new Error(
          'The first input must render after establishing history',
        )
      const historySummary = page
        .locator('summary')
        .filter({ hasText: /^Previous saved settings$/ })
      const savedEntrance = await page.evaluate(() => ({
        scrollY,
        headerY: document.querySelector('.wk-header')?.getBoundingClientRect()
          .y,
        paneScroll: document.querySelector('.wk-main')?.scrollTop,
        documentOverflow: document.documentElement.scrollWidth - innerWidth,
      }))
      const savedId = `saved-settings-entrance-${width}-${theme}`
      await test.info().attach(savedId, {
        body: JSON.stringify(
          {
            commandBox: savedCommandBox,
            savedEntrance,
            historySummaries: await historySummary.count(),
          },
          null,
          2,
        ),
        contentType: 'application/json',
      })
      const savedScreenshot = test.info().outputPath(`${savedId}.png`)
      await page.screenshot({ path: savedScreenshot })
      await test
        .info()
        .attach(savedId, { path: savedScreenshot, contentType: 'image/png' })
      expect
        .soft(
          savedCommandBox.y,
          `${savedId}: Command starts inside the first viewport`,
        )
        .toBeGreaterThanOrEqual(0)
      expect
        .soft(
          savedCommandBox.y + savedCommandBox.height,
          `${savedId}: the complete Command stays in the first viewport after save`,
        )
        .toBeLessThanOrEqual(height)
      expect
        .soft(
          await historySummary.count(),
          `${savedId}: history recovery remains discoverable as a native disclosure`,
        )
        .toBe(1)
      expect(savedEntrance).toEqual({
        scrollY: 0,
        headerY: 0,
        paneScroll: 0,
        documentOverflow: 0,
      })
      const restore = page.getByRole('button', {
        name: 'Restore previous settings',
        exact: true,
      })
      if ((await historySummary.count()) === 1) {
        const historyDetails = page
          .locator('details')
          .filter({ has: historySummary })
        await expect(historyDetails).not.toHaveAttribute('open')
        await save.focus()
        for (
          let step = 0;
          step < 12 &&
          !(await historySummary.evaluate(
            (element) => document.activeElement === element,
          ));
          step++
        )
          await page.keyboard.press('Tab')
        await expect(historySummary).toBeFocused()
        const historyBox = await historySummary.boundingBox()
        expect(historyBox?.height).toBeGreaterThanOrEqual(48)
        await assertSettingsFocusPaint(page, `${savedId}: history discovery`)
        await historySummary.press('Enter')
        await expect(historyDetails).toHaveAttribute('open')
        await page.keyboard.press('Tab')
        await expect(restore).toBeFocused()
        expect((await restore.boundingBox())?.height).toBeGreaterThanOrEqual(48)
        await assertSettingsFocusPaint(page, `${savedId}: Restore action`)
      }
      const newerCommand = `${editedCommand} --pending`
      await phoneCommand.fill(newerCommand)
      await restore.focus()
      await restore.press('Enter')
      const confirmation = page.getByRole('alertdialog')
      await expect(confirmation).toBeVisible()
      await confirmation
        .getByRole('button', { name: 'Cancel', exact: true })
        .press('Enter')
      await expect(phoneCommand).toHaveValue(newerCommand)
      await restore.focus()
      await restore.press('Enter')
      await confirmation
        .getByRole('button', { name: 'Restore draft', exact: true })
        .press('Enter')
      const restoredReview = page.getByRole('region', {
        name: 'Review pending changes',
        exact: true,
      })
      await expect(restoredReview).toBeFocused()
      await expect(phoneCommand).toHaveValue('./target/debug/ade')
      await expect(
        page.getByLabel('API_TOKEN value', { exact: true }),
      ).toHaveCount(0)
      await expect(restoredReview).toContainText(
        'Saved secrets stay as they are now unless explicitly replaced',
      )
      await page.getByRole('button', { name: 'Discard', exact: true }).click()
      await expect(phoneCommand).toHaveValue(editedCommand)
      const savedWrites = await editCalls(page)
      expect(savedWrites).toHaveLength(1)
      expect(savedWrites[0].payload).toMatchObject({
        container: 'ade',
        run: editedCommand,
      })
      expect(savedWrites[0].payload).not.toHaveProperty('environment')
      expect(
        await page.evaluate(() =>
          window.__WORKERS_HARNESS__.calls.filter((call) =>
            ['compose::restart', 'compose::up', 'compose::down'].includes(
              call.functionId,
            ),
          ),
        ),
      ).toEqual([])
      expect(await page.evaluate(() => scrollY)).toBe(0)
    }
  }
})

test('undoes a discarded invalid worker draft without writing it', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 1000 })
  const command = await openSettings(page, '?commands=1&stoppedWorkers=1')
  const setYaml = async (value: string) => {
    await expect(page.locator('.wk-code .monaco-editor')).toBeVisible()
    const editor = page.getByRole('textbox', {
      name: 'Configuration override YAML',
      exact: true,
    })
    await editor.focus()
    await editor.press('ControlOrMeta+A')
    await page.keyboard.type(value)
    if (value.endsWith('[')) await page.keyboard.press('Delete')
    await expect
      .poll(
        async () =>
          (
            await page.evaluate(() =>
              window.__WORKERS_HARNESS__.editorSnapshot(),
            )
          ).value,
      )
      .toBe(value)
  }
  await command.fill(editedCommand)
  await page.getByLabel('LOG_LEVEL value', { exact: true }).fill('debug')
  const secret = 'r10-secret-only-in-pending-memory'
  await page.getByRole('button', { name: 'Replace', exact: true }).click()
  await page.getByLabel('API_TOKEN value', { exact: true }).fill(secret)
  await page.getByRole('button', { name: 'Add variable', exact: true }).click()
  await expect(
    page.getByRole('textbox', { name: 'Variable name', exact: true }),
  ).toHaveValue('')
  await setYaml('port: [')
  await expect(
    page.getByRole('textbox', {
      name: 'Configuration override YAML',
      exact: true,
    }),
  ).toHaveAttribute('aria-invalid', 'true')
  await page.getByRole('button', { name: 'Discard', exact: true }).click()
  await expect(command).not.toHaveValue(editedCommand)
  await expect(page.getByLabel('API_TOKEN value', { exact: true })).toHaveCount(
    0,
  )
  await expect(
    page.getByRole('textbox', { name: 'Variable name', exact: true }),
  ).toHaveCount(0)
  const undo = page.getByRole('button', { name: 'Undo discard', exact: true })
  await expect(undo).toBeVisible()
  expect((await textPresentation(undo)).height).toBeGreaterThanOrEqual(48)
  expect(
    await page.evaluate(
      () =>
        window.__WORKERS_HARNESS__
          .commands()
          .find((command) => command.id === 'undo-discard')?.enabled,
    ),
  ).toBe(true)
  await selectWorker(page, 'agent')
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await expect(undo).toHaveCount(0)
  await command.fill('agent discarded draft')
  await page.getByRole('button', { name: 'Discard', exact: true }).click()
  await selectWorker(page, 'ade')
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await undo.focus()
  await undo.press('Enter')
  await expect(command).toHaveValue(editedCommand)
  await expect(page.getByLabel('LOG_LEVEL value', { exact: true })).toHaveValue(
    'debug',
  )
  expect(
    (await page.evaluate(() => window.__WORKERS_HARNESS__.editorSnapshot()))
      .value,
  ).toBe('port: [')
  await expect(page.getByLabel('API_TOKEN value', { exact: true })).toHaveValue(
    secret,
  )
  await expect(
    page.getByLabel('API_TOKEN value', { exact: true }),
  ).toHaveAttribute('type', 'password')
  await expect(
    page.getByRole('textbox', { name: 'Variable name', exact: true }),
  ).toHaveValue('')
  await expect(
    page.getByRole('region', { name: 'Worker settings result', exact: true }),
  ).toBeFocused()
  await expect(undo).toHaveCount(0)
  await expect(
    page.getByRole('button', { name: 'Save and restart', exact: true }),
  ).toBeDisabled()
  expect(await editCalls(page)).toEqual([])
  await selectWorker(page, 'agent')
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await undo.click()
  await expect(command).toHaveValue('agent discarded draft')
  await expect(
    page.getByRole('region', { name: 'Review pending changes', exact: true }),
  ).toBeFocused()
  await selectWorker(page, 'ade')
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await expect(command).toHaveValue(editedCommand)
  await page.getByRole('button', { name: 'Discard', exact: true }).click()
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.externalEdit('ade', {
      run: 'cargo run --locked --bin ade --external',
      config_override: 'port: 3115',
      environment: [
        { key: 'III_PORT', value: '3999', secret: false },
        { key: 'LOG_LEVEL', value: 'info', secret: false },
        { key: 'API_TOKEN', value: null, secret: true },
        { key: 'EXTERNAL_MODE', value: 'from-other-client', secret: false },
      ],
    })
    window.__WORKERS_HARNESS__.runCommand('refresh')
  })
  await expect(command).toHaveValue('cargo run --locked --bin ade --external')
  await expect(page.getByLabel('III_PORT value', { exact: true })).toHaveValue(
    '3999',
  )
  await command.fill('newer current draft')
  await undo.click()
  const confirmation = page.getByRole('alertdialog')
  await expect(confirmation).toContainText('Replace the current draft?')
  await confirmation
    .getByRole('button', { name: 'Cancel', exact: true })
    .click()
  await expect(command).toHaveValue('newer current draft')
  await expect(undo).toBeVisible()
  await page.evaluate(() =>
    window.__WORKERS_HARNESS__.runCommand('undo-discard'),
  )
  await confirmation
    .getByRole('button', { name: 'Undo discard', exact: true })
    .click()
  await expect(command).toHaveValue(editedCommand)
  await expect(page.getByLabel('LOG_LEVEL value', { exact: true })).toHaveValue(
    'debug',
  )
  await expect(page.getByLabel('III_PORT value', { exact: true })).toHaveValue(
    '3999',
  )
  await expect(
    page.getByLabel('EXTERNAL_MODE value', { exact: true }),
  ).toHaveValue('from-other-client')
  await expect(page.getByLabel('API_TOKEN value', { exact: true })).toHaveValue(
    secret,
  )
  expect(
    (await page.evaluate(() => window.__WORKERS_HARNESS__.editorSnapshot()))
      .value,
  ).toBe('port: [')
  expect(await editCalls(page)).toEqual([])
  await setYaml('port: 3114')
  await page
    .getByRole('textbox', { name: 'Variable name', exact: true })
    .fill('RESTORED_VARIABLE')
  await page
    .getByRole('button', { name: 'Preview changes', exact: true })
    .click()
  const review = page.getByRole('region', {
    name: 'Review pending changes',
    exact: true,
  })
  await expect(review).not.toContainText(secret)
  await expect(review).toContainText('# new value')
  await page.getByRole('button', { name: 'Discard', exact: true }).click()
  await expect(undo).toBeVisible()
  await command.fill('cargo run --locked --bin ade --final')
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await expect(confirmation).toBeVisible()
  await confirmation
    .getByRole('button', { name: 'Cancel', exact: true })
    .click()
  expect(await editCalls(page)).toEqual([])
  await expect(undo).toBeVisible()
  await expect(command).toHaveValue('cargo run --locked --bin ade --final')
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await confirmation
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await expect(
    page.getByRole('region', { name: 'Worker settings result', exact: true }),
  ).toContainText('Settings saved for ade')
  await expect(undo).toHaveCount(0)
  expect(
    await page.evaluate(
      () =>
        window.__WORKERS_HARNESS__
          .commands()
          .find((command) => command.id === 'undo-discard')?.enabled,
    ),
  ).toBe(false)
  expect(await editCalls(page)).toHaveLength(1)
  expect((await editCalls(page))[0].payload).toMatchObject({
    run: 'cargo run --locked --bin ade --final',
  })
  expect(JSON.stringify((await editCalls(page))[0].payload)).not.toContain(
    secret,
  )
  expect(await page.evaluate(() => window.scrollY)).toBe(0)
})

test('keeps meaningful native Monaco YAML and JSON punctuation at AA contrast in both themes', async ({
  page,
}) => {
  for (const theme of ['dark', 'light']) {
    for (const language of ['yaml', 'json']) {
      if (language === 'json') {
        await page.goto(`${fixture}?theme=${theme}&monacoCase=json`)
      } else {
        await openSettings(page, `?theme=${theme}`)
        await expect(page.locator('.wk-code .monaco-editor')).toBeVisible()
        const editor = page.getByRole('textbox', {
          name: 'Configuration override YAML',
          exact: true,
        })
        await editor.focus()
        await editor.press('ControlOrMeta+A')
        await page.keyboard.type('port: [3113, 3114]')
        await expect
          .poll(
            async () =>
              (
                await page.evaluate(() =>
                  window.__WORKERS_HARNESS__.editorSnapshot(),
                )
              ).value,
          )
          .toBe('port: [3113, 3114]')
      }
      await expect(page.locator('.wk-code .monaco-editor')).toBeVisible()
      const punctuation = page
        .locator('.wk-code .view-line span:not(:has(span))')
        .filter({ hasText: /^[\s:{}[\],]+$/ })
        .filter({ hasText: /[:{}[\],]/ })
      const text = (await punctuation.allTextContents()).join('')
      for (const symbol of language === 'json'
        ? [':', ',', '[', ']', '{', '}']
        : [':', ',', '[', ']'])
        expect(text).toContain(symbol)
      for (let index = 0; index < (await punctuation.count()); index++) {
        const token = punctuation.nth(index)
        await expect(token).toBeVisible()
        let presentation:
          | Awaited<ReturnType<typeof textPresentation>>
          | undefined
        await expect(async () => {
          const measured = await textPresentation(token)
          expect(measured.connected).toBe(true)
          expect(measured.width).toBeGreaterThan(0)
          expect(measured.height).toBeGreaterThan(0)
          expect(Number.isFinite(measured.opacity)).toBe(true)
          presentation = measured
        }).toPass()
        if (!presentation) throw new Error('Native punctuation did not settle')
        expect
          .soft(
            presentation.contrast,
            `${theme} native ${language} punctuation ${await token.textContent()}: ${JSON.stringify(presentation)}`,
          )
          .toBeGreaterThanOrEqual(4.5)
      }
      expect(await editCalls(page)).toEqual([])
    }
  }
})

test.describe('touch worker controls', () => {
  test.use({ hasTouch: true })

  test('keeps the phone Settings verification state accessible without clipping its painted tab text', async ({
    page,
  }) => {
    for (const width of [390, 450]) {
      await page.setViewportSize({ width, height: 844 })
      const command = await openSettings(
        page,
        '?configurationReference=identity',
      )
      await expect(page.locator('.wk-tabs [role="tab"]')).toHaveCount(4)
      await page.evaluate(() => {
        window.__WORKERS_HARNESS__.saveMode = 'reload-error'
      })
      await command.fill(editedCommand)
      await page
        .getByRole('button', { name: 'Save and restart', exact: true })
        .click()
      const tab = page.getByRole('tab', { name: /^Settings/ })
      await expect(tab).toHaveAccessibleName(/Settings.*awaiting verification/i)
      await expect(
        page.getByRole('region', {
          name: 'Apply worker settings',
          exact: true,
        }),
      ).toContainText('Saved; awaiting verification')
      await expect(
        page.getByRole('region', {
          name: 'Worker settings result',
          exact: true,
        }),
      ).toContainText('Settings saved; declaration could not be reloaded')
      const assertPaint = async (state: string) => {
        await page.evaluate(() => document.fonts.ready)
        await textPresentation(tab)
        const paint = await tab.evaluate((element) => {
          const box = element.getBoundingClientRect()
          const label = element.firstElementChild
          const labelRange = document.createRange()
          if (label) labelRange.selectNodeContents(label)
          const labelContentWidth = label
            ? labelRange.getBoundingClientRect().width
            : 0
          const labelWidth = label?.getBoundingClientRect().width ?? 0
          const walker = document.createTreeWalker(
            element,
            NodeFilter.SHOW_TEXT,
          )
          const overflow: { text: string; left: number; right: number }[] = []
          const clipped: {
            text: string
            textWidth: number
            availableWidth: number
          }[] = []
          let visible = ''
          for (let node = walker.nextNode(); node; node = walker.nextNode()) {
            if (!node.textContent?.trim()) continue
            let hidden = false
            for (
              let parent = node.parentElement;
              parent && parent !== element;
              parent = parent.parentElement
            ) {
              const style = getComputedStyle(parent)
              if (
                parent.classList.contains('sr-only') ||
                style.display === 'none' ||
                style.visibility === 'hidden' ||
                style.clipPath !== 'none' ||
                (style.clip !== 'auto' && style.clip !== '')
              )
                hidden = true
            }
            if (hidden) continue
            const range = document.createRange()
            range.selectNodeContents(node)
            const textWidth = range.getBoundingClientRect().width
            const availableWidth =
              node.parentElement?.getBoundingClientRect().width ?? 0
            if (textWidth > availableWidth + 1)
              clipped.push({
                text: node.textContent ?? '',
                textWidth,
                availableWidth,
              })
            for (const rect of range.getClientRects()) {
              if (!rect.width || !rect.height) continue
              visible += node.textContent
              if (rect.left < box.left - 1 || rect.right > box.right + 1)
                overflow.push({
                  text: node.textContent,
                  left: rect.left,
                  right: rect.right,
                })
            }
          }
          return {
            overflow,
            clipped,
            visible,
            width: box.width,
            labelWidth,
            labelContentWidth,
          }
        })
        expect(paint.visible).toContain('Settings')
        expect(
          paint.overflow,
          `phone tab width ${paint.width} must contain every visible text run`,
        ).toEqual([])
        expect
          .soft(
            paint.clipped,
            `${width}px four-tab Settings label must paint in full: ${JSON.stringify(paint)}`,
          )
          .toEqual([])
        expect
          .soft(
            paint.labelContentWidth,
            `${width}px ${state} full verification label/clock must fit its clipping box: ${JSON.stringify(paint)}`,
          )
          .toBeLessThanOrEqual(paint.labelWidth + 1)
      }
      await assertPaint('saved baseline')
      await command.fill(`${editedCommand} --later`)
      await expect(tab).toHaveAccessibleName(
        /Settings.*1.*awaiting verification/i,
      )
      await expect(
        page.getByRole('region', {
          name: 'Apply worker settings',
          exact: true,
        }),
      ).toContainText('1 pending change')
      await expect(
        page.getByRole('button', { name: 'Save and restart', exact: true }),
      ).toBeEnabled()
      await assertPaint('newer pending draft')
      await command.fill(editedCommand)
      await page
        .getByRole('button', { name: 'Add variable', exact: true })
        .click()
      await expect(tab).toHaveAccessibleName(
        /Settings.*unsaved.*awaiting verification/i,
      )
      await expect(
        page.getByRole('button', { name: 'Save and restart', exact: true }),
      ).toBeDisabled()
      await expect(
        page.getByRole('button', {
          name: 'Fix 1 field before saving',
          exact: true,
        }),
      ).toBeVisible()
      await assertPaint('invalid unsaved draft')
      expect(await editCalls(page)).toHaveLength(1)
    }
  })

  test('keeps worker relationship navigation pills reachable at phone touch sizes without overlap', async ({
    page,
  }) => {
    for (const width of [390, 450, 1656]) {
      await page.setViewportSize({ width, height: width < 640 ? 844 : 1314 })
      const command = await openSettings(page)
      await command.fill(editedCommand)
      if (width < 640) {
        const summary = page
          .locator('summary')
          .filter({ hasText: /^Worker context$/ })
        await summary.focus()
        await summary.press('Enter')
      }
      const relations = page.locator('.wk-relations')
      await relations.scrollIntoViewIfNeeded()
      const pills = relations.getByRole('button')
      await expect(pills).toHaveCount(2)
      const boxes = []
      for (const name of ['state', 'storage']) {
        const pill = pills.filter({ hasText: new RegExp(`^${name}`) })
        const presentation = await textPresentation(pill)
        expect
          .soft(presentation.height, `${width}px ${name} relationship height`)
          .toBeGreaterThanOrEqual(width < 640 ? 48 : 28)
        expect
          .soft(presentation.width, `${width}px ${name} relationship width`)
          .toBeGreaterThanOrEqual(48)
        if (width >= 640) expect(presentation.height).toBeLessThanOrEqual(30)
        boxes.push(presentation)
      }
      expect(
        boxes[0].right <= boxes[1].left || boxes[0].bottom <= boxes[1].top,
      ).toBe(true)
      for (const box of boxes) {
        expect(box.left).toBeGreaterThanOrEqual(0)
        expect(box.right).toBeLessThanOrEqual(width)
      }
      await pills.filter({ hasText: /^state/ }).click()
      await expect(page.locator('.wk-title')).toHaveText('state')
      await selectWorker(page, 'ade')
      await page.getByRole('tab', { name: /^Settings/ }).click()
      await expect(command).toHaveValue(editedCommand)
      expect(await editCalls(page)).toEqual([])
    }
  })
})

test('uses the sticky first-error action to reach exact native YAML, environment, and dependency errors', async ({
  page,
}) => {
  test.setTimeout(60_000)
  await page.setViewportSize({ width: 390, height: 844 })
  await openSettings(page)
  await page.getByRole('button', { name: 'Add variable', exact: true }).click()
  const duplicateName = page.getByRole('textbox', {
    name: 'Variable name',
    exact: true,
  })
  await duplicateName.fill('API_TOKEN')
  const duplicate = page.locator('.wk-env').filter({ has: duplicateName })
  await duplicate
    .getByRole('textbox', { name: 'API_TOKEN value', exact: true })
    .fill('pending-secret')
  const original = page.locator('.wk-env').filter({
    has: page.locator('.wk-env-key').filter({ hasText: /^API_TOKEN$/ }),
  })
  const duplicateAction = page.getByRole('button', {
    name: 'Fix 2 fields before saving',
    exact: true,
  })
  await duplicateAction.focus()
  await duplicateAction.press('Enter')
  await expect(duplicateName).toBeFocused()
  await expect(duplicateName).toHaveAttribute('aria-invalid', 'true')
  await expect(duplicateName).toBeInViewport()
  await expect(
    original.getByRole('button', { name: 'Replace', exact: true }),
  ).toBeVisible()
  await expect(original.getByRole('textbox')).toHaveCount(0)
  await expect(
    duplicate.getByRole('textbox', { name: 'API_TOKEN value', exact: true }),
  ).toHaveValue('pending-secret')
  expect(await editCalls(page)).toEqual([])

  const savedCommand = await openSettings(page, '?commands=1')
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.saveMode = 'reload-error'
  })
  await savedCommand.fill(editedCommand)
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await expect(
    page.getByRole('region', { name: 'Worker settings result', exact: true }),
  ).toContainText('Settings saved; declaration could not be reloaded')
  await page.getByRole('button', { name: 'Replace', exact: true }).click()
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.externalEdit('ade', {
      environment: [
        { key: 'III_PORT', value: '3113', secret: false },
        { key: 'LOG_LEVEL', value: 'info', secret: false },
      ],
    })
    window.__WORKERS_HARNESS__.saveMode = 'success'
  })
  await page
    .getByRole('button', { name: 'Reload saved settings', exact: true })
    .click()
  const rebasedName = page.getByRole('textbox', {
    name: 'Variable name',
    exact: true,
  })
  await expect(rebasedName).toHaveValue('API_TOKEN')
  await rebasedName.fill('')
  const unnamedSecret = page.getByRole('textbox', {
    name: 'New variable value',
    exact: true,
  })
  await expect(unnamedSecret).toHaveAttribute('type', 'password')
  await expect(unnamedSecret).toHaveValue('')
  const rebasedAction = page.getByRole('button', {
    name: 'Fix 1 field before saving',
    exact: true,
  })
  await expect(rebasedAction).toHaveAccessibleDescription(/variable name/i)
  await rebasedAction.focus()
  await rebasedAction.press('Enter')
  await expect(rebasedName).toBeFocused()
  await rebasedName.fill('RESTORED_SECRET')
  await expect(rebasedAction).toHaveAccessibleDescription(/secret value/i)
  await rebasedAction.focus()
  await rebasedAction.press('Enter')
  const secretValue = page.getByRole('textbox', {
    name: 'RESTORED_SECRET value',
    exact: true,
  })
  await expect(secretValue).toBeFocused()
  await expect(secretValue).toHaveAttribute('type', 'password')
  await expect(secretValue).toHaveValue('')
  await secretValue.fill('pending-secret')
  await expect(
    page.getByRole('button', { name: 'Save and restart', exact: true }),
  ).toBeEnabled()
  expect(await editCalls(page)).toHaveLength(1)
  expect((await editCalls(page))[0].payload.environment).toBeUndefined()
  expect(await page.evaluate(() => scrollY)).toBe(0)

  const command = await openSettings(page, '?commands=1')
  const lines = Array.from(
    { length: 100 },
    (_, index) =>
      `key_${String(index + 1).padStart(3, '0')}: value_${index + 1}`,
  )
  lines[69] = 'key_001: duplicate'
  const config = lines.join('\n')
  await page.evaluate((config_override) => {
    window.__WORKERS_HARNESS__.externalEdit('ade', { config_override })
    window.__WORKERS_HARNESS__.runCommand('refresh')
  }, config)
  await expect(page.locator('.wk-code .monaco-editor')).toBeVisible()
  await expect
    .poll(
      async () =>
        (await page.evaluate(() => window.__WORKERS_HARNESS__.editorSnapshot()))
          .value,
    )
    .toBe(config)
  await command.fill(editedCommand)
  const editor = page.getByRole('textbox', {
    name: 'Configuration override YAML',
    exact: true,
  })
  await expect(editor).toHaveAttribute('aria-invalid', 'true')
  const links = await editor.evaluate((element) => {
    const errorId = element.getAttribute('aria-errormessage')
    return {
      errorId,
      described: element.getAttribute('aria-describedby')?.split(/\s+/) ?? [],
      text: errorId ? document.getElementById(errorId)?.textContent : null,
    }
  })
  expect(links.described).toContain(links.errorId)
  expect(links.text).toMatch(/unique.*line 70|line 70.*unique/i)
  await page.locator('.wk-main').evaluate((element) => {
    element.scrollTop = 0
  })
  const action = page.getByRole('button', {
    name: 'Fix 1 field before saving',
    exact: true,
  })
  await expect(action).toBeVisible()
  await expect(action).toHaveAccessibleDescription(/unique.*70|70.*unique/i)
  await action.focus()
  await action.press('Enter')
  await expect
    .poll(() =>
      page.evaluate(() => window.__WORKERS_HARNESS__.editorSnapshot()),
    )
    .toMatchObject({
      value: config,
      focused: true,
      position: { lineNumber: 70, column: 1 },
    })
  const caret = page.locator(
    '.wk-code .native-edit-context:focus, .wk-code textarea.inputarea:focus',
  )
  await expect(caret).toBeInViewport()
  await expect(
    page
      .locator('.wk-code .view-line')
      .filter({ hasText: 'key_001: duplicate' }),
  ).toBeInViewport()
  const apply = page.getByRole('region', {
    name: 'Apply worker settings',
    exact: true,
  })
  await expect(apply).toContainText(/unique.*70|70.*unique/i)
  expect(await page.evaluate(() => scrollY)).toBe(0)
  expect((await page.locator('.wk-header').boundingBox())?.y).toBe(0)
  await expect(
    page.getByRole('button', { name: 'Save and restart', exact: true }),
  ).toBeDisabled()
  expect(await editCalls(page)).toEqual([])

  const flowCommand = await openSettings(page, '?commands=1')
  const flowConfig = `${Array.from(
    { length: 99 },
    (_, index) => `key_${index + 1}: ${index + 1}`,
  ).join('\n')}\nbroken: [\n`
  await page.evaluate((config_override) => {
    window.__WORKERS_HARNESS__.externalEdit('ade', { config_override })
    window.__WORKERS_HARNESS__.runCommand('refresh')
  }, flowConfig)
  await expect(page.locator('.wk-code .monaco-editor')).toBeVisible()
  await expect
    .poll(
      async () =>
        (await page.evaluate(() => window.__WORKERS_HARNESS__.editorSnapshot()))
          .value,
    )
    .toBe(flowConfig)
  await flowCommand.fill(editedCommand)
  await expect(editor).toHaveAttribute('aria-invalid', 'true')
  const flowLinks = await editor.evaluate((element) => {
    const errorId = element.getAttribute('aria-errormessage')
    const error = errorId ? document.getElementById(errorId) : null
    return {
      errorId,
      described: element.getAttribute('aria-describedby')?.split(/\s+/) ?? [],
      brief: error?.firstChild?.textContent,
      technical: error?.querySelector('span')?.textContent,
    }
  })
  expect(flowLinks.described).toContain(flowLinks.errorId)
  expect(flowLinks.brief).toMatch(/line 101, column 1.*indentation.*close.*\]/i)
  expect(flowLinks.brief).not.toMatch(/Flow sequence|block collection/)
  expect(flowLinks.technical).toBe(
    'Flow sequence in block collection must be sufficiently indented and end with a ]',
  )
  await page.locator('.wk-main').evaluate((element) => {
    element.scrollTop = 0
  })
  await expect(action).toHaveAccessibleDescription(/101.*close.*\]/i)
  await action.focus()
  await action.press('Enter')
  await expect
    .poll(() =>
      page.evaluate(() => window.__WORKERS_HARNESS__.editorSnapshot()),
    )
    .toMatchObject({
      value: flowConfig,
      focused: true,
      position: { lineNumber: 101, column: 1 },
    })
  await expect(caret).toBeInViewport()
  await expect(
    page.locator('.wk-code .view-lines > .view-line').last(),
  ).toBeInViewport()
  await expect(apply).toContainText(/101.*indentation.*close.*\]/i)
  await expect(apply).not.toContainText(/Flow sequence|block collection/)
  if (!flowLinks.errorId)
    throw new Error('The flow error needs its native ARIA link')
  await expect(page.locator(`[id="${flowLinks.errorId}"] span`)).toBeVisible()
  expect(await page.evaluate(() => scrollY)).toBe(0)
  expect((await page.locator('.wk-header').boundingBox())?.y).toBe(0)
  await expect(
    page.getByRole('button', { name: 'Save and restart', exact: true }),
  ).toBeDisabled()
  expect(await editCalls(page)).toEqual([])

  await openSettings(page)
  await page.getByRole('button', { name: 'Add variable', exact: true }).click()
  await page
    .getByRole('textbox', { name: 'New variable value', exact: true })
    .fill('hello')
  await action.focus()
  await action.press('Enter')
  const name = page.getByRole('textbox', { name: 'Variable name', exact: true })
  await expect(name).toBeFocused()
  await expect(name).toHaveAttribute('aria-invalid', 'true')
  const errorId = await name.getAttribute('aria-describedby')
  if (!errorId)
    throw new Error('The invalid environment field needs its error link')
  await expect(page.locator(`[id="${errorId}"]`)).toBeInViewport()
  await expect(apply).toContainText(/name/i)
  expect(await page.evaluate(() => scrollY)).toBe(0)

  await openSettings(page, '?commands=1&cycleWorkers=1')
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.externalEdit('ade', { start_after: ['storage'] })
    window.__WORKERS_HARNESS__.runCommand('refresh')
  })
  const remove = page.getByRole('button', {
    name: 'Stop waiting for storage',
    exact: true,
  })
  await expect(remove).toBeVisible()
  await action.focus()
  await action.press('Enter')
  await expect(remove).toBeFocused()
  await expect(remove).toBeInViewport()
  await expect(apply).toContainText('ade → storage → state → ade')
  await expect(
    page.getByRole('button', { name: 'Save and restart', exact: true }),
  ).toBeDisabled()
  expect(await page.evaluate(() => scrollY)).toBe(0)
  expect(await editCalls(page)).toEqual([])
})

test('reports native filtered log clipboard success and failure while preserving log focus and pending worker drafts', async ({
  page,
  browser,
}) => {
  test.setTimeout(120_000)
  await page.addInitScript(() => {
    const clipboard = {
      mode: 'success' as 'success' | 'failure',
      writes: [] as string[],
      fallbackCalls: [] as string[],
    }
    window.__CLIPBOARD_HARNESS__ = clipboard
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: {
        async writeText(text: string) {
          clipboard.writes.push(text)
          if (clipboard.mode === 'failure')
            throw new DOMException(
              'Fixture clipboard denied',
              'NotAllowedError',
            )
        },
      },
    })
    Object.defineProperty(document, 'execCommand', {
      configurable: true,
      value: (command: string) => {
        clipboard.fallbackCalls.push(command)
        return false
      },
    })
  })
  for (const width of [1656, 390, 450]) {
    for (const theme of ['dark', 'light']) {
      await page.setViewportSize({ width, height: width === 1656 ? 1314 : 900 })
      const command = await openSettings(page, `?logCase=copy&theme=${theme}`)
      await command.fill(editedCommand)
      await page.getByRole('tab', { name: 'Log', exact: true }).click()
      const logTab = page.locator('.wk-log-tab')
      const filter = logTab.getByRole('searchbox', {
        name: 'Filter log lines',
        exact: true,
      })
      await filter.fill('REQUEST')
      const log = logTab.getByRole('log', { name: 'ade log', exact: true })
      const messages = [
        'Worker is ready and listening for requests.',
        'A second request completed successfully.',
      ]
      await expect(log.locator(':scope > div')).toHaveCount(2)
      expect(await log.locator(':scope > div').allTextContents()).toEqual(
        messages,
      )
      await expect(logTab.getByText(/^Local time(?: ·|$)/)).toHaveCount(0)
      const copied = messages.join('\n')
      const copy = logTab.getByRole('button', {
        name: /^(Copy log|Log copied)$/,
      })
      const copyBox = await copy.boundingBox()
      const followBox = await logTab
        .getByRole('button', { name: 'Follow the log', exact: true })
        .boundingBox()
      const filterMetrics = await filter.evaluate((element) => {
        const style = getComputedStyle(element)
        const box = element.getBoundingClientRect()
        const canvas = document.createElement('canvas')
        const context = canvas.getContext('2d')
        if (!context || !(element instanceof HTMLInputElement))
          throw new Error('The native log search must be measurable')
        context.font = style.font
        return {
          height: box.height,
          textWidth: context.measureText(element.placeholder).width,
          contentWidth:
            element.clientWidth -
            parseFloat(style.paddingLeft) -
            parseFloat(style.paddingRight),
        }
      })
      const toolbarOverflow = await logTab
        .getByRole('toolbar', { name: 'ade log', exact: true })
        .evaluate((element) => element.scrollWidth - element.clientWidth)
      if (!copyBox || !followBox)
        throw new Error('The native Copy and Follow controls must be painted')
      expect.soft(copyBox.width).toBeGreaterThanOrEqual(width < 640 ? 48 : 30)
      expect.soft(copyBox.height).toBeGreaterThanOrEqual(width < 640 ? 48 : 30)
      if (width === 1656) {
        expect(copyBox.width).toBe(30)
        expect(copyBox.height).toBe(30)
      } else {
        expect.soft(followBox.width).toBeGreaterThanOrEqual(48)
        expect.soft(followBox.height).toBeGreaterThanOrEqual(48)
        expect.soft(filterMetrics.height).toBeGreaterThanOrEqual(48)
      }
      expect(filterMetrics.textWidth).toBeLessThanOrEqual(
        filterMetrics.contentWidth,
      )
      expect(toolbarOverflow).toBeLessThanOrEqual(1)
      if (width < 640) {
        const follow = logTab.getByRole('button', {
          name: 'Follow the log',
          exact: true,
        })
        const label = follow.locator('.wk-follow-label')
        await expect.soft(label).toBeVisible({ timeout: 500 })
        const labelPaint = await label.evaluate((element) => {
          const range = document.createRange()
          range.selectNodeContents(element)
          const text = range.getBoundingClientRect()
          const button = element.closest('button')?.getBoundingClientRect()
          return {
            text: element.textContent,
            bounds: {
              x: text.x,
              y: text.y,
              width: text.width,
              height: text.height,
            },
            button: button
              ? {
                  x: button.x,
                  y: button.y,
                  width: button.width,
                  height: button.height,
                }
              : null,
          }
        })
        expect
          .soft(
            labelPaint.bounds.width,
            'Follow needs actual painted text on phone',
          )
          .toBeGreaterThan(0)
        if (labelPaint.bounds.width > 0 && labelPaint.button) {
          expect(labelPaint.bounds.x).toBeGreaterThanOrEqual(
            labelPaint.button.x,
          )
          expect(
            labelPaint.bounds.x + labelPaint.bounds.width,
          ).toBeLessThanOrEqual(labelPaint.button.x + labelPaint.button.width)
          expect(
            labelPaint.bounds.y + labelPaint.bounds.height,
          ).toBeLessThanOrEqual(labelPaint.button.y + labelPaint.button.height)
        }
        const id = `phone-follow-${width}-${theme}`
        await test.info().attach(id, {
          body: JSON.stringify(
            {
              labelPaint,
              filterMetrics,
              toolbarOverflow,
              copyBox,
              followBox,
            },
            null,
            2,
          ),
          contentType: 'application/json',
        })
        const screenshot = test.info().outputPath(`${id}.png`)
        await page.screenshot({ path: screenshot })
        await test
          .info()
          .attach(id, { path: screenshot, contentType: 'image/png' })
      }
      await copy.focus()
      await copy.press('Enter')
      await expect
        .poll(() => page.evaluate(() => window.__CLIPBOARD_HARNESS__?.writes))
        .toEqual([copied])
      const success = logTab
        .getByRole('status')
        .filter({ hasText: 'Log copied' })
      await expect.soft(success).toBeVisible()
      await page.evaluate(() => {
        if (window.__CLIPBOARD_HARNESS__)
          window.__CLIPBOARD_HARNESS__.mode = 'failure'
      })
      await copy.press('Enter')
      await expect
        .poll(() => page.evaluate(() => window.__CLIPBOARD_HARNESS__?.writes))
        .toEqual([copied, copied])
      expect(
        await page.evaluate(() => window.__CLIPBOARD_HARNESS__?.fallbackCalls),
      ).toEqual(['copy'])
      const failure = logTab.getByRole('alert').filter({
        hasText: 'Log could not be copied',
      })
      await expect.soft(failure).toBeVisible()
      await expect.soft(failure).toContainText(/select.*log.*copy.*keyboard/i)
      await expect(copy).toBeFocused()
      await expect(copy).toBeInViewport()
      await expect(failure).toBeInViewport()
      await expect(filter).toHaveValue('REQUEST')
      expect(await log.locator(':scope > div').allTextContents()).toEqual(
        messages,
      )
      await test
        .info()
        .attach(`filtered-native-log-clipboard-${width}-${theme}`, {
          body: JSON.stringify(
            await page.evaluate(() => ({
              clipboard: window.__CLIPBOARD_HARNESS__,
              writes: window.__WORKERS_HARNESS__.calls.filter(
                (call) => call.functionId === 'console::compose::edit',
              ),
              active: document.activeElement?.getAttribute('aria-label'),
            })),
            null,
            2,
          ),
          contentType: 'application/json',
        })
      await page.evaluate(() => {
        if (window.__CLIPBOARD_HARNESS__)
          window.__CLIPBOARD_HARNESS__.mode = 'success'
      })
      await copy.press('Enter')
      await expect
        .poll(() => page.evaluate(() => window.__CLIPBOARD_HARNESS__?.writes))
        .toEqual([copied, copied, copied])
      await expect.soft(failure).toHaveCount(0)
      await expect
        .soft(logTab.getByRole('status').filter({ hasText: 'Log copied' }))
        .toBeVisible()
      await expect(copy).toBeFocused()
      await expect(success).toBeInViewport()
      const feedbackGeometry = await success.evaluate((element) => {
        const box = element.getBoundingClientRect()
        const hit = document.elementFromPoint(
          box.x + box.width / 2,
          box.y + box.height / 2,
        )
        return {
          painted: hit === element || element.contains(hit),
          width: box.width,
          height: box.height,
        }
      })
      expect(feedbackGeometry.painted).toBe(true)
      expect(feedbackGeometry.height).toBeGreaterThan(0)
      await filter.fill('no matching fixture output')
      await expect(log.locator(':scope > div')).toHaveCount(0)
      await expect(copy).toBeDisabled()
      await expect(success).toHaveCount(0)
      const noMatch = logTab.getByText('No log lines match this filter', {
        exact: true,
      })
      await expect(noMatch).toBeVisible()
      await expect(
        logTab.getByText('Change the filter or clear it to see the log.', {
          exact: true,
        }),
      ).toBeVisible()
      await filter.press('Escape')
      await expect(filter).toBeFocused()
      await expect(filter).toHaveValue('')
      await expect(noMatch).toHaveCount(0)
      await expect(log.locator(':scope > div')).toHaveCount(3)
      expect(await log.locator(':scope > div').allTextContents()).toEqual([
        messages[0],
        'A background maintenance cycle completed.',
        messages[1],
      ])
      await filter.fill('no matching fixture output')
      const clear = logTab.getByRole('button', { name: 'Clear', exact: true })
      await clear.focus()
      await clear.press('Enter')
      await expect(filter).toBeFocused()
      await expect(filter).toHaveValue('')
      await expect(noMatch).toHaveCount(0)
      await expect(log.locator(':scope > div')).toHaveCount(3)
      await filter.press('Tab')
      const follow = logTab.getByRole('button', {
        name: 'Follow the log',
        exact: true,
      })
      await expect(follow).toBeFocused()
      await expect(follow).toBeInViewport()
      expect(
        await follow.evaluate((element) => {
          const box = element.getBoundingClientRect()
          const hit = document.elementFromPoint(
            box.x + box.width / 2,
            box.y + box.height / 2,
          )
          return hit === element || element.contains(hit)
        }),
      ).toBe(true)
      await filter.fill('REQUEST')
      await expect(log.locator(':scope > div')).toHaveCount(2)
      await expect(copy).toBeEnabled()
      expect(
        await page.evaluate(() => window.__CLIPBOARD_HARNESS__?.writes),
      ).toEqual([copied, copied, copied])
      await page.getByRole('tab', { name: /^Settings/ }).click()
      await expect(command).toHaveValue(editedCommand)
      await selectWorker(page, 'agent')
      await selectWorker(page, 'ade')
      await page.getByRole('tab', { name: /^Settings/ }).click()
      await expect(command).toHaveValue(editedCommand)
      expect(await editCalls(page)).toEqual([])
    }
  }

  const structured = [
    '2026-10-09T12:34:56.123456Z TRACE ade::rpc: Routine poll began.',
    '2026-10-09T12:34:57.234567Z DEBUG ade::rpc: Poll details recorded.',
    '2026-10-09T12:34:58.345678Z INFO ade::rpc: REQUEST accepted.',
    '2026-10-09T12:34:59.456789Z WARN ade::rpc: REQUEST retry scheduled.',
    '2026-10-09T12:35:00.567890Z ERROR ade::rpc: REQUEST failed after retries.',
  ]
  const levels = ['TRACE', 'DEBUG', 'INFO', 'WARN', 'ERROR']
  for (const layout of [
    { width: 1656, pane: false },
    { width: 1656, pane: true },
    { width: 390, pane: false },
    { width: 450, pane: false },
  ]) {
    for (const theme of ['dark', 'light']) {
      await page.setViewportSize({
        width: layout.width,
        height: layout.width < 640 ? 900 : 1314,
      })
      const command = await openSettings(
        page,
        `?logCase=structured&theme=${theme}${layout.pane ? '&pane=720' : ''}`,
      )
      await command.fill(editedCommand)
      await page.getByRole('tab', { name: 'Log', exact: true }).click()
      const tab = page.locator('.wk-log-tab')
      const log = tab.getByRole('log', { name: 'ade log', exact: true })
      await expect(log.locator(':scope > div')).toHaveCount(5)
      await expect(log.locator('.wk-log-level')).toHaveText(
        levels.map((level) => level.padEnd(5)),
      )
      const samples: {
        role: string
        text: string
        css: { color: string; background: string; fontSize: string }
        presentation: Awaited<ReturnType<typeof textPresentation>>
      }[] = []
      const hiddenTargets: { text: string; display: string }[] = []
      for (const [role, selector] of [
        ['timestamp', '.wk-log-time'],
        ['level', '.wk-log-level'],
        ['target', '.wk-log-target'],
      ]) {
        const tokens = log.locator(selector)
        await expect(tokens).toHaveCount(5)
        for (let index = 0; index < 5; index++) {
          const token = tokens.nth(index)
          if (role === 'target' && layout.width < 640) {
            await expect(token).toBeHidden()
            hiddenTargets.push(
              await token.evaluate((element) => ({
                text: element.textContent ?? '',
                display: getComputedStyle(element).display,
              })),
            )
            continue
          }
          if (role === 'timestamp')
            await expect(token).toHaveAttribute(
              'title',
              structured[index].split(' ')[0],
            )
          await expect(token).toBeVisible()
          await expect(token).toBeInViewport()
          const presentation = await textPresentation(token)
          expect(presentation.connected).toBe(true)
          expect(presentation.textBounds.length).toBeGreaterThan(0)
          samples.push({
            role: `${role}-${levels[index]}`,
            text: (await token.textContent()) ?? '',
            css: await token.evaluate((element) => {
              const style = getComputedStyle(element)
              return {
                color: style.color,
                background: style.backgroundColor,
                fontSize: style.fontSize,
              }
            }),
            presentation,
          })
        }
      }
      const geometry = await log.evaluate((element) => ({
        internalOverflow: element.scrollWidth - element.clientWidth,
        whiteSpace: getComputedStyle(element).whiteSpace,
        clientWidth: element.clientWidth,
        documentOverflow: document.documentElement.scrollWidth - innerWidth,
        scrollY,
        headerY: document.querySelector('.wk-header')?.getBoundingClientRect()
          .y,
      }))
      expect(geometry.documentOverflow).toBe(0)
      expect(geometry.scrollY).toBe(0)
      expect(geometry.headerY).toBe(0)
      if (layout.width < 640) {
        expect(geometry.whiteSpace).toBe('pre-wrap')
        expect(geometry.internalOverflow).toBeLessThanOrEqual(1)
        expect(hiddenTargets).toHaveLength(5)
      } else {
        expect(geometry.whiteSpace).toBe('pre')
        expect(hiddenTargets).toEqual([])
      }
      const id = `${theme}-${layout.pane ? 'pane720' : layout.width}`
      await test.info().attach(`structured-log-${id}`, {
        body: JSON.stringify({ samples, hiddenTargets, geometry }, null, 2),
        contentType: 'application/json',
      })
      await page.evaluate(async () => {
        await document.fonts.ready
        await Promise.allSettled(
          document
            .getAnimations()
            .filter((animation) =>
              Number.isFinite(animation.effect?.getComputedTiming().endTime),
            )
            .map((animation) => animation.finished),
        )
      })
      const screenshot = test.info().outputPath(`structured-log-${id}.png`)
      await page.screenshot({ path: screenshot })
      await test.info().attach(`structured-log-${id}`, {
        path: screenshot,
        contentType: 'image/png',
      })
      expect
        .soft(
          samples
            .filter((sample) => sample.presentation.contrast < 4.5)
            .map((sample) => ({
              role: sample.role,
              text: sample.text,
              contrast: sample.presentation.contrast,
            })),
          `${id} actual structured log tokens must meet normal-text AA`,
        )
        .toEqual([])
      const filter = tab.getByRole('searchbox', {
        name: 'Filter log lines',
        exact: true,
      })
      await filter.fill('REQUEST')
      await expect(log.locator(':scope > div')).toHaveCount(3)
      await expect(log.locator('.wk-log-level')).toHaveText([
        'INFO ',
        'WARN ',
        'ERROR',
      ])
      const copy = tab.getByRole('button', { name: 'Copy log', exact: true })
      await copy.focus()
      await copy.press('Enter')
      await expect
        .poll(() => page.evaluate(() => window.__CLIPBOARD_HARNESS__?.writes))
        .toEqual([structured.slice(2).join('\n')])
      await expect(
        tab.getByRole('status').filter({ hasText: 'Log copied' }),
      ).toBeVisible()
      await expect(copy).toBeFocused()
      await filter.fill('no matching fixture output')
      await expect(log.locator(':scope > div')).toHaveCount(0)
      await expect(
        tab.getByRole('button', { name: 'Log copied', exact: true }),
      ).toHaveCount(0)
      await expect(
        tab.getByRole('button', { name: 'Copy log', exact: true }),
      ).toBeDisabled()
      const clear = tab.getByRole('button', { name: 'Clear', exact: true })
      await clear.focus()
      await clear.press('Enter')
      await expect(filter).toBeFocused()
      await expect(log.locator(':scope > div')).toHaveCount(5)
      await filter.press('Tab')
      const follow = tab.getByRole('button', {
        name: 'Follow the log',
        exact: true,
      })
      await expect(follow).toBeFocused()
      await expect(follow).toBeInViewport()
      expect(
        await follow.evaluate((element) => {
          const rect = element.getBoundingClientRect()
          const hit = document.elementFromPoint(
            rect.x + rect.width / 2,
            rect.y + rect.height / 2,
          )
          return hit === element || element.contains(hit)
        }),
      ).toBe(true)
      await page.getByRole('tab', { name: /^Settings/ }).click()
      await expect(command).toHaveValue(editedCommand)
      await selectWorker(page, 'agent')
      await selectWorker(page, 'ade')
      await page.getByRole('tab', { name: /^Settings/ }).click()
      await expect(command).toHaveValue(editedCommand)
      expect(
        await page.evaluate(() =>
          window.__WORKERS_HARNESS__.calls.filter((call) =>
            [
              'console::compose::edit',
              'compose::restart',
              'compose::start',
              'compose::stop',
            ].includes(call.functionId),
          ),
        ),
      ).toEqual([])
    }
  }

  for (const zone of [
    { id: 'UTC', time: '12:34:56.123', width: 1656, theme: 'dark' },
    { id: 'America/Recife', time: '09:34:56.123', width: 390, theme: 'light' },
    { id: 'Asia/Kathmandu', time: '18:19:56.123', width: 390, theme: 'light' },
  ]) {
    const context = await browser.newContext({
      baseURL: 'http://127.0.0.1:4180',
      timezoneId: zone.id,
      viewport: { width: zone.width, height: zone.width < 640 ? 900 : 1314 },
    })
    const zonedPage = await context.newPage()
    const errors: string[] = []
    const sockets: string[] = []
    zonedPage.on('pageerror', (error) => errors.push(error.message))
    zonedPage.on('websocket', (socket) => sockets.push(socket.url()))
    await zonedPage.addInitScript(() => {
      window.localStorage.clear()
      const clipboard = {
        mode: 'success' as 'success' | 'failure',
        writes: [] as string[],
        fallbackCalls: [] as string[],
      }
      window.__CLIPBOARD_HARNESS__ = clipboard
      Object.defineProperty(navigator, 'clipboard', {
        configurable: true,
        value: {
          async writeText(text: string) {
            clipboard.writes.push(text)
          },
        },
      })
      Object.defineProperty(document, 'execCommand', {
        configurable: true,
        value: (command: string) => {
          clipboard.fallbackCalls.push(command)
          return false
        },
      })
    })
    try {
      const command = await openSettings(
        zonedPage,
        `?logCase=structured&theme=${zone.theme}`,
      )
      await command.fill(editedCommand)
      await zonedPage.getByRole('tab', { name: 'Log', exact: true }).click()
      const tab = zonedPage.locator('.wk-log-tab')
      const log = tab.getByRole('log', { name: 'ade log', exact: true })
      const timestamp = log.locator('.wk-log-time').first()
      await expect(timestamp).toHaveText(zone.time)
      await expect(timestamp).toHaveAttribute(
        'title',
        '2026-10-09T12:34:56.123456Z',
      )
      const resolvedZone = await zonedPage.evaluate(
        () => Intl.DateTimeFormat().resolvedOptions().timeZone,
      )
      const label = tab.getByText(`Local time · ${resolvedZone}`, {
        exact: true,
      })
      await test.info().attach(`log-timezone-${zone.id.replaceAll('/', '-')}`, {
        body: JSON.stringify(
          {
            requestedZone: zone.id,
            resolvedZone,
            localTime: await timestamp.textContent(),
            rawUtcTitle: await timestamp.getAttribute('title'),
            labelPresent: await label.isVisible(),
          },
          null,
          2,
        ),
        contentType: 'application/json',
      })
      const screenshot = test
        .info()
        .outputPath(`log-timezone-${zone.id.replaceAll('/', '-')}.png`)
      await zonedPage.evaluate(async () => {
        await document.fonts.ready
        await Promise.allSettled(
          document
            .getAnimations()
            .filter((animation) =>
              Number.isFinite(animation.effect?.getComputedTiming().endTime),
            )
            .map((animation) => animation.finished),
        )
      })
      await zonedPage.screenshot({ path: screenshot })
      await test.info().attach(`log-timezone-${zone.id.replaceAll('/', '-')}`, {
        path: screenshot,
        contentType: 'image/png',
      })
      await expect
        .soft(
          label,
          `${zone.id} local timestamps need their actual browser timezone`,
        )
        .toBeVisible()
      if (await label.isVisible()) {
        const presentation = await textPresentation(label)
        await test
          .info()
          .attach(`log-timezone-presentation-${zone.id.replaceAll('/', '-')}`, {
            body: JSON.stringify(
              { requestedZone: zone.id, resolvedZone, presentation },
              null,
              2,
            ),
            contentType: 'application/json',
          })
        expect(presentation.contrast).toBeGreaterThanOrEqual(4.5)
        expect(presentation.left).toBeGreaterThanOrEqual(0)
        expect(presentation.right).toBeLessThanOrEqual(zone.width)
        expect(presentation.scrollWidth).toBeLessThanOrEqual(
          presentation.clientWidth + 1,
        )
      }
      const filter = tab.getByRole('searchbox', {
        name: 'Filter log lines',
        exact: true,
      })
      await filter.fill('REQUEST')
      await expect(log.locator(':scope > div')).toHaveCount(3)
      const copy = tab.getByRole('button', { name: /^(Copy log|Log copied)$/ })
      await copy.focus()
      await copy.press('Enter')
      await expect
        .poll(() =>
          zonedPage.evaluate(() => window.__CLIPBOARD_HARNESS__?.writes),
        )
        .toEqual([structured.slice(2).join('\n')])
      await expect(
        tab.getByRole('status').filter({ hasText: 'Log copied' }),
      ).toBeVisible()
      await expect(copy).toBeFocused()
      await filter.fill('no matching fixture output')
      await expect(
        tab.getByRole('button', { name: 'Copy log', exact: true }),
      ).toBeDisabled()
      const clear = tab.getByRole('button', { name: 'Clear', exact: true })
      await clear.focus()
      await clear.press('Enter')
      await expect(filter).toBeFocused()
      await expect(log.locator(':scope > div')).toHaveCount(5)
      await filter.press('Tab')
      await expect(
        tab.getByRole('button', { name: 'Follow the log', exact: true }),
      ).toBeFocused()
      await zonedPage.getByRole('tab', { name: /^Settings/ }).click()
      await expect(command).toHaveValue(editedCommand)
      expect(await editCalls(zonedPage)).toEqual([])
      expect(await zonedPage.evaluate(() => scrollY)).toBe(0)
      expect((await zonedPage.locator('.wk-header').boundingBox())?.y).toBe(0)
    } finally {
      await context.close()
      expect(errors).toEqual([])
      expect(sockets).toEqual([])
    }
  }
})

test('opens the worker Log natively from a restart-failure result without another operation or losing drafts', async ({
  page,
}) => {
  const command = await openSettings(page)
  await selectWorker(page, 'agent')
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await command.fill('cargo run --locked --bin agent')
  await selectWorker(page, 'ade')
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await command.fill(editedCommand)
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.saveMode = 'restart-error'
  })
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  const result = page.getByRole('region', {
    name: 'Worker settings result',
    exact: true,
  })
  await expect(result).toContainText('Configuration saved; restart failed')
  const before = await page.evaluate(() =>
    window.__WORKERS_HARNESS__.calls.filter((call) =>
      [
        'console::compose::edit',
        'compose::restart',
        'compose::start',
        'compose::stop',
      ].includes(call.functionId),
    ),
  )
  const openLog = result.getByRole('button', { name: 'Open Log', exact: true })
  await expect(openLog).toBeVisible()
  await openLog.focus()
  await openLog.press('Enter')
  const log = page.getByRole('tab', { name: 'Log', exact: true })
  await expect(log).toHaveAttribute('aria-selected', 'true')
  await expect(log).toBeFocused()
  await expect(
    page.getByText('Worker is ready and listening for requests.', {
      exact: true,
    }),
  ).toBeVisible()
  expect(
    await page.evaluate(() =>
      window.__WORKERS_HARNESS__.calls.filter((call) =>
        [
          'console::compose::edit',
          'compose::restart',
          'compose::start',
          'compose::stop',
        ].includes(call.functionId),
      ),
    ),
  ).toEqual(before)
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await expect(command).toHaveValue(editedCommand)
  await expect(result).toContainText('Configuration saved; restart failed')
  await expect(
    page.getByRole('button', { name: 'Save and restart', exact: true }),
  ).toBeDisabled()
  await selectWorker(page, 'agent')
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await expect(command).toHaveValue('cargo run --locked --bin agent')
  expect(await editCalls(page)).toHaveLength(1)
})

test('keeps the actual first painted Environment prose line within ninety-two characters in both themes', async ({
  page,
}) => {
  for (const theme of ['dark', 'light']) {
    await openSettings(page, `?theme=${theme}`)
    const description = page
      .getByRole('region', { name: 'Environment', exact: true })
      .locator('.iii-ui-settings-section__description')
    await textPresentation(description)
    const lines = await paintedTextLines(description)
    expect(lines.join(' ')).toContain(
      'saved secrets never appear in the form or preview.',
    )
    expect
      .soft(lines[0].length, `${theme} first painted line: ${lines[0]}`)
      .toBeLessThanOrEqual(92)
    await selectWorker(page, 'state')
    await page.getByRole('tab', { name: /^Settings/ }).click()
    await expect(
      page.getByRole('textbox', { name: 'Command', exact: true }),
    ).toHaveValue('./target/debug/state')
    const applyNote = page
      .getByRole('region', { name: 'Apply worker settings', exact: true })
      .locator('.wk-note')
    await textPresentation(applyNote)
    const noteLines = await paintedTextLines(applyNote)
    expect(noteLines.join(' ')).toContain('ade depends on it')
    for (const line of noteLines) {
      expect
        .soft(line.length, `${theme} actual apply note line: ${line}`)
        .toBeLessThanOrEqual(80)
    }
    expect(await editCalls(page)).toEqual([])
  }
})

for (const scenario of [
  { mode: 'identity', theme: 'dark', width: 1656, height: 1314 },
  { mode: 'catalog', theme: 'light', width: 390, height: 844 },
  { mode: 'constraints', theme: 'dark', width: 1656, height: 1314 },
  {
    mode: 'constraints',
    theme: 'light',
    width: 390,
    height: 844,
    catalog: true,
  },
  { mode: 'error', theme: 'light', width: 390, height: 844 },
  { mode: 'error', theme: 'dark', width: 1656, height: 1314, pane: 720 },
]) {
  test(`opens the actual worker configuration reference beside a pending override without changing its draft in ${scenario.theme} at ${scenario.width}${'pane' in scenario ? `/${scenario.pane} pane` : ''}: ${scenario.mode}`, async ({
    page,
  }) => {
    await page.setViewportSize({
      width: scenario.width,
      height: scenario.height,
    })
    const command = await openSettings(
      page,
      `?configurationReference=${scenario.mode}&theme=${scenario.theme}${'pane' in scenario ? `&pane=${scenario.pane}` : ''}`,
    )
    const catalog =
      scenario.mode === 'catalog' || ('catalog' in scenario && scenario.catalog)
    if ('catalog' in scenario && scenario.catalog)
      await page.evaluate(() => {
        window.__WORKERS_HARNESS__.configurationReferenceMode = 'catalog'
      })
    await command.fill(editedCommand)
    await expect(page.locator('.wk-code .monaco-editor')).toBeVisible()
    const yaml = page.getByRole('textbox', {
      name: 'Configuration override YAML',
      exact: true,
    })
    await yaml.focus()
    await yaml.press('ControlOrMeta+A')
    await page.keyboard.type('port: 3114')
    await expect
      .poll(
        async () =>
          (
            await page.evaluate(() =>
              window.__WORKERS_HARNESS__.editorSnapshot(),
            )
          ).value,
      )
      .toBe('port: 3114')
    const section = page.getByRole('region', {
      name: 'Configuration override',
      exact: true,
    })
    const reference = section.locator('summary').filter({
      hasText: /^Configuration reference$/,
    })
    await expect(reference).toBeVisible()
    expect(
      await page.evaluate(() =>
        window.__WORKERS_HARNESS__.calls.filter(
          (call) =>
            call.functionId.startsWith('configuration::') ||
            call.functionId.endsWith('::configuration-id'),
        ),
      ),
    ).toEqual([])
    await reference.focus()
    await reference.press('Enter')
    const panel = section.locator('.wk-config-reference-body')
    if (catalog) {
      await expect(panel).toContainText(
        'This worker did not advertise a configuration reference.',
      )
      expect(
        await page.evaluate(() =>
          window.__WORKERS_HARNESS__.calls.filter(
            (call) =>
              call.functionId.endsWith('::configuration-id') ||
              call.functionId === 'configuration::schema',
          ),
        ),
      ).toEqual([])
      await expect(
        panel.getByText(
          scenario.mode === 'constraints' ? 'http_port' : 'port',
          { exact: true },
        ),
      ).toHaveCount(0)
      await panel
        .getByRole('button', {
          name: 'Configuration reference entry',
          exact: true,
        })
        .click()
      await page
        .getByRole('option', { name: /^Registered ADE configuration/ })
        .click()
    }
    if (scenario.mode === 'error') {
      await expect(panel).toContainText('Configuration reference unavailable')
      await expect(panel).toContainText(
        'Mock registered schema is temporarily unavailable',
      )
      const retry = panel.getByRole('button', { name: 'Retry', exact: true })
      const retryBox = await retry.boundingBox()
      const detailBox = await panel
        .getByText('Mock registered schema is temporarily unavailable', {
          exact: false,
        })
        .boundingBox()
      const alertMetrics = await panel
        .getByRole('alert')
        .evaluate((element) => {
          const box = element.getBoundingClientRect()
          const style = getComputedStyle(element)
          return {
            width:
              box.width -
              parseFloat(style.paddingLeft) -
              parseFloat(style.paddingRight),
          }
        })
      if (!retryBox || !detailBox)
        throw new Error(
          'The schema read error and its native Retry must be visible',
        )
      expect
        .soft(
          retryBox.height,
          'reference Retry must retain the narrow48px target',
        )
        .toBeGreaterThanOrEqual(48)
      expect
        .soft(retryBox.width, 'reference Retry must span the full recovery row')
        .toBeGreaterThanOrEqual(alertMetrics.width - 1)
      expect
        .soft(
          retryBox.y,
          'reference Retry must appear below the complete error description',
        )
        .toBeGreaterThanOrEqual(detailBox.y + detailBox.height)
      await retry.focus()
      await retry.press('Enter')
    }
    await expect(
      panel
        .locator('strong')
        .filter({ hasText: /^Registered ADE configuration$/ }),
    ).toBeVisible()
    await expect(
      panel.getByText('actual-ade-config', { exact: true }),
    ).toBeVisible()
    if (catalog) {
      const reads = await page.evaluate(
        () => window.__WORKERS_HARNESS__.calls.length,
      )
      await reference.focus()
      await reference.press('Enter')
      await expect(panel).toBeHidden()
      await reference.press('Enter')
      await expect(
        panel.getByText('actual-ade-config', { exact: true }),
      ).toBeVisible()
      expect(
        await page.evaluate(() => window.__WORKERS_HARNESS__.calls.length),
        'Collapsing and reopening a chosen reference must retain it without new reads',
      ).toBe(reads)
    }
    const port = panel.getByRole('listitem').filter({
      has: page.locator('code').filter({
        hasText: scenario.mode === 'constraints' ? /^http_port$/ : /^port$/,
      }),
    })
    await expect(port).toContainText('Type: integer')
    const registeredDefault = port.getByText('Registered default: 3113', {
      exact: true,
    })
    await expect(registeredDefault).toBeVisible()
    expect(
      (await textPresentation(registeredDefault)).contrast,
    ).toBeGreaterThanOrEqual(4.5)
    const token = panel.getByRole('listitem').filter({
      has: page.locator('code').filter({ hasText: /^api_token$/ }),
    })
    await expect(token).toContainText('Type: string')
    await expect(token).toContainText('No declared default')
    if (scenario.mode === 'constraints') {
      await expect(port).toContainText('Minimum: 0')
      await expect(port).toContainText('Maximum: 65535')
      const host = panel.getByRole('listitem').filter({
        has: page.locator('code').filter({ hasText: /^http_host$/ }),
      })
      await expect(host).toContainText('Min length: 1')
      await expect(host).toContainText('Registered default: "0.0.0.0"')
      const directory = panel
        .getByRole('listitem')
        .filter({ has: page.locator('code').filter({ hasText: /^data_dir$/ }) })
      await expect(directory).toContainText('Min length: 1')
      await expect(directory).toContainText('No declared default')
      await expect(panel).not.toContainText('Required in registered schema')
    }
    const presentation = await textPresentation(reference)
    expect(presentation.height).toBeGreaterThanOrEqual(
      scenario.width < 640 ? 48 : 32,
    )
    const complete = panel
      .locator('summary')
      .filter({ hasText: /^Complete registered schema$/ })
    await complete.focus()
    await complete.press('Enter')
    expect((await textPresentation(complete)).height).toBeGreaterThanOrEqual(
      scenario.width < 640 ? 48 : 32,
    )
    const schema = panel.getByLabel(
      'Complete registered schema for Registered ADE configuration',
      { exact: true },
    )
    await expect(schema).toContainText('"default": 3113')
    await schema.focus()
    await expect(schema).toBeFocused()
    await expect(schema).toBeInViewport()
    await expect(command).toHaveValue(editedCommand)
    expect(
      (await page.evaluate(() => window.__WORKERS_HARNESS__.editorSnapshot()))
        .value,
    ).toBe('port: 3114')
    await expect(
      page.getByRole('button', { name: 'Save and restart', exact: true }),
    ).toBeEnabled()
    expect(await editCalls(page)).toEqual([])
    const calls = await page.evaluate(() => window.__WORKERS_HARNESS__.calls)
    expect(
      calls.filter(
        (call) =>
          call.functionId.startsWith('configuration::') &&
          !['configuration::list', 'configuration::schema'].includes(
            call.functionId,
          ),
      ),
    ).toEqual([])
    if (!catalog) {
      expect(calls).toContainEqual({
        functionId: 'console::configuration-id',
        payload: {},
        options: { namespace: 'workers-test' },
      })
      expect(calls).toContainEqual({
        functionId: 'configuration::schema',
        payload: { id: 'actual-ade-config' },
      })
    }
    if (scenario.mode === 'constraints' && catalog) {
      await panel
        .getByRole('button', {
          name: 'Configuration reference entry',
          exact: true,
        })
        .click()
      await page
        .getByRole('option', { name: /^Registered harness configuration/ })
        .click()
      const reconciliation = panel.getByRole('listitem').filter({
        has: page.locator('code').filter({ hasText: /^call_reconciliation$/ }),
      })
      await expect(reconciliation).toContainText(
        'Allowed values: ["off","coerce","judge"]',
      )
      await expect(reconciliation).toContainText('Registered default: "judge"')
      await expect(reconciliation).not.toContainText(
        'Required in registered schema',
      )
      const harnessSchema = panel.getByLabel(
        'Complete registered schema for Registered harness configuration',
        { exact: true },
      )
      if (!(await harnessSchema.isVisible())) {
        await complete.focus()
        await complete.press('Enter')
      }
      await expect(harnessSchema).toContainText('"CallReconciliation"')
      await expect(harnessSchema).toContainText('"off"')
      await harnessSchema.focus()
      await expect(harnessSchema).toBeFocused()
      await expect(harnessSchema).toBeInViewport()
      await expect(command).toHaveValue(editedCommand)
      expect(
        (await page.evaluate(() => window.__WORKERS_HARNESS__.editorSnapshot()))
          .value,
      ).toBe('port: 3114')
      expect(await editCalls(page)).toEqual([])
      const currentCalls = await page.evaluate(
        () => window.__WORKERS_HARNESS__.calls,
      )
      expect(
        currentCalls.filter(
          (call) =>
            call.functionId.startsWith('configuration::') &&
            !['configuration::list', 'configuration::schema'].includes(
              call.functionId,
            ),
        ),
      ).toEqual([])
    }
    if (scenario.mode === 'identity') {
      await selectWorker(page, 'agent')
      await page.getByRole('tab', { name: /^Settings/ }).click()
      await reference.focus()
      await reference.press('Enter')
      await expect(
        panel.getByText('actual-agent-config', { exact: true }),
      ).toBeVisible()
      await expect(panel).toContainText('poll_interval')
      await expect(panel).toContainText('Registered default: "30s"')
      await selectWorker(page, 'ade')
      await page.getByRole('tab', { name: /^Settings/ }).click()
      await expect(command).toHaveValue(editedCommand)
      await expect
        .poll(
          async () =>
            (
              await page.evaluate(() =>
                window.__WORKERS_HARNESS__.editorSnapshot(),
              )
            ).value,
        )
        .toBe('port: 3114')
      expect(await editCalls(page)).toEqual([])
    }
  })
}

for (const theme of ['dark', 'light']) {
  for (const scenario of [
    { width: 1656, height: 1314, narrow: false },
    { width: 1656, height: 1314, narrow: true, pane: 720 },
    { width: 390, height: 844, narrow: true },
    { width: 450, height: 900, narrow: true },
  ]) {
    test(`provides a thirty-two-pixel desktop Build on start control while retaining narrow touch sizing and the actual command patch in ${theme} at ${scenario.width}${'pane' in scenario ? `/${scenario.pane} pane` : ''}`, async ({
      page,
    }) => {
      const { narrow } = scenario
      await page.setViewportSize({
        width: scenario.width,
        height: scenario.height,
      })
      await openSettings(
        page,
        `?theme=${theme}${'pane' in scenario ? '&pane=720' : ''}`,
      )
      const build = page.locator('button.wk-suggestion')
      await build.scrollIntoViewIfNeeded()
      await expect
        .soft(build)
        .toHaveAccessibleName(
          /^Use build command.*cargo run --locked --bin ade/,
          { timeout: 500 },
        )
      const backgroundAlpha = await build.evaluate((element) => {
        const canvas = document.createElement('canvas')
        canvas.width = canvas.height = 1
        const context = canvas.getContext('2d')
        if (!context)
          throw new Error('Canvas is unavailable for button paint measurement')
        context.fillStyle = getComputedStyle(element).backgroundColor
        context.fillRect(0, 0, 1, 1)
        return context.getImageData(0, 0, 1, 1).data[3]
      })
      expect
        .soft(
          backgroundAlpha,
          `${theme} build replacement must look like an actionable button at rest`,
        )
        .toBeGreaterThan(0)
      const presentation = await textPresentation(build)
      expect
        .soft(
          presentation.height,
          `${theme} ${narrow ? 'narrow' : 'desktop'} Build on start height`,
        )
        .toBeGreaterThanOrEqual(narrow ? 48 : 32)
      const labelBox = await build.locator('span').first().boundingBox()
      const commandBox = await build.locator('.wk-mono').boundingBox()
      if (!labelBox || !commandBox)
        throw new Error('Build label and exact command must render')
      if (narrow) {
        expect(commandBox.y).toBeGreaterThanOrEqual(
          labelBox.y + labelBox.height,
        )
        expect(commandBox.x).toBe(labelBox.x)
      } else {
        expect(commandBox.x).toBeGreaterThanOrEqual(labelBox.x + labelBox.width)
      }
      expect(presentation.scrollWidth).toBeLessThanOrEqual(
        presentation.clientWidth,
      )
      const buildId = `build-layout-${theme}-${scenario.width}${'pane' in scenario ? '-720-pane' : ''}`
      await test.info().attach(buildId, {
        body: JSON.stringify(
          { scenario, presentation, labelBox, commandBox },
          null,
          2,
        ),
        contentType: 'application/json',
      })
      const screenshot = test.info().outputPath(`${buildId}.png`)
      await page.screenshot({ path: screenshot })
      await test
        .info()
        .attach(buildId, { path: screenshot, contentType: 'image/png' })
      await build.focus()
      await build.press('Enter')
      await expect(
        page.getByRole('textbox', { name: 'Command', exact: true }),
      ).toHaveValue(editedCommand)
      await page
        .getByRole('button', { name: 'Save and restart', exact: true })
        .click()
      await expect(
        page.getByRole('region', {
          name: 'Worker settings result',
          exact: true,
        }),
      ).toContainText('Settings saved for ade')
      expect(await editCalls(page)).toHaveLength(1)
      expect((await editCalls(page))[0].payload.run).toBe(editedCommand)
    })
  }
}

test('distinguishes a confirmed write awaiting verification from new unsaved edits and reloads safely', async ({
  page,
}) => {
  const command = await openSettings(page, '?commands=1')
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.saveMode = 'reload-error'
  })
  await command.fill(editedCommand)
  const save = page.getByRole('button', {
    name: 'Save and restart',
    exact: true,
  })
  await save.click()
  await expect(
    page.getByText('Settings saved; declaration could not be reloaded', {
      exact: true,
    }),
  ).toBeVisible()
  const apply = page.getByRole('region', {
    name: 'Apply worker settings',
    exact: true,
  })
  await expect(save).toBeDisabled()
  await expect(apply).toContainText(
    /saved.*(?:awaiting|verification|reload|unverified)/i,
  )
  await expect(apply.getByText(/pending change/)).toHaveCount(0)
  await expect(page.getByText('unsaved', { exact: true })).toHaveCount(0)
  expect(
    await page.evaluate(
      () =>
        window.__WORKERS_HARNESS__
          .commands()
          .find((command) => command.id === 'save-settings')?.enabled,
    ),
  ).toBe(false)
  await command.press('ControlOrMeta+s')
  expect(await editCalls(page)).toHaveLength(1)

  const afterWrite = `${editedCommand} --release`
  await command.fill(afterWrite)
  await expect(save).toBeEnabled()
  await expect(apply).toContainText('1 pending change')
  await expect(page.getByText('unsaved', { exact: true })).toBeVisible()
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.saveMode = 'success'
    window.__WORKERS_HARNESS__.readDelay = 600
  })
  await page
    .getByRole('button', { name: 'Reload saved settings', exact: true })
    .click()
  await expect(command).toBeDisabled()
  await expect(
    page.getByText('Saved settings reloaded', { exact: true }),
  ).toBeVisible()
  await expect(command).toHaveValue(afterWrite)
  await expect(save).toBeEnabled()
  await expect(apply).toContainText('1 pending change')
  expect(await editCalls(page)).toHaveLength(1)
  await save.click()
  await expect(
    page.getByText('Settings saved for ade', { exact: true }).first(),
  ).toBeVisible()
  expect((await editCalls(page)).map((call) => call.payload.run)).toEqual([
    editedCommand,
    afterWrite,
  ])
})

test('explains native YAML Tab indentation and the platform focus-navigation toggle for keyboard users', async ({
  page,
}) => {
  for (const platform of [
    {
      agent: 'Mozilla/5.0 (X11; Linux x86_64)',
      hint: /Tab indents.*Ctrl\+M.*Tab move between controls/i,
      key: 'Control+m',
      theme: 'dark',
      width: 390,
      height: 844,
    },
    {
      agent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)',
      hint: /Tab indents.*Ctrl\+Shift\+M.*Tab move between controls/i,
      key: 'Control+Shift+m',
      theme: 'light',
      width: 450,
      height: 900,
    },
  ]) {
    await page.addInitScript((agent) => {
      Object.defineProperty(navigator, 'userAgent', {
        value: agent,
        configurable: true,
      })
    }, platform.agent)
    await page.setViewportSize({
      width: platform.width,
      height: platform.height,
    })
    const command = await openSettings(
      page,
      `?commands=1&theme=${platform.theme}`,
    )
    const configuration = Array.from(
      { length: 100 },
      (_, index) => `setting_${index + 1}: ${index + 1}`,
    ).join('\n')
    await page.evaluate((config_override) => {
      window.__WORKERS_HARNESS__.externalEdit('ade', { config_override })
      window.__WORKERS_HARNESS__.runCommand('refresh')
    }, configuration)
    await expect(page.locator('.wk-code .monaco-editor')).toBeVisible()
    await expect(async () => {
      const loaded = await page.evaluate(() =>
        window.__WORKERS_HARNESS__.editorSnapshot(),
      )
      expect(loaded.value).toBe(configuration)
      expect(loaded.value.split('\n')).toHaveLength(100)
    }).toPass({ timeout: 5000 })
    await command.fill(editedCommand)
    await expect(page.locator('.wk-code .monaco-editor')).toBeVisible()
    const editor = page.getByRole('textbox', {
      name: 'Configuration override YAML',
      exact: true,
    })
    const section = page.getByRole('region', {
      name: 'Configuration override',
      exact: true,
    })
    const heading = section.getByRole('heading', {
      name: 'Configuration override',
      exact: true,
    })
    await heading.scrollIntoViewIfNeeded()
    const hint = page.getByText(platform.hint)
    const reference = section
      .locator('summary')
      .filter({ hasText: /^Configuration reference$/ })
    await expect(hint).toBeVisible()
    await expect(editor).toHaveAccessibleDescription(platform.hint)
    const editorBox = await page.locator('.wk-code').boundingBox()
    const hintBox = await hint.boundingBox()
    const referenceBox = await reference.boundingBox()
    const headingBox = await heading.boundingBox()
    if (!editorBox || !hintBox || !referenceBox || !headingBox)
      throw new Error('Long native editor and discoverable help must render')
    const id = `long-editor-help-${platform.width}-${platform.theme}`
    await test.info().attach(id, {
      body: JSON.stringify(
        {
          editorBox,
          hintBox,
          referenceBox,
          headingBox,
          hintTravel: hintBox.y - headingBox.y,
          referenceTravel: referenceBox.y - headingBox.y,
        },
        null,
        2,
      ),
      contentType: 'application/json',
    })
    const screenshot = test.info().outputPath(`${id}.png`)
    await page.screenshot({ path: screenshot })
    await test.info().attach(id, { path: screenshot, contentType: 'image/png' })
    expect
      .soft(
        hintBox.y + hintBox.height,
        'Keyboard help must precede growing code',
      )
      .toBeLessThanOrEqual(editorBox.y)
    expect
      .soft(
        referenceBox.y + referenceBox.height,
        'Configuration reference must precede growing code',
      )
      .toBeLessThanOrEqual(editorBox.y)
    await expect.soft(hint).toBeInViewport({ ratio: 1, timeout: 500 })
    await expect.soft(reference).toBeInViewport({ ratio: 1, timeout: 500 })
    await editor.focus()
    await page.keyboard.press('Tab')
    const indented = await page.evaluate(() =>
      window.__WORKERS_HARNESS__.editorSnapshot(),
    )
    expect(indented.focused).toBe(true)
    expect(indented.value).toMatch(/^\s+setting_1:/)
    expect(indented.value.split('\n')).toHaveLength(100)
    const readOnly = await editor.getAttribute('aria-readonly')
    await page.keyboard.press(platform.key)
    await page.keyboard.press('Shift+Tab')
    await expect.soft(reference).toBeFocused({ timeout: 500 })
    if (
      await reference.evaluate((element) => document.activeElement === element)
    ) {
      const disclosure = reference.locator('..')
      await expect(disclosure).not.toHaveAttribute('open')
      await reference.press('Enter')
      await expect(disclosure).toHaveAttribute('open')
      await reference.press('Enter')
      await expect(disclosure).not.toHaveAttribute('open')
      await reference.press('Tab')
      await expect(editor).toBeFocused()
      const returned = await page.evaluate(() =>
        window.__WORKERS_HARNESS__.editorSnapshot(),
      )
      expect(returned).toMatchObject({
        value: indented.value,
        position: indented.position,
        selection: indented.selection,
        focused: true,
      })
      expect(await editor.getAttribute('aria-readonly')).toBe(readOnly)
      await expect(command).toHaveValue(editedCommand)
    }
    expect(
      await page.evaluate(() => ({
        scrollY,
        headerY: document.querySelector('.wk-header')?.getBoundingClientRect()
          .y,
      })),
    ).toEqual({ scrollY: 0, headerY: 0 })

    await editor.focus()
    await page.keyboard.press(
      platform.key.includes('Shift') ? 'Meta+A' : 'Control+A',
    )
    await page.keyboard.type('- 1')
    await expect(async () => {
      const invalid = await page.evaluate(() =>
        window.__WORKERS_HARNESS__.editorSnapshot(),
      )
      expect(invalid.value).toBe('- 1')
    }).toPass({ timeout: 5000 })
    await expect(editor).toHaveAttribute('aria-invalid', 'true')
    await expect(editor).toHaveAccessibleDescription(/Tab indents.*mapping/i)
    const linkage = await editor.evaluate((element) => {
      const described =
        element.getAttribute('aria-describedby')?.split(/\s+/) ?? []
      const errorId = element.getAttribute('aria-errormessage')
      return {
        described,
        errorId,
        errorText: errorId
          ? document.getElementById(errorId)?.textContent
          : null,
      }
    })
    expect(linkage.described.length).toBeGreaterThanOrEqual(2)
    expect(linkage.described).toContain(linkage.errorId)
    expect(linkage.errorText).toMatch(/mapping/i)
    expect(await editCalls(page)).toEqual([])
  }
})

test('keeps actual diff tokens and the masked-secret change marker at AA contrast in both themes', async ({
  page,
}, testInfo) => {
  const measurements: {
    theme: string
    sample: string
    contrast: number
    background: number[]
  }[] = []
  for (const theme of ['dark', 'light']) {
    await openSettings(page, `?theme=${theme}`)
    await page.getByLabel('LOG_LEVEL value', { exact: true }).fill('debug')
    await page.getByRole('button', { name: 'Replace', exact: true }).click()
    const replacement = 'sk-r7-secret-never-render-this'
    await page.getByLabel('API_TOKEN value', { exact: true }).fill(replacement)
    await page
      .getByRole('button', { name: 'Preview changes', exact: true })
      .click()
    const review = page.getByRole('region', {
      name: 'Review pending changes',
      exact: true,
    })
    const tokenCases = [
      { label: 'unchanged key', line: 'context', text: 'theme' },
      { label: 'unchanged value', line: 'context', text: 'system' },
      { label: 'unchanged literal', line: 'context', text: 'false' },
      { label: 'added key', line: 'change-addition', text: 'LOG_LEVEL' },
      { label: 'deleted key', line: 'change-deletion', text: 'LOG_LEVEL' },
      {
        label: 'added intraline value',
        line: 'change-addition',
        text: 'debug',
      },
      {
        label: 'deleted intraline value',
        line: 'change-deletion',
        text: 'info',
      },
      {
        label: 'masked replacement marker',
        line: 'change-addition',
        text: '# new value',
      },
    ]
    for (const token of tokenCases) {
      const leaf = review
        .locator(
          `[data-line][data-line-type="${token.line}"] span:not(:has(span))`,
        )
        .filter({ hasText: new RegExp(`^\\s*${token.text}\\s*$`) })
      await expect(leaf).toHaveCount(1)
      await expect(leaf).toBeVisible()
      const presentation = await textPresentation(leaf)
      measurements.push({
        theme,
        sample: token.label,
        contrast: presentation.contrast,
        background: presentation.background,
      })
      expect
        .soft(presentation.contrast, `${theme} ${token.label}: ${token.text}`)
        .toBeGreaterThanOrEqual(4.5)
    }
    const punctuation = review
      .locator('[data-line-type="context"] span:not(:has(span))')
      .filter({ hasText: /^:$/ })
      .first()
    const punctuationPresentation = await textPresentation(punctuation)
    measurements.push({
      theme,
      sample: 'syntax punctuation',
      contrast: punctuationPresentation.contrast,
      background: punctuationPresentation.background,
    })
    expect
      .soft(punctuationPresentation.contrast, `${theme} syntax punctuation`)
      .toBeGreaterThanOrEqual(4.5)
    const fills = []
    for (const type of ['change-addition', 'change-deletion']) {
      const number = review
        .locator(
          `[data-gutter] [data-line-type="${type}"] [data-line-number-content]`,
        )
        .first()
      await expect(number).toBeVisible()
      const presentation = await textPresentation(number)
      measurements.push({
        theme,
        sample: `${type} gutter number`,
        contrast: presentation.contrast,
        background: presentation.background,
      })
      expect
        .soft(presentation.contrast, `${theme} ${type} gutter number`)
        .toBeGreaterThanOrEqual(4.5)
      fills.push(presentation.background)
    }
    expect
      .soft(fills[0][1], `${theme} addition fill retains green semantics`)
      .toBeGreaterThan(fills[0][0])
    expect
      .soft(fills[1][0], `${theme} deletion fill retains red semantics`)
      .toBeGreaterThan(fills[1][1])
    expect
      .soft(fills[0], `${theme} added and deleted fills remain distinct`)
      .not.toEqual(fills[1])
    expect(
      (await review.locator('[data-line]').allTextContents()).join('\n'),
    ).not.toContain(replacement)
    expect(await editCalls(page)).toEqual([])
    await page.goto(`${fixture}?diffCase=multiple&theme=${theme}`)
    const multiple = page.getByRole('region', {
      name: 'Native diff regression',
      exact: true,
    })
    for (const token of [
      { line: 'context', value: '19' },
      { line: 'change-deletion', value: '20' },
      { line: 'change-deletion', value: '280' },
      { line: 'change-addition', value: '200' },
      { line: 'change-addition', value: '2800' },
    ]) {
      const leaf = multiple
        .locator(
          `[data-line][data-line-type="${token.line}"] span:not(:has(span))`,
        )
        .filter({ hasText: new RegExp(`^\\s*${token.value}\\s*$`) })
      await expect(leaf).toHaveCount(1)
      await expect(leaf).toBeVisible()
      const presentation = await textPresentation(leaf)
      const sample = `multiple-hunk ${token.line} numeric ${token.value}`
      measurements.push({
        theme,
        sample,
        contrast: presentation.contrast,
        background: presentation.background,
      })
      expect
        .soft(presentation.contrast, `${theme} ${sample}`)
        .toBeGreaterThanOrEqual(4.5)
    }
    expect(await editCalls(page)).toEqual([])
  }
  await testInfo.attach('actual-diff-contrast', {
    body: JSON.stringify(measurements, null, 2),
    contentType: 'application/json',
  })
})

test('paints the multi-hunk Expand all action and supports actual native Tab Enter and Space expansion', async ({
  page,
}) => {
  test.setTimeout(90_000)
  for (const width of [390, 1656]) {
    await page.setViewportSize({ width, height: width === 390 ? 844 : 1314 })
    for (const theme of ['dark', 'light']) {
      for (const key of ['Enter', 'Space']) {
        await page.goto(`${fixture}?diffCase=multiple&theme=${theme}`)
        const review = page.getByRole('region', {
          name: 'Native diff regression',
          exact: true,
        })
        const all = review.locator('button[data-expand-all-button]').first()
        await expect(
          all,
          `${theme}/${width}: Expand all must actually be painted`,
        ).toBeVisible()
        await review.focus()
        let reached = false
        const stops = await review.locator('button[data-expand-button]').count()
        for (let index = 0; index <= stops; index++) {
          await page.keyboard.press('Tab')
          reached = await all.evaluate((element) => {
            const root = element.getRootNode()
            return root instanceof ShadowRoot && root.activeElement === element
          })
          if (reached) break
        }
        expect(
          reached,
          `${theme}/${width}: Expand all must be in real Tab order`,
        ).toBe(true)
        await expect(all).toBeFocused()
        await expect(all).toBeInViewport()
        const paint = await all.evaluate((element) => {
          const box = element.getBoundingClientRect()
          const root = element.getRootNode()
          const top =
            root instanceof ShadowRoot
              ? root.elementFromPoint(box.x + 1, box.y + 1)
              : null
          return {
            width: box.width,
            height: box.height,
            focused: element.matches(':focus-visible'),
            outline: parseFloat(getComputedStyle(element).outlineWidth),
            topPainted:
              top === element || (top != null && element.contains(top)),
          }
        })
        expect(paint.focused).toBe(true)
        expect(paint.outline).toBeGreaterThanOrEqual(2)
        expect(paint.topPainted).toBe(true)
        if (width === 390) {
          expect(paint.width).toBeGreaterThanOrEqual(48)
          expect(paint.height).toBeGreaterThanOrEqual(48)
        }
        const before = await review.locator('[data-line]').count()
        await page.keyboard.press(key)
        await expect
          .poll(() => review.locator('[data-line]').count())
          .toBeGreaterThan(before)
        await expect(
          review
            .locator('[data-line]')
            .filter({ hasText: /^setting_100: 100$/ }),
        ).toHaveCount(1)
        await expect(all).toHaveCount(0)
        expect(await editCalls(page)).toEqual([])
      }
    }
  }
})

test('reveals Settings results and review inside the worker pane without scrolling outer chrome', async ({
  page,
}) => {
  for (const viewport of [
    { width: 390, height: 844, parameters: '?commands=1' },
    { width: 1656, height: 1314, parameters: '?commands=1&pane=720' },
  ]) {
    await page.setViewportSize({
      width: viewport.width,
      height: viewport.height,
    })
    const command = await openSettings(page, viewport.parameters)
    await expect(page.locator('.wk-code .monaco-editor')).toBeVisible()
    expect(await page.evaluate(() => scrollY)).toBe(0)
    await command.fill(editedCommand)
    await scrollSettingsBottom(page)
    await page
      .getByRole('button', { name: 'Save and restart', exact: true })
      .click()
    const result = page.getByRole('region', {
      name: 'Worker settings result',
      exact: true,
    })
    await expect(result).toContainText('Settings saved for ade')
    await expect(result).toBeFocused()
    await expect(result).toBeInViewport({ ratio: 0.99 })
    expect(
      await page.evaluate(() => scrollY),
      `${viewport.width}: result reveal must not scroll the outer document`,
    ).toBe(0)
    expect((await page.locator('.wk-header').boundingBox())?.y).toBe(0)
    await command.fill(`${editedCommand} --release`)
    await scrollSettingsBottom(page)
    await page
      .getByRole('button', { name: 'Preview changes', exact: true })
      .click()
    const review = page.getByRole('region', {
      name: 'Review pending changes',
      exact: true,
    })
    await expect(review).toBeInViewport({ ratio: 0.99 })
    expect(
      await page.evaluate(() => scrollY),
      `${viewport.width}: review reveal must not scroll the outer document`,
    ).toBe(0)
    expect((await page.locator('.wk-header').boundingBox())?.y).toBe(0)
    expect(await editCalls(page)).toHaveLength(1)
  }
  await page.setViewportSize({ width: 390, height: 844 })
  await openSettings(page, '?commands=1')
  const lines = Array.from(
    { length: 100 },
    (_, index) => `setting_${index + 1}: ${index + 1}`,
  )
  await page.evaluate((config_override) => {
    window.__WORKERS_HARNESS__.externalEdit('ade', { config_override })
    window.__WORKERS_HARNESS__.runCommand('refresh')
  }, lines.join('\n'))
  await expect(page.locator('.wk-code .monaco-editor')).toBeVisible()
  await expect
    .poll(
      async () =>
        (await page.evaluate(() => window.__WORKERS_HARNESS__.editorSnapshot()))
          .value,
    )
    .toBe(lines.join('\n'))
  const last = page
    .locator('.wk-code .view-lines .view-line')
    .filter({ hasText: 'setting_100:' })
    .last()
  await scrollRenderedLineIntoView(last)
  const clickLast = async () => {
    const box = await last.boundingBox()
    if (!box) throw new Error('The last rendered YAML line must be visible')
    await page.mouse.click(box.x + 20, box.y + box.height / 2)
    await page.keyboard.press('End')
  }
  await clickLast()
  await page.keyboard.type('0')
  lines[99] += '0'
  const expected = lines.join('\n')
  expect(
    await page.evaluate(() => window.__WORKERS_HARNESS__.editorSnapshot()),
  ).toMatchObject({
    value: expected,
    focused: true,
    position: { lineNumber: 100, column: lines[99].length + 1 },
  })
  await scrollSettingsBottom(page)
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await expect(
    page.getByRole('region', { name: 'Worker settings result', exact: true }),
  ).toContainText('Settings saved for ade')
  expect(await editCalls(page)).toHaveLength(1)
  expect((await editCalls(page))[0].payload.config_override).toBe(expected)
  expect(await page.evaluate(() => scrollY)).toBe(0)
  await scrollRenderedLineIntoView(last)
  await clickLast()
  // Isolate the native keyboard reveal from any browser locator scrolling.
  await page.evaluate(() => window.scrollTo(0, 0))
  const focused = page.locator(
    '.wk-code .native-edit-context:focus, .wk-code textarea.inputarea:focus',
  )
  for (const key of ['Control+Home', 'Control+End']) {
    await page.keyboard.press(key)
    await expect(focused).toBeInViewport()
    expect(
      await page.evaluate(() => scrollY),
      `after-save ${key} must stay within the worker pane`,
    ).toBe(0)
    expect((await page.locator('.wk-header').boundingBox())?.y).toBe(0)
    expect(
      await page.evaluate(() => window.__WORKERS_HARNESS__.editorSnapshot()),
    ).toMatchObject({
      value: expected,
      focused: true,
      position:
        key === 'Control+Home'
          ? { lineNumber: 1, column: 1 }
          : { lineNumber: 100, column: lines[99].length + 1 },
    })
  }
  expect(await editCalls(page)).toHaveLength(1)
})

test('reveals bottom-save results for success and each failure mode on desktop and phone', async ({
  page,
}) => {
  test.setTimeout(90_000)
  for (const width of [1000, 390]) {
    for (const mode of [
      'success',
      'error',
      'restart-error',
      'reload-error',
    ] as const) {
      await test.step(`${width}px ${mode}`, async () => {
        await page.setViewportSize({ width, height: width === 390 ? 844 : 900 })
        const command = await openSettings(page, '?commands=1')
        await command.fill(editedCommand)
        await page.evaluate((mode) => {
          window.__WORKERS_HARNESS__.saveMode = mode
        }, mode)
        await scrollSettingsBottom(page)
        await page
          .getByRole('button', { name: 'Save and restart', exact: true })
          .click()
        const headline =
          mode === 'success'
            ? 'Settings saved for ade'
            : mode === 'error'
              ? 'The save did not complete'
              : mode === 'restart-error'
                ? 'Configuration saved; restart failed'
                : 'Settings saved; declaration could not be reloaded'
        const result = page.getByText(headline, { exact: true }).first()
        await expect(result).toBeVisible()
        await expect
          .soft(result, `${width}px ${mode} must reveal its result`)
          .toBeInViewport({ timeout: 1200 })
      })
    }
  }
})

async function settingsFocusPaint(page: Page) {
  return page.evaluate(async () => {
    await new Promise((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(resolve)),
    )
    let active = document.activeElement
    while (active?.shadowRoot?.activeElement)
      active = active.shadowRoot.activeElement
    const bar = document.querySelector('.wk-applybar')
    const pane = document.querySelector('.wk-main')
    if (!(active instanceof HTMLElement) || !bar || !pane)
      throw new Error('Settings focus, toolbar, and owning pane must exist')
    const bounds = active.getBoundingClientRect()
    const barBounds = bar.getBoundingClientRect()
    const root = active.getRootNode()
    const center = (
      root instanceof ShadowRoot ? root : document
    ).elementFromPoint(
      bounds.left + bounds.width / 2,
      bounds.top + bounds.height / 2,
    )
    return {
      tag: active.tagName,
      connected: active.isConnected,
      inSettings:
        !!active.closest('.wk-settings') ||
        (root instanceof ShadowRoot && !!root.host.closest('.wk-settings')),
      inReview:
        !!active.closest('.wk-review') ||
        (root instanceof ShadowRoot && !!root.host.closest('.wk-review')),
      label: active.getAttribute('aria-label') || active.textContent,
      top: bounds.top,
      bottom: bounds.bottom,
      barBottom: barBounds.bottom,
      painted: center === active || (center != null && active.contains(center)),
      paintedElement: center?.outerHTML.slice(0, 300),
      focusVisible: active.matches(':focus-visible'),
      indicator:
        (parseFloat(getComputedStyle(active).outlineWidth) > 0 &&
          getComputedStyle(active).outlineStyle !== 'none') ||
        getComputedStyle(active).boxShadow !== 'none',
      inApplyBar: !!active.closest('.wk-applybar'),
      paneScroll: pane.scrollTop,
      outerScroll: scrollY,
      headerY: document.querySelector('.wk-header')?.getBoundingClientRect().y,
    }
  })
}

test('keeps ordinary Settings recovery keyboard focus painted below the sticky toolbar', async ({
  page,
}) => {
  test.setTimeout(90_000)
  for (const scenario of [
    {
      width: 1656,
      theme: 'light',
      state: 'restore',
      target: 'Restore previous settings',
    },
    {
      width: 390,
      theme: 'dark',
      state: 'restore',
      target: 'Restore previous settings',
    },
    { width: 1656, theme: 'dark', state: 'undo', target: 'Undo discard' },
    { width: 450, theme: 'light', state: 'undo', target: 'Undo discard' },
    {
      width: 1656,
      theme: 'dark',
      state: 'reload',
      target: 'Reload saved settings',
      pane: 720,
    },
    {
      width: 390,
      theme: 'light',
      state: 'reload',
      target: 'Reload saved settings',
    },
    { width: 1656, theme: 'light', state: 'retry', target: 'Retry restart' },
    { width: 450, theme: 'dark', state: 'retry', target: 'Retry restart' },
  ]) {
    await page.setViewportSize({
      width: scenario.width,
      height: scenario.width < 500 ? 900 : 1314,
    })
    const command = await openSettings(
      page,
      `?commands=1&theme=${scenario.theme}${'pane' in scenario ? `&pane=${scenario.pane}` : ''}`,
    )
    await command.fill(editedCommand)
    if (scenario.state === 'undo') {
      await page.getByRole('button', { name: 'Discard', exact: true }).click()
    } else {
      await page.evaluate((state) => {
        window.__WORKERS_HARNESS__.saveMode =
          state === 'reload'
            ? 'reload-error'
            : state === 'retry'
              ? 'restart-error'
              : 'success'
      }, scenario.state)
      await page
        .getByRole('button', { name: 'Save and restart', exact: true })
        .click()
      await expect(
        page.getByRole('region', {
          name: 'Worker settings result',
          exact: true,
        }),
      ).toContainText(
        scenario.state === 'restore'
          ? 'Settings saved for ade'
          : scenario.state === 'reload'
            ? 'Settings saved; declaration could not be reloaded'
            : 'Configuration saved; restart failed',
      )
    }
    if (scenario.state === 'restore') await openPreviousSettings(page)
    if (scenario.state !== 'retry') {
      await command.fill(`${editedCommand} --pending`)
      await page
        .getByRole('button', { name: 'Preview changes', exact: true })
        .click()
    }
    const section = page.getByRole('region', {
      name:
        scenario.state === 'retry'
          ? 'Worker settings result'
          : 'Review pending changes',
      exact: true,
    })
    await expect(section).toBeInViewport()
    // Stage an ordinarily scrolled pane; all subsequent focus movement is
    // native Tab/Shift+Tab, never target.focus() or a synthetic focus event.
    await section.evaluate((element) => {
      const pane = element.closest('.wk-main')
      const bar = document.querySelector('.wk-applybar')
      if (!pane || !bar)
        throw new Error('The section must belong to its worker pane')
      pane.scrollTop +=
        element.getBoundingClientRect().top -
        bar.getBoundingClientRect().bottom -
        6
    })
    if (scenario.state === 'retry') await section.focus()
    const target = page.getByRole('button', {
      name: scenario.target,
      exact: true,
    })
    const trail = []
    for (
      let step = 0;
      step < 8 &&
      !(await target.evaluate((element) => document.activeElement === element));
      step += 1
    ) {
      const before = await page
        .locator('.wk-main')
        .evaluate((element) => element.scrollTop)
      await page.keyboard.press('Tab')
      const paint = await settingsFocusPaint(page)
      trail.push(paint)
      expect
        .soft(
          paint.painted,
          `${scenario.target}: every native forward stop must be top-painted`,
        )
        .toBe(true)
      if (paint.inApplyBar)
        expect
          .soft(
            Math.abs(paint.paneScroll - before),
            'Tab among sticky actions must not jump the pane',
          )
          .toBeLessThanOrEqual(1)
    }
    await expect(target).toBeFocused()
    await assertSettingsFocusPaint(page, scenario.target)
    const beforeBack = await page
      .locator('.wk-main')
      .evaluate((element) => element.scrollTop)
    await page.keyboard.press('Shift+Tab')
    const back = await settingsFocusPaint(page)
    trail.push(back)
    expect
      .soft(
        back.painted,
        'ordinary reverse traversal must retain painted focus',
      )
      .toBe(true)
    if (back.inApplyBar)
      expect
        .soft(
          Math.abs(back.paneScroll - beforeBack),
          'Shift+Tab into sticky actions must not jump the pane',
        )
        .toBeLessThanOrEqual(1)
    await page.keyboard.press('Tab')
    await expect(target).toBeFocused()
    await assertSettingsFocusPaint(page, `${scenario.target} forward return`)
    if (scenario.state === 'restore' || scenario.state === 'undo') {
      await command.focus()
      await page.keyboard.press('Shift+Tab')
      const diffFocus = await assertSettingsFocusPaint(
        page,
        'native shadow diff reverse traversal',
      )
      expect(diffFocus.label).toMatch(/Expand.*unchanged lines/)
      await page.keyboard.press('Tab')
      await expect(command).toBeFocused()
      await assertSettingsFocusPaint(page, 'Command after native shadow diff')
    }
    await test
      .info()
      .attach(
        `ordinary-keyboard-focus-${scenario.width}-${scenario.theme}-${scenario.state}`,
        {
          body: JSON.stringify(trail, null, 2),
          contentType: 'application/json',
        },
      )
    expect(await editCalls(page)).toHaveLength(
      scenario.state === 'undo' ? 0 : 1,
    )
    expect(await page.evaluate(() => scrollY)).toBe(0)
    expect((await page.locator('.wk-header').boundingBox())?.y).toBe(0)
  }
})

async function assertSettingsFocusPaint(page: Page, label: string) {
  const paint = await settingsFocusPaint(page)
  expect
    .soft(paint.focusVisible, `${label}: native focus indication`)
    .toBe(true)
  expect
    .soft(
      paint.painted,
      `${label}: focused control must own its painted center`,
    )
    .toBe(true)
  if (!paint.inApplyBar)
    expect
      .soft(paint.top, `${label}: focus must clear sticky toolbar`)
      .toBeGreaterThanOrEqual(paint.barBottom + 2)
  expect(paint.outerScroll).toBe(0)
  expect(paint.headerY).toBe(0)
  return paint
}

test('keeps keyboard field focus below the sticky apply bar after tabbing from bottom Save', async ({
  page,
}) => {
  for (const width of [1000, 390]) {
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 })
    const command = await openSettings(page, '?commands=1')
    await command.fill(editedCommand)
    await scrollSettingsBottom(page)
    await page
      .getByRole('button', { name: 'Save and restart', exact: true })
      .focus()
    for (
      let step = 0;
      step < 4 &&
      !(await command.evaluate(
        (element) => document.activeElement === element,
      ));
      step += 1
    )
      await page.keyboard.press('Tab')
    await expect(command).toBeFocused()
    await assertFieldUnobscured(page)
  }
})

test('reveals a preview opened from the bottom of the form on desktop and phone', async ({
  page,
}) => {
  for (const width of [1000, 390]) {
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 })
    const command = await openSettings(page, '?commands=1')
    await command.fill(editedCommand)
    await scrollSettingsBottom(page)
    await page
      .getByRole('button', { name: 'Preview changes', exact: true })
      .click()
    const preview = page.getByRole('region', {
      name: 'Review pending changes',
      exact: true,
    })
    await expect(preview).toBeVisible()
    await expect(preview).toBeInViewport()
  }
})

test('leaves visible keyboard focus after a Mod+S result', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  const command = await openSettings(page, '?commands=1')
  await command.fill(editedCommand)
  await command.press('ControlOrMeta+s')
  await expect(
    page.getByText('Settings saved for ade', { exact: true }).first(),
  ).toBeVisible()
  const result = page.getByRole('region', {
    name: 'Worker settings result',
    exact: true,
  })
  await expect(result).toBeFocused()
  await expect(result).toBeInViewport()
})

test('loads and retains separate settings for special worker names', async ({
  page,
}) => {
  await page.goto(`${fixture}?specialWorkers=1`)
  for (const name of ['constructor', 'toString', '__proto__']) {
    await selectWorker(page, name)
    await page.getByRole('tab', { name: /^Settings/ }).click()
    const command = page.getByRole('textbox', { name: 'Command', exact: true })
    await expect(command).toHaveValue(`./target/debug/${name}`)
    await command.fill(`cargo run --bin ${name}`)
  }
  for (const name of ['constructor', 'toString', '__proto__']) {
    await selectWorker(page, name)
    await expect(
      page.getByRole('textbox', { name: 'Command', exact: true }),
    ).toHaveValue(`cargo run --bin ${name}`)
  }
})

test('refreshes a clean declaration on revisit and explicit Refresh while preserving drafts and edits during a slow read', async ({
  page,
}) => {
  const command = await openSettings(page, '?commands=1')
  await page.evaluate(() =>
    window.__WORKERS_HARNESS__.externalEdit('ade', {
      run: 'external command on revisit',
    }),
  )
  await selectWorker(page, 'agent')
  await selectWorker(page)
  await expect(command).toHaveValue('external command on revisit')
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.externalEdit('ade', {
      run: 'external command on refresh',
    })
    window.__WORKERS_HARNESS__.runCommand('refresh')
  })
  await expect(command).toHaveValue('external command on refresh')
  await command.fill(editedCommand)
  const snapshots = await page.evaluate(
    () =>
      window.__WORKERS_HARNESS__.calls.filter(
        (call) => call.functionId === 'compose::status',
      ).length,
  )
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.externalEdit('ade', {
      run: 'external command while dirty',
    })
    window.__WORKERS_HARNESS__.runCommand('refresh')
  })
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__WORKERS_HARNESS__.calls.filter(
            (call) => call.functionId === 'compose::status',
          ).length,
      ),
    )
    .toBeGreaterThan(snapshots)
  await expect(command).toHaveValue(editedCommand)
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.dirty))
    .toBeTruthy()
  await page.getByRole('button', { name: 'Discard', exact: true }).click()
  const beforeSlow = await page.evaluate(() => ({
    completed: window.__WORKERS_HARNESS__.readsCompleted,
    calls: window.__WORKERS_HARNESS__.calls.filter(
      (call) => call.functionId === 'console::compose::container',
    ).length,
  }))
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.readDelay = 800
    window.__WORKERS_HARNESS__.externalEdit('ade', {
      run: 'external command during slow read',
    })
    window.__WORKERS_HARNESS__.runCommand('refresh')
  })
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__WORKERS_HARNESS__.calls.filter(
            (call) => call.functionId === 'console::compose::container',
          ).length,
      ),
    )
    .toBeGreaterThan(beforeSlow.calls)
  await command.fill('local edit during slow read')
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.readsCompleted))
    .toBeGreaterThan(beforeSlow.completed)
  await expect(command).toHaveValue('local edit during slow read')
})

test('keeps the Monaco caret visible when clicking and typing on the last line of a long configuration', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1000, height: 900 })
  await openSettings(page, '?commands=1')
  const configuration = Array.from(
    { length: 100 },
    (_, index) =>
      `key_${String(index + 1).padStart(3, '0')}: value_${index + 1}`,
  ).join('\n')
  await page.evaluate((config_override) => {
    window.__WORKERS_HARNESS__.externalEdit('ade', { config_override })
    window.__WORKERS_HARNESS__.runCommand('refresh')
  }, configuration)
  const lastLine = page
    .locator('.wk-code .view-lines .view-line')
    .filter({ hasText: 'key_100:' })
    .last()
  await expect(lastLine).toBeAttached()
  await scrollRenderedLineIntoView(lastLine)
  await lastLine.click({ position: { x: 150, y: 9 } })
  await page.keyboard.type('typed')
  await expect(lastLine).toContainText('typed')
  const focusedControl = page.locator(
    '.wk-code .native-edit-context:focus, .wk-code textarea.inputarea:focus',
  )
  await expect(focusedControl).toBeVisible()
  await expect(focusedControl).toBeInViewport()
  await expect(lastLine).toBeInViewport()
  const firstLine = page
    .locator('.wk-code .view-lines .view-line')
    .filter({ hasText: 'key_001:' })
    .first()
  await page.keyboard.press('Control+Home')
  await expect(focusedControl).toBeInViewport()
  await expect(firstLine).toBeInViewport()
  await page.keyboard.press('Control+End')
  await expect(focusedControl).toBeInViewport()
  await expect(lastLine).toBeInViewport()
})

async function assertColdPointerEdit(
  page: Page,
  size: { width: number; height: number; pane: string },
  targetLine: number,
  interaction: 'mouse' | 'touch' = 'mouse',
  theme = 'dark',
) {
  await page.setViewportSize({ width: size.width, height: size.height })
  await openSettings(page, `?commands=1&theme=${theme}${size.pane}`)
  const lines = Array.from(
    { length: 100 },
    (_, index) => `setting_${index + 1}: ${index + 1}`,
  )
  const configuration = lines.join('\n')
  await page.evaluate((config_override) => {
    window.__WORKERS_HARNESS__.externalEdit('ade', { config_override })
    window.__WORKERS_HARNESS__.runCommand('refresh')
  }, configuration)
  await expect(page.locator('.wk-code .monaco-editor')).toBeVisible()
  await expect
    .poll(() =>
      page.evaluate(
        async () => (await window.__WORKERS_HARNESS__.editorSnapshot()).value,
      ),
    )
    .toBe(configuration)
  const target = page
    .locator('.wk-code .view-lines .view-line')
    .filter({ hasText: new RegExp(`^setting_${targetLine}:`) })
    .first()
  await scrollRenderedLineIntoView(target)
  const before = await page.evaluate(() =>
    window.__WORKERS_HARNESS__.editorSnapshot(),
  )
  expect(
    before.focused,
    'the first pointer interaction must start from an unfocused editor',
  ).toBe(false)
  expect(before.position?.lineNumber).toBe(1)
  const point = await target.evaluate((element) => {
    const range = document.createRange()
    range.selectNodeContents(element)
    const text = range.getBoundingClientRect()
    const x = text.left + 15
    const y = text.top + text.height / 2
    if (!document.elementFromPoint(x, y)?.closest('.view-line'))
      throw new Error('The physical click must hit real rendered editor text')
    return { x, y }
  })
  if (interaction === 'touch') {
    await page.evaluate(() => {
      const events: Record<string, unknown>[] = []
      window.__COLD_TOUCH_EVENTS__ = events
      for (const type of [
        'pointerdown',
        'touchstart',
        'pointerup',
        'touchend',
        'focus',
        'mousedown',
      ]) {
        document.addEventListener(
          type,
          (event) => {
            const target = event.target
            if (!(target instanceof Element) || !target.closest('.wk-code'))
              return
            events.push({
              type,
              time: performance.now(),
              pointer: event instanceof PointerEvent ? event.pointerType : null,
              focused: document.activeElement?.className,
              paneScroll: document.querySelector('.wk-main')?.scrollTop,
              outerScroll: scrollY,
            })
          },
          true,
        )
      }
    })
    await page.touchscreen.tap(point.x, point.y)
  } else await page.mouse.click(point.x, point.y)
  const clicked = await page.evaluate(() =>
    window.__WORKERS_HARNESS__.editorSnapshot(),
  )
  if (interaction === 'touch') {
    await test.info().attach('native-cold-touch', {
      body: JSON.stringify(
        {
          targetLine,
          point,
          before: { focused: before.focused, position: before.position },
          clicked: { focused: clicked.focused, position: clicked.position },
          events: await page.evaluate(() => window.__COLD_TOUCH_EVENTS__),
        },
        null,
        2,
      ),
      contentType: 'application/json',
    })
  }
  expect(
    clicked.focused,
    'the physical pointer interaction must focus the native editor',
  ).toBe(true)
  expect
    .soft(
      clicked.position?.lineNumber,
      'one physical pointer interaction must position the caret on the intended line',
    )
    .toBe(targetLine)
  await page.keyboard.press('End')
  await page.keyboard.type('0')
  lines[targetLine - 1] += '0'
  const expectedConfiguration = lines.join('\n')
  await expect.soft
    .poll(() =>
      page.evaluate(
        async () => (await window.__WORKERS_HARNESS__.editorSnapshot()).value,
      ),
    )
    .toBe(expectedConfiguration)
  await expect.soft
    .poll(() =>
      page.evaluate(
        async () =>
          (await window.__WORKERS_HARNESS__.editorSnapshot()).position,
      ),
    )
    .toEqual({
      lineNumber: targetLine,
      column: lines[targetLine - 1].length + 1,
    })
  const nativeControl = page.locator(
    '.wk-code .native-edit-context:focus, .wk-code textarea.inputarea:focus',
  )
  await expect(nativeControl).toBeInViewport()
  expect(await page.evaluate(() => scrollY)).toBe(0)
  expect((await page.locator('.wk-header').boundingBox())?.y).toBe(0)
  await page.keyboard.press('ControlOrMeta+S')
  await expect(
    page.getByText('Settings saved for ade', { exact: true }),
  ).toBeVisible()
  const writes = await editCalls(page)
  expect(writes).toHaveLength(1)
  expect(
    writes[0].payload.config_override,
    'the persisted mapping must change only the clicked key',
  ).toBe(expectedConfiguration)
  expect(await page.evaluate(() => scrollY)).toBe(0)
  expect((await page.locator('.wk-header').boundingBox())?.y).toBe(0)
}

for (const size of [
  { name: 'phone', width: 390, height: 844, pane: '' },
  { name: '450px', width: 450, height: 900, pane: '' },
  { name: '720px pane', width: 1656, height: 1314, pane: '&pane=720' },
  { name: 'desktop', width: 1656, height: 1314, pane: '' },
]) {
  for (const targetLine of [50, 100]) {
    test(`edits and saves the intended line on the first cold pointer click at ${size.name} line ${targetLine}`, async ({
      page,
    }) => {
      await assertColdPointerEdit(page, size, targetLine)
    })
  }
}

test.describe('cold touchscreen editing', () => {
  test.use({ hasTouch: true })
  test('preserves settled native caret and exact subsequent edit after a touch pan from another code line', async ({
    page,
  }) => {
    test.setTimeout(120_000)
    const scenarios = [390, 450].flatMap((width) =>
      ['dark', 'light'].flatMap((theme) =>
        ['up', 'down'].map((direction) => ({
          width,
          theme,
          direction,
          selection: 'caret',
          origin: 'other',
        })),
      ),
    )
    scenarios.push(
      {
        width: 390,
        theme: 'light',
        direction: 'up',
        selection: 'caret',
        origin: 'current',
      },
      {
        width: 450,
        theme: 'dark',
        direction: 'down',
        selection: 'caret',
        origin: 'current',
      },
      {
        width: 390,
        theme: 'dark',
        direction: 'up',
        selection: 'word',
        origin: 'other',
      },
      {
        width: 390,
        theme: 'light',
        direction: 'down',
        selection: 'word',
        origin: 'other',
      },
      {
        width: 450,
        theme: 'dark',
        direction: 'down',
        selection: 'word',
        origin: 'other',
      },
      {
        width: 450,
        theme: 'light',
        direction: 'up',
        selection: 'word',
        origin: 'other',
      },
    )
    for (const scenario of scenarios) {
      const { width, theme, direction, selection, origin } = scenario
      await page.setViewportSize({ width, height: 900 })
      await openSettings(page, `?commands=1&theme=${theme}`)
      const lines = Array.from(
        { length: 100 },
        (_, index) => `setting_${index + 1}: ${index + 1}`,
      )
      const original = lines.join('\n')
      await page.evaluate((config_override) => {
        window.__WORKERS_HARNESS__.externalEdit('ade', { config_override })
        window.__WORKERS_HARNESS__.runCommand('refresh')
      }, original)
      await expect(page.locator('.wk-code .monaco-editor')).toBeVisible()
      await expect
        .poll(() =>
          page.evaluate(
            async () =>
              (await window.__WORKERS_HARNESS__.editorSnapshot()).value,
          ),
        )
        .toBe(original)
      await scrollRenderedLineIntoView(
        page
          .locator('.wk-code .view-line')
          .filter({ hasText: /^setting_50:/ })
          .first(),
      )
      expect(
        await page.evaluate(() => window.__WORKERS_HARNESS__.editorSnapshot()),
      ).toMatchObject({
        value: original,
        focused: false,
        position: { lineNumber: 1, column: 1 },
      })
      const tap = await renderedSpecificCodePoint(
        page,
        50,
        selection === 'word' ? 'key' : 'end',
      )
      await page.touchscreen.tap(tap.x, tap.y)
      if (selection === 'word') await page.touchscreen.tap(tap.x, tap.y)
      // Wait beyond Monaco's double-tap window. A physical glyph tap chooses
      // the caret/word; no keypress, focus reset or setter cancels its monitor.
      await page.waitForTimeout(700)
      const before = await page.evaluate(() =>
        window.__WORKERS_HARNESS__.editorSnapshot(),
      )
      expect(before).toMatchObject({ value: original, focused: true })
      if (selection === 'caret') {
        expect(before.position).toEqual({
          lineNumber: 50,
          column: lines[49].length + 1,
        })
        expect(before.selection).toMatchObject({
          startLineNumber: 50,
          endLineNumber: 50,
          startColumn: lines[49].length + 1,
          endColumn: lines[49].length + 1,
        })
      } else {
        expect(before.selection).toMatchObject({
          startLineNumber: 50,
          endLineNumber: 50,
          startColumn: 1,
          endColumn: 'setting_50'.length + 1,
        })
      }
      const gestureLine =
        origin === 'current' ? 50 : direction === 'up' ? 55 : 45
      const start = await renderedSpecificCodePoint(page, gestureLine)
      const paneBefore = await page
        .locator('.wk-main')
        .evaluate((el) => el.scrollTop)
      await nativeTouchSwipe(
        page,
        start,
        start.y + (direction === 'up' ? -180 : 180),
      )
      const paneAfter = await page
        .locator('.wk-main')
        .evaluate((el) => el.scrollTop)
      const afterPan = await page.evaluate(() =>
        window.__WORKERS_HARNESS__.editorSnapshot(),
      )
      // The next real key must affect precisely the pre-gesture selection.
      // Do not click, press End, or refocus between the pan and this edit.
      await page.keyboard.type('0')
      const afterType = await page.evaluate(() =>
        window.__WORKERS_HARNESS__.editorSnapshot(),
      )
      lines[49] = selection === 'word' ? '0: 50' : `${lines[49]}0`
      await test
        .info()
        .attach(
          `settled-native-pan-${width}-${theme}-${direction}-${selection}-${origin}`,
          {
            body: JSON.stringify(
              {
                scenario,
                tap,
                start,
                paneBefore,
                paneAfter,
                before,
                afterPan,
                afterType,
              },
              null,
              2,
            ),
            contentType: 'application/json',
          },
        )
      expect(
        direction === 'up' ? paneAfter - paneBefore : paneBefore - paneAfter,
      ).toBeGreaterThan(80)
      expect.soft(afterPan).toMatchObject({
        value: before.value,
        position: before.position,
        selection: before.selection,
        focused: true,
      })
      expect(afterType.value).toBe(lines.join('\n'))
      expect(afterType.position).toEqual({
        lineNumber: 50,
        column: selection === 'word' ? 2 : lines[49].length + 1,
      })
      await expect(
        page.locator(
          '.wk-code .native-edit-context:focus, .wk-code textarea.inputarea:focus',
        ),
      ).toBeInViewport()
      expect(await page.evaluate(() => scrollY)).toBe(0)
      expect((await page.locator('.wk-header').boundingBox())?.y).toBe(0)
      await page.keyboard.press('ControlOrMeta+S')
      await expect(
        page.getByText('Settings saved for ade', { exact: true }),
      ).toBeVisible()
      const writes = await editCalls(page)
      expect(writes).toHaveLength(1)
      expect(writes[0].payload.config_override).toBe(lines.join('\n'))
      expect(await page.evaluate(() => scrollY)).toBe(0)
      expect((await page.locator('.wk-header').boundingBox())?.y).toBe(0)
    }
  })
  test('scrolls the owning worker pane with native swipes over growing code while preserving cold tap editing', async ({
    page,
  }) => {
    test.setTimeout(90_000)
    for (const width of [390, 450]) {
      for (const theme of ['dark', 'light']) {
        await page.setViewportSize({ width, height: 900 })
        await openSettings(page, `?commands=1&theme=${theme}`)
        const lines = Array.from(
          { length: 100 },
          (_, index) => `setting_${index + 1}: ${index + 1}`,
        )
        const original = lines.join('\n')
        await page.evaluate((config_override) => {
          window.__WORKERS_HARNESS__.externalEdit('ade', { config_override })
          window.__WORKERS_HARNESS__.runCommand('refresh')
        }, original)
        await expect(page.locator('.wk-code .monaco-editor')).toBeVisible()
        await expect
          .poll(() =>
            page.evaluate(
              async () =>
                (await window.__WORKERS_HARNESS__.editorSnapshot()).value,
            ),
          )
          .toBe(original)
        await scrollRenderedLineIntoView(
          page
            .locator('.wk-code .view-line')
            .filter({ hasText: /^setting_50:/ })
            .first(),
        )
        const gestures = []
        for (const direction of ['up', 'down'] as const) {
          const point = await renderedCodePoint(
            page,
            direction === 'up' ? 'lower' : 'upper',
          )
          const before = await page
            .locator('.wk-main')
            .evaluate((element) => element.scrollTop)
          await nativeTouchSwipe(
            page,
            point,
            point.y + (direction === 'up' ? -200 : 200),
          )
          const after = await page
            .locator('.wk-main')
            .evaluate((element) => element.scrollTop)
          gestures.push({ direction, point, before, after })
          expect
            .soft(
              direction === 'up' ? after - before : before - after,
              `${width}px ${theme} native ${direction} swipe on actual code must move its owning pane`,
            )
            .toBeGreaterThan(80)
          expect(
            await page.evaluate(() =>
              window.__WORKERS_HARNESS__.editorSnapshot(),
            ),
          ).toMatchObject({
            value: original,
            focused: false,
            position: { lineNumber: 1, column: 1 },
          })
          expect(await page.evaluate(() => scrollY)).toBe(0)
          expect((await page.locator('.wk-header').boundingBox())?.y).toBe(0)
        }
        // Positive input-method control: the same physical gesture on the
        // visible outer margin must scroll, without touching editor state.
        const marginPoint = await renderedCodePoint(page, 'lower')
        marginPoint.x = await page
          .locator('.wk-main')
          .evaluate((element) => element.getBoundingClientRect().left + 6)
        const marginBefore = await page
          .locator('.wk-main')
          .evaluate((element) => element.scrollTop)
        await nativeTouchSwipe(page, marginPoint, marginPoint.y - 200)
        const marginAfter = await page
          .locator('.wk-main')
          .evaluate((element) => element.scrollTop)
        expect(
          marginAfter - marginBefore,
          'foreground CDP touch input must actually scroll the pane at its margin',
        ).toBeGreaterThan(80)
        await test.info().attach(`native-swipes-${width}-${theme}`, {
          body: JSON.stringify(
            { gestures, marginBefore, marginAfter },
            null,
            2,
          ),
          contentType: 'application/json',
        })
        expect(
          await page.evaluate(() =>
            window.__WORKERS_HARNESS__.editorSnapshot(),
          ),
        ).toMatchObject({
          value: original,
          focused: false,
          position: { lineNumber: 1, column: 1 },
        })
        const point = await renderedCodePoint(page, 'middle')
        await page.touchscreen.tap(point.x, point.y)
        expect(
          (
            await page.evaluate(() =>
              window.__WORKERS_HARNESS__.editorSnapshot(),
            )
          ).position?.lineNumber,
        ).toBe(point.line)
        await page.keyboard.press('End')
        await page.keyboard.type('0')
        lines[point.line - 1] += '0'
        expect(
          await page.evaluate(() =>
            window.__WORKERS_HARNESS__.editorSnapshot(),
          ),
        ).toMatchObject({
          value: lines.join('\n'),
          focused: true,
          position: {
            lineNumber: point.line,
            column: lines[point.line - 1].length + 1,
          },
        })
        const typed = await page.evaluate(() =>
          window.__WORKERS_HARNESS__.editorSnapshot(),
        )
        for (const direction of ['up', 'down'] as const) {
          const focusedPoint = await renderedCodePoint(
            page,
            direction === 'up' ? 'lower' : 'upper',
          )
          const before = await page
            .locator('.wk-main')
            .evaluate((element) => element.scrollTop)
          await nativeTouchSwipe(
            page,
            focusedPoint,
            focusedPoint.y + (direction === 'up' ? -200 : 200),
          )
          const after = await page
            .locator('.wk-main')
            .evaluate((element) => element.scrollTop)
          expect(
            direction === 'up' ? after - before : before - after,
            'native code swipes must also scroll an already focused editor',
          ).toBeGreaterThan(80)
          expect(
            await page.evaluate(() =>
              window.__WORKERS_HARNESS__.editorSnapshot(),
            ),
          ).toMatchObject({
            value: typed.value,
            position: typed.position,
            selection: typed.selection,
            focused: true,
          })
        }
        const word = await renderedCodePoint(page, 'middle')
        await page.touchscreen.tap(word.x, word.y)
        await page.touchscreen.tap(word.x, word.y)
        expect(
          await page.evaluate(() =>
            window.__WORKERS_HARNESS__.editorSnapshot(),
          ),
        ).toMatchObject({
          value: typed.value,
          selection: {
            startLineNumber: word.line,
            endLineNumber: word.line,
            startColumn: 1,
            endColumn: `setting_${word.line}`.length + 1,
          },
        })
        const holdBefore = await page
          .locator('.wk-main')
          .evaluate((element) => element.scrollTop)
        await nativeTouchHold(page, word)
        expect(
          (
            await page.evaluate(() =>
              window.__WORKERS_HARNESS__.editorSnapshot(),
            )
          ).value,
        ).toBe(typed.value)
        expect(
          await page
            .locator('.wk-main')
            .evaluate((element) => element.scrollTop),
          'held touch must not jump the pane or edit the model',
        ).toBe(holdBefore)
        await page.keyboard.press('ControlOrMeta+S')
        await expect(
          page.getByText('Settings saved for ade', { exact: true }),
        ).toBeVisible()
        const writes = await editCalls(page)
        expect(writes).toHaveLength(1)
        expect(writes[0].payload.config_override).toBe(lines.join('\n'))
        expect(await page.evaluate(() => scrollY)).toBe(0)
        expect((await page.locator('.wk-header').boundingBox())?.y).toBe(0)
      }
    }
    for (const theme of ['dark', 'light']) {
      await page.setViewportSize({ width: 390, height: 900 })
      await page.goto(`${fixture}?monacoCase=fill&theme=${theme}`)
      await expect(page.locator('.wk-code .monaco-editor')).toBeVisible()
      const before = await page.evaluate(() =>
        window.__WORKERS_HARNESS__.editorSnapshot(),
      )
      expect(before.scrollTop).toBe(0)
      const originalPoint = await renderedCodePoint(page, 'middle')
      const up = await renderedCodePoint(page, 'lower')
      await nativeTouchSwipe(page, up, up.y - 200)
      const advanced = await page.evaluate(() =>
        window.__WORKERS_HARNESS__.editorSnapshot(),
      )
      expect(
        advanced.scrollTop,
        'fill=true must retain its native internal editor scrolling',
      ).toBeGreaterThan(80)
      expect((await renderedCodePoint(page, 'middle')).line).toBeGreaterThan(
        originalPoint.line,
      )
      const down = await renderedCodePoint(page, 'upper')
      await nativeTouchSwipe(page, down, down.y + 200)
      const returned = await page.evaluate(() =>
        window.__WORKERS_HARNESS__.editorSnapshot(),
      )
      expect(advanced.scrollTop - returned.scrollTop).toBeGreaterThan(80)
      expect(returned.value).toBe(before.value)
      expect(await page.evaluate(() => scrollY)).toBe(0)
      expect(await editCalls(page)).toEqual([])
    }
  })
  for (const width of [390, 450]) {
    for (const theme of ['dark', 'light']) {
      for (const targetLine of [50, 100]) {
        test(`edits and saves the intended line on the first native touch at ${width}px ${theme} line ${targetLine}`, async ({
          page,
        }) => {
          expect(
            await page.evaluate(() => matchMedia('(pointer: coarse)').matches),
          ).toBe(true)
          await assertColdPointerEdit(
            page,
            { width, height: 900, pane: '' },
            targetLine,
            'touch',
            theme,
          )
        })
      }
    }
  }
})

async function renderedSpecificCodePoint(
  page: Page,
  lineNumber: number,
  horizontal: 'value' | 'end' | 'key' = 'value',
) {
  return page
    .locator('.wk-code .view-line')
    .filter({ hasText: new RegExp(`^setting_${lineNumber}:`) })
    .first()
    .evaluate((element, horizontal) => {
      const range = document.createRange()
      range.selectNodeContents(element)
      const bounds = range.getBoundingClientRect()
      const point = {
        x:
          horizontal === 'end'
            ? bounds.right - 1
            : bounds.left + (horizontal === 'key' ? 15 : 85),
        y: bounds.top + bounds.height / 2,
      }
      if (
        document.elementFromPoint(point.x, point.y)?.closest('.view-line') !==
        element
      )
        throw new Error(
          'The physical gesture must start on its actual painted code line',
        )
      return point
    }, horizontal)
}

async function renderedCodePoint(
  page: Page,
  position: 'upper' | 'middle' | 'lower',
) {
  return page.evaluate((position) => {
    const pane =
      document.querySelector('.wk-main') ??
      document.querySelector('.wk-code .monaco-editor')
    const bar = document.querySelector('.wk-applybar')
    if (!pane)
      throw new Error('The actual worker pane and toolbar must be present')
    const lower =
      Math.max(
        pane.getBoundingClientRect().top,
        bar?.getBoundingClientRect().bottom ?? pane.getBoundingClientRect().top,
      ) + 64
    const upper =
      Math.min(pane.getBoundingClientRect().bottom, innerHeight) - 64
    const desired =
      position === 'upper'
        ? lower + 48
        : position === 'lower'
          ? upper - 48
          : (lower + upper) / 2
    const candidates = [...document.querySelectorAll('.wk-code .view-line')]
      .flatMap((element) => {
        const match = element.textContent?.match(/^setting_(\d+):/)
        if (!match) return []
        const range = document.createRange()
        range.selectNodeContents(element)
        const bounds = range.getBoundingClientRect()
        const y = bounds.top + bounds.height / 2
        if (y < lower || y > upper) return []
        return [{ x: bounds.left + 15, y, line: Number(match[1]) }]
      })
      .sort((a, b) => Math.abs(a.y - desired) - Math.abs(b.y - desired))
    const point = candidates[0]
    if (
      !point ||
      !document.elementFromPoint(point.x, point.y)?.closest('.view-line')
    )
      throw new Error(
        'The native gesture must begin on actual painted code text',
      )
    return point
  }, position)
}

async function nativeTouchHold(page: Page, point: { x: number; y: number }) {
  await page.bringToFront()
  const session = await page.context().newCDPSession(page)
  try {
    await session.send('Input.dispatchTouchEvent', {
      type: 'touchStart',
      touchPoints: [{ x: point.x, y: point.y, id: 1 }],
    })
    await page.waitForTimeout(850)
    await session.send('Input.dispatchTouchEvent', {
      type: 'touchEnd',
      touchPoints: [],
    })
  } finally {
    await session.detach()
  }
}

async function nativeTouchSwipe(
  page: Page,
  start: { x: number; y: number },
  endY: number,
) {
  await page.bringToFront()
  const session = await page.context().newCDPSession(page)
  try {
    await session.send('Input.dispatchTouchEvent', {
      type: 'touchStart',
      touchPoints: [{ x: start.x, y: start.y, id: 1 }],
    })
    for (let step = 1; step <= 10; step += 1) {
      await session.send('Input.dispatchTouchEvent', {
        type: 'touchMove',
        touchPoints: [
          { x: start.x, y: start.y + ((endY - start.y) * step) / 10, id: 1 },
        ],
      })
      await page.evaluate(
        () => new Promise((resolve) => requestAnimationFrame(resolve)),
      )
    }
    await page.waitForTimeout(120)
    await session.send('Input.dispatchTouchEvent', {
      type: 'touchEnd',
      touchPoints: [],
    })
    await page.evaluate(
      () =>
        new Promise((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(resolve)),
        ),
    )
  } finally {
    await session.detach()
  }
}

test('keeps an unsaved worker draft when navigating to another worker and back', async ({
  page,
}) => {
  const command = await openSettings(page)
  await command.fill(editedCommand)
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.dirty))
    .toBeTruthy()
  await selectWorker(page, 'agent')
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.dirty))
    .toBeTruthy()
  await selectWorker(page)
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await expect(command).toHaveValue(editedCommand)
  expect(await editCalls(page)).toEqual([])
})

test('names the native diff context expander and supports Tab with Enter and Space', async ({
  page,
}) => {
  test.setTimeout(90_000)
  for (const scenario of [390, 1656].flatMap((width) =>
    ['dark', 'light'].flatMap((theme) =>
      ['Enter', 'Space'].map((key) => ({ width, theme, key })),
    ),
  )) {
    const { width, theme, key } = scenario
    await page.setViewportSize({ width, height: width === 390 ? 844 : 1314 })
    const command = await openSettings(page, `?theme=${theme}`)
    await expect(page.locator('.wk-code .monaco-editor')).toBeVisible()
    await expect(async () => {
      expect(
        (await page.evaluate(() => window.__WORKERS_HARNESS__.editorSnapshot()))
          .value,
      ).toContain('theme: system')
    }).toPass({ timeout: 5000 })
    const modelBefore = await page.evaluate(() =>
      window.__WORKERS_HARNESS__.editorSnapshot(),
    )
    await command.fill(editedCommand)
    const preview = page.getByRole('button', {
      name: 'Preview changes',
      exact: true,
    })
    await preview.click()
    const review = page.getByRole('region', {
      name: 'Review pending changes',
      exact: true,
    })
    const expander = review
      .getByRole('button', { name: /Expand.*unchanged lines/i })
      .first()
    await expect(expander).toBeVisible()
    await expect(expander).toHaveAccessibleName(/Expand.*unchanged lines/i)
    if (width === 390) {
      const box = await expander.boundingBox()
      expect
        .soft(box?.width, `${theme} native diff expander touch width`)
        .toBeGreaterThanOrEqual(48)
      expect
        .soft(box?.height, `${theme} native diff expander touch height`)
        .toBeGreaterThanOrEqual(48)
    }
    const before = await review.locator('[data-line]').count()
    await review.focus()
    let reached = false
    for (let step = 0; step < 30; step++) {
      await page.keyboard.press('Tab')
      if (await expander.evaluate((element) => element.matches(':focus'))) {
        reached = true
        break
      }
    }
    expect(
      reached,
      'the unchanged-context control must be in ordinary Tab order',
    ).toBe(true)
    await expect(expander).toBeInViewport({ ratio: 0.99 })
    const focus = await expander.evaluate((element) => {
      const box = element.getBoundingClientRect()
      const root = element.getRootNode()
      const hit =
        root instanceof ShadowRoot
          ? root.elementFromPoint(box.left + 1, box.top + 1)
          : document.elementFromPoint(box.left + 1, box.top + 1)
      return {
        visible: element.matches(':focus-visible'),
        outline: Number.parseFloat(getComputedStyle(element).outlineWidth),
        painted: hit === element || (hit !== null && element.contains(hit)),
      }
    })
    expect(
      focus.visible,
      'keyboard expansion needs visible focus semantics',
    ).toBe(true)
    expect(
      focus.outline,
      'the focused expansion action needs a visible outline',
    ).toBeGreaterThanOrEqual(2)
    expect(
      focus.painted,
      'an equivalent decorative affordance must not cover the keyboard focus target',
    ).toBe(true)
    const paneBefore = await page
      .locator('.wk-main')
      .evaluate((element) => element.scrollTop)
    await page.keyboard.press(key)
    const paneAfterActivation = await page
      .locator('.wk-main')
      .evaluate((element) => element.scrollTop)
    await expect
      .poll(() => review.locator('[data-line]').count())
      .toBeGreaterThan(before)
    await expect(
      review.locator('[data-line]').filter({ hasText: 'theme: system' }),
    ).toBeVisible()
    const expandedFocus = await settingsFocusPaint(page)
    const focusScreenshot = test
      .info()
      .outputPath(`diff-expanded-focus-${width}-${theme}-${key}.png`)
    await page.screenshot({ path: focusScreenshot })
    await test.info().attach(`diff-expanded-focus-${width}-${theme}-${key}`, {
      path: focusScreenshot,
      contentType: 'image/png',
    })
    const nextFocus = await (async () => {
      await page.keyboard.press('Tab')
      return settingsFocusPaint(page)
    })()
    await page.keyboard.press('Shift+Tab')
    const previousFocus = await settingsFocusPaint(page)
    const id = `diff-focus-recovery-${width}-${theme}-${key}`
    await test.info().attach(id, {
      body: JSON.stringify(
        {
          paneBefore,
          paneAfterActivation,
          expandedFocus,
          nextFocus,
          previousFocus,
        },
        null,
        2,
      ),
      contentType: 'application/json',
    })
    for (const [stage, paint] of [
      ['expanded', expandedFocus],
      ['Tab', nextFocus],
      ['Shift+Tab', previousFocus],
    ] as const) {
      expect
        .soft(paint.connected, `${id}/${stage}: focus stays connected`)
        .toBe(true)
      expect
        .soft(paint.tag, `${id}/${stage}: focus must not fall to BODY`)
        .not.toBe('BODY')
      expect
        .soft(
          paint.inSettings,
          `${id}/${stage}: native traversal stays in Settings`,
        )
        .toBe(true)
      expect
        .soft(paint.focusVisible, `${id}/${stage}: native focus is indicated`)
        .toBe(true)
      expect
        .soft(paint.painted, `${id}/${stage}: focus owns its painted center`)
        .toBe(true)
      expect
        .soft(paint.indicator, `${id}/${stage}: focus has a painted indicator`)
        .toBe(true)
      if (!paint.inApplyBar)
        expect
          .soft(paint.top, `${id}/${stage}: focus clears the sticky bar`)
          .toBeGreaterThanOrEqual(paint.barBottom + 2)
      expect(paint.outerScroll).toBe(0)
      expect(paint.headerY).toBe(0)
    }
    expect
      .soft(
        expandedFocus.inReview,
        `${id}: removed expander keeps focus in its review`,
      )
      .toBe(true)
    await expect(command).toHaveValue(editedCommand)
    expect(
      await page.evaluate(() => window.__WORKERS_HARNESS__.dirty),
    ).toBeTruthy()
    expect(
      (await page.evaluate(() => window.__WORKERS_HARNESS__.editorSnapshot()))
        .value,
    ).toBe(modelBefore.value)
    expect(await editCalls(page)).toEqual([])
    expect(
      await page.evaluate(() =>
        window.__WORKERS_HARNESS__.calls.filter((call) =>
          ['compose::restart', 'compose::up', 'compose::down'].includes(
            call.functionId,
          ),
        ),
      ),
    ).toEqual([])
  }
})

test('uses one keyboard stop per native diff context action while retaining both pointer affordances', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 })
  for (const action of [
    'Enter',
    'Space',
    'pointer-gutter',
    'pointer-content',
  ]) {
    const command = await openSettings(page)
    await command.fill(editedCommand)
    await page
      .getByRole('button', { name: 'Preview changes', exact: true })
      .click()
    const review = page.getByRole('region', {
      name: 'Review pending changes',
      exact: true,
    })
    const controls = review.locator('[data-expand-button]')
    await expect(controls).toHaveCount(2)
    for (const control of await controls.all()) {
      const box = await control.boundingBox()
      expect(
        box?.width,
        'both physical phone affordances retain touch width',
      ).toBeGreaterThanOrEqual(48)
      expect(
        box?.height,
        'both physical phone affordances retain touch height',
      ).toBeGreaterThanOrEqual(48)
    }
    const groups = await controls.evaluateAll((elements) => {
      const grouped = new Map<string, { controls: number; tabbable: number }>()
      for (const element of elements) {
        const hunk = element
          .closest('[data-expand-index]')
          ?.getAttribute('data-expand-index')
        const direction = element.hasAttribute('data-expand-up')
          ? 'up'
          : element.hasAttribute('data-expand-down')
            ? 'down'
            : 'both'
        const key = `${hunk}:${direction}:${element.hasAttribute('data-expand-all-button')}`
        const group = grouped.get(key) ?? { controls: 0, tabbable: 0 }
        group.controls++
        if (
          element instanceof HTMLButtonElement &&
          element.tabIndex >= 0 &&
          !element.disabled
        )
          group.tabbable++
        grouped.set(key, group)
      }
      return [...grouped.entries()]
    })
    expect(groups).toHaveLength(1)
    expect
      .soft(
        groups[0][1],
        'same hunk/direction/action must retain both pointer controls with one ordinary keyboard stop',
      )
      .toEqual({ controls: 2, tabbable: 1 })
    const before = await review.locator('[data-line]').count()
    if (action.startsWith('pointer-')) {
      const box = await controls
        .nth(action === 'pointer-gutter' ? 0 : 1)
        .boundingBox()
      if (!box)
        throw new Error('The physical expansion affordance must be visible')
      await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2)
    } else {
      await review.focus()
      await page.keyboard.press('Tab')
      await expect(
        review.locator('button[data-expand-button]').first(),
      ).toBeFocused()
      await page.keyboard.press(action)
    }
    await expect
      .poll(() => review.locator('[data-line]').count())
      .toBeGreaterThan(before)
    expect(await editCalls(page)).toEqual([])
  }
  await page.goto(`${fixture}?diffCase=multiple`)
  const distinct = page.getByRole('region', {
    name: 'Native diff regression',
    exact: true,
  })
  await expect(distinct.locator('[data-expand-button]').first()).toBeVisible()
  const groups = await distinct
    .locator('[data-expand-button]')
    .evaluateAll((elements) => {
      const grouped = new Map<
        string,
        {
          hunk: string | null | undefined
          direction: string
          all: boolean
          controls: number
          tabbable: number
        }
      >()
      for (const element of elements) {
        const hunk = element
          .closest('[data-expand-index]')
          ?.getAttribute('data-expand-index')
        const direction = element.hasAttribute('data-expand-up')
          ? 'up'
          : element.hasAttribute('data-expand-down')
            ? 'down'
            : 'both'
        const all = element.hasAttribute('data-expand-all-button')
        const key = `${hunk}:${direction}:${all}`
        const group = grouped.get(key) ?? {
          hunk,
          direction,
          all,
          controls: 0,
          tabbable: 0,
        }
        group.controls++
        if (
          element instanceof HTMLButtonElement &&
          element.tabIndex >= 0 &&
          !element.disabled
        )
          group.tabbable++
        grouped.set(key, group)
      }
      return [...grouped.values()]
    })
  expect(
    new Set(groups.map((group) => group.hunk)).size,
    'different hunks must retain independent actions',
  ).toBeGreaterThanOrEqual(2)
  expect(groups.some((group) => group.direction === 'up' && !group.all)).toBe(
    true,
  )
  expect(groups.some((group) => group.direction === 'down' && !group.all)).toBe(
    true,
  )
  expect(
    groups.some((group) => group.all),
    'expand-all must remain separate from directional expansion',
  ).toBe(true)
  for (const group of groups) {
    expect(
      group.controls,
      'both physical affordances must remain rendered',
    ).toBe(2)
    expect(
      group.tabbable,
      `${group.hunk}/${group.direction}/${group.all} retains one keyboard action`,
    ).toBe(1)
  }
  expect(await editCalls(page)).toEqual([])
})

test('keeps phone settings actions and searches at readable touch sizes without shrinking', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await page.goto(fixture)
  const workerSearch = page.getByRole('searchbox', {
    name: 'Filter workers',
    exact: true,
  })
  await expect(workerSearch).toBeVisible()
  const searchMetrics = await workerSearch.evaluate((element) => ({
    height: element.getBoundingClientRect().height,
    fontSize: Number.parseFloat(getComputedStyle(element).fontSize),
  }))
  expect
    .soft(searchMetrics.height, 'phone worker search height')
    .toBeGreaterThanOrEqual(48)
  expect
    .soft(searchMetrics.fontSize, 'phone worker search text size')
    .toBeGreaterThanOrEqual(16)
  await selectWorker(page)
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await expect(
    page.getByRole('textbox', { name: 'Command', exact: true }),
  ).toHaveValue('./target/debug/ade')
  for (const width of [390, 450]) {
    await page.setViewportSize({ width, height: 900 })
    for (const name of ['Workers', 'Restart', 'Stop', 'Add worker']) {
      const action = page.getByRole('button', { name, exact: true })
      const box = await action.boundingBox()
      expect
        .soft(box?.width, `${name} ${width}px target width`)
        .toBeGreaterThanOrEqual(48)
      expect
        .soft(box?.height, `${name} ${width}px target height`)
        .toBeGreaterThanOrEqual(48)
    }
    const header = page.getByRole('banner')
    const headerBox = await header.boundingBox()
    const title = await header
      .getByRole('heading', { name: 'Workers', exact: true })
      .boundingBox()
    const namespace = await header
      .getByText('iii-local', { exact: true })
      .boundingBox()
    const add = await header
      .getByRole('button', { name: 'Add worker', exact: true })
      .boundingBox()
    if (!headerBox || !title || !namespace || !add)
      throw new Error('Header labels and Add worker must remain visible')
    for (const box of [title, namespace, add]) {
      expect(box.x).toBeGreaterThanOrEqual(headerBox.x)
      expect(box.x + box.width).toBeLessThanOrEqual(
        headerBox.x + headerBox.width,
      )
      expect(box.y).toBeGreaterThanOrEqual(headerBox.y)
      expect(box.y + box.height).toBeLessThanOrEqual(
        headerBox.y + headerBox.height,
      )
    }
    expect(title.x + title.width).toBeLessThanOrEqual(namespace.x)
    expect(namespace.x + namespace.width).toBeLessThanOrEqual(add.x)
  }
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: width === 390 ? 844 : 1314 })
    for (const name of [
      'Copy run command',
      'Stop waiting for state',
      'Remove III_PORT',
      'Remove LOG_LEVEL',
      'Remove API_TOKEN',
    ]) {
      const action = page.getByRole('button', { name, exact: true })
      await action.scrollIntoViewIfNeeded()
      const metrics = await action.evaluate((element) => ({
        width: element.getBoundingClientRect().width,
        height: element.getBoundingClientRect().height,
        shrink: getComputedStyle(element).flexShrink,
      }))
      expect
        .soft(metrics.width, `${name} ${width}px icon width`)
        .toBeGreaterThanOrEqual(width === 390 ? 48 : 30)
      expect
        .soft(metrics.height, `${name} ${width}px icon height`)
        .toBeGreaterThanOrEqual(width === 390 ? 48 : 30)
      expect
        .soft(metrics.shrink, `${name} must retain its allocated icon width`)
        .toBe('0')
    }
  }
  const build = page.getByRole('button', { name: /^Use build command/ })
  await build.scrollIntoViewIfNeeded()
  expect
    .soft((await build.boundingBox())?.height, 'phone build-on-start height')
    .toBeGreaterThanOrEqual(48)
  await page
    .getByRole('button', { name: 'Add dependency', exact: true })
    .click()
  const search = page.getByRole('combobox', {
    name: 'Search dependencies',
    exact: true,
  })
  await textPresentation(search)
  const metrics = await search.evaluate((element) => ({
    height: element.getBoundingClientRect().height,
    fontSize: Number.parseFloat(getComputedStyle(element).fontSize),
  }))
  expect
    .soft(metrics.height, 'phone dependency search height')
    .toBeGreaterThanOrEqual(48)
  expect
    .soft(metrics.fontSize, 'phone dependency search text size')
    .toBeGreaterThanOrEqual(16)
  await search.fill('agent')
  await page.getByRole('option', { name: /^agent ready$/ }).click()
  await expect(
    page.getByRole('button', { name: 'Stop waiting for agent', exact: true }),
  ).toBeVisible()
  expect(await editCalls(page)).toEqual([])
  await page.setViewportSize({ width: 1656, height: 1314 })
  for (const name of ['Restart', 'Stop', 'Add worker']) {
    const box = await page
      .getByRole('button', { name, exact: true })
      .boundingBox()
    expect(box?.height, `${name} desktop chrome retains compact sizing`).toBe(
      32,
    )
  }
})

test('keeps simple dependency name state and removal compact in a 720px hosting pane', async ({
  page,
}) => {
  await openSettings(page, '?pane=720')
  const section = page.getByRole('region', {
    name: 'Starts after',
    exact: true,
  })
  for (const name of ['state', 'storage']) {
    const action = section.getByRole('button', {
      name: `Stop waiting for ${name}`,
      exact: true,
    })
    const row = action.locator('xpath=ancestor::*[@role="listitem"][1]')
    const metrics = await row.evaluate((element) => {
      const button = element.querySelector('button')
      const label = element.querySelector('.wk-mono')
      if (!button || !label)
        throw new Error('Dependency label and removal are required')
      const box = element.getBoundingClientRect()
      const action = button.getBoundingClientRect()
      const text = label.getBoundingClientRect()
      return {
        height: box.height,
        actionTop: action.top,
        actionBottom: action.bottom,
        textTop: text.top,
        textBottom: text.bottom,
      }
    })
    expect
      .soft(metrics.height, `${name} simple dependency row height`)
      .toBeLessThanOrEqual(80)
    expect
      .soft(
        metrics.actionTop,
        `${name} removal must remain alongside the label/state`,
      )
      .toBeLessThan(metrics.textBottom)
    expect.soft(metrics.actionBottom).toBeGreaterThan(metrics.textTop)
  }
  expect(await editCalls(page)).toEqual([])
})

test('does not report a persisted configuration as a failed save in global activity', async ({
  page,
}) => {
  const command = await openSettings(page)
  await command.fill(editedCommand)
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.saveMode = 'restart-error'
  })
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await expect(
    page.getByText('Configuration saved; restart failed', { exact: true }),
  ).toBeVisible()
  await expect(
    page.getByText('Saving ade failed', { exact: true }),
  ).toHaveCount(0)
  const activity = page.locator('.wk-activity-complete')
  await expect(activity).toContainText(
    'ade: Configuration saved; restart failed',
  )
  await expect(activity).not.toContainText(/Saving|Restarting/)
  expect((await editCalls(page))[0]?.payload.run).toBe(editedCommand)
  await expect(
    page.getByRole('button', { name: 'Save and restart', exact: true }),
  ).toBeDisabled()
  await selectWorker(page, 'agent')
  await expect(activity).toContainText(
    'ade: Configuration saved; restart failed',
  )
  await expect(activity).not.toContainText(/Saving|Restarting/)
  await selectWorker(page, 'ade')
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await expect(command).toHaveValue(editedCommand)
  await page.getByRole('button', { name: 'Retry restart', exact: true }).click()
  expect(await editCalls(page)).toHaveLength(1)
})

test('preserves separate drafts for two workers and through the project overview', async ({
  page,
}) => {
  const command = await openSettings(page)
  await command.fill(editedCommand)
  await selectWorker(page, 'agent')
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await command.fill('cargo run --locked --bin agent')
  await page
    .getByRole('button', { name: /^Project/ })
    .first()
    .click()
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.dirty))
    .toBeTruthy()
  await selectWorker(page)
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await expect(command).toHaveValue(editedCommand)
  await selectWorker(page, 'agent')
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await expect(command).toHaveValue('cargo run --locked --bin agent')
})

test('reports an unnamed variable and prevents saving until its name is supplied', async ({
  page,
}) => {
  await openSettings(page)
  await page.getByRole('button', { name: 'Add variable' }).click()
  await page
    .getByRole('textbox', { name: 'New variable value', exact: true })
    .fill('hello')
  const save = page.getByRole('button', {
    name: 'Save and restart',
    exact: true,
  })
  await expect(save).toBeDisabled()
  await expect(
    page.getByRole('textbox', { name: 'Variable name', exact: true }),
  ).toHaveAttribute('aria-invalid', 'true')
  await page
    .getByRole('textbox', { name: 'Variable name', exact: true })
    .fill('GREETING')
  await expect(save).toBeEnabled()
  expect(await editCalls(page)).toEqual([])
})

test('rejects duplicate and invalid variable names without silently rewriting them', async ({
  page,
}) => {
  await openSettings(page)
  await page.getByRole('button', { name: 'Add variable' }).click()
  const name = page.getByRole('textbox', { name: 'Variable name', exact: true })
  const save = page.getByRole('button', {
    name: 'Save and restart',
    exact: true,
  })
  await name.fill('III_PORT')
  await expect(save).toBeDisabled()
  await expect(name).toHaveAttribute('aria-invalid', 'true')
  await name.fill('API-KEY')
  await expect(name).toHaveValue('API-KEY')
  await expect(save).toBeDisabled()
  await name.fill('API_KEY')
  await expect(save).toBeEnabled()
  expect(await editCalls(page)).toEqual([])
})

test('blocks malformed YAML and accepts a corrected mapping', async ({
  page,
}) => {
  await openSettings(page)
  await replaceYaml(page, 'ui: [unterminated')
  const save = page.getByRole('button', {
    name: 'Save and restart',
    exact: true,
  })
  await expect(save).toBeDisabled()
  await replaceYaml(page, 'ui:\n  retries: 3')
  await expect(page.locator('.wk-code .view-lines')).toContainText('retries: 3')
  await expect(save).toBeEnabled()
  expect(await editCalls(page)).toEqual([])
})

test('saves only the edited worker patch and clears its draft after reading the saved entry', async ({
  page,
}) => {
  const command = await openSettings(page)
  await command.fill(editedCommand)
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await expect(
    page.getByText('Settings saved for ade', { exact: true }).first(),
  ).toBeVisible()
  await expect(command).toHaveValue(editedCommand)
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.dirty))
    .toBe(false)
  await expect(
    page.getByRole('button', { name: 'Save and restart', exact: true }),
  ).toBeDisabled()
  const calls = await editCalls(page)
  expect(calls).toHaveLength(1)
  expect(calls[0].payload).toMatchObject({
    container: 'ade',
    run: editedCommand,
  })
  expect(calls[0].payload).not.toHaveProperty('environment')
  await selectWorker(page, 'agent')
  await selectWorker(page)
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await expect(command).toHaveValue(editedCommand)
})

async function assertCredentialReviewReading(
  page: Page,
  review: Locator,
  viewport: { width: number; height: number; pane?: number },
  readingId: string,
) {
  const credentialWarning = review.getByText(
    /Saved secrets stay as they are now.*previous values cannot be recovered/,
  )
  await expect(credentialWarning).toBeVisible({ timeout: 1500 })
  await expect(credentialWarning).toBeInViewport({ ratio: 1 })
  const warningBox = await credentialWarning.boundingBox()
  const diffBox = await review.locator('.wk-diff').boundingBox()
  if (!warningBox || !diffBox)
    throw new Error(
      'Credential recovery guidance and diff must both be visible',
    )
  expect(warningBox.y + warningBox.height).toBeLessThanOrEqual(diffBox.y)
  const warningLines = await paintedTextLines(credentialWarning)
  const warningMeasure = await credentialWarning.evaluate((element) => {
    const style = getComputedStyle(element)
    const canvas = document.createElement('canvas')
    const context = canvas.getContext('2d')
    if (!context) throw new Error('Text measurement needs a canvas context')
    context.font = `${style.fontWeight} ${style.fontSize} ${style.fontFamily}`
    return {
      text: element.textContent,
      widthInCharacters:
        element.getBoundingClientRect().width / context.measureText('0').width,
    }
  })
  const reentry = review
    .locator('p')
    .filter({ hasText: /^Re-enter previous secrets if needed:/ })
  const reentryCount = await reentry.count()
  await test.info().attach(readingId, {
    body: JSON.stringify(
      {
        viewport,
        warningBox,
        warningLines,
        warningMeasure,
        separateReentryParagraphs: reentryCount,
        diffBox,
      },
      null,
      2,
    ),
    contentType: 'application/json',
  })
  const readingScreenshot = test.info().outputPath(`${readingId}.png`)
  await page.screenshot({ path: readingScreenshot })
  await test.info().attach(readingId, {
    path: readingScreenshot,
    contentType: 'image/png',
  })
  expect
    .soft(
      Math.max(...warningLines.map((line) => line.length)),
      `${readingId} painted credential warning: ${warningLines.join(' | ')}`,
    )
    .toBeLessThanOrEqual(80)
  expect.soft(warningMeasure.widthInCharacters).toBeLessThanOrEqual(75)
  expect
    .soft(reentryCount, 'Re-entry instructions need their own paragraph')
    .toBe(1)
  if (reentryCount === 1) {
    await expect(reentry).toHaveText(
      'Re-enter previous secrets if needed: API_TOKEN.',
    )
    await expect(reentry).toBeInViewport({ ratio: 1 })
    const reentryBox = await reentry.boundingBox()
    if (!reentryBox)
      throw new Error(
        'The separate credential re-entry instruction must render',
      )
    expect(warningBox.y + warningBox.height).toBeLessThanOrEqual(reentryBox.y)
    expect(reentryBox.y + reentryBox.height).toBeLessThanOrEqual(diffBox.y)
  }
  const reviewBox = await review.boundingBox()
  const applyBox = await page
    .getByRole('region', { name: 'Apply worker settings', exact: true })
    .boundingBox()
  const outline = await review.evaluate((element) => {
    const style = getComputedStyle(element)
    return {
      width: Number.parseFloat(style.outlineWidth),
      offset: Number.parseFloat(style.outlineOffset),
    }
  })
  if (!reviewBox || !applyBox)
    throw new Error('Review and its sticky toolbar must be visible')
  expect(outline.width).toBeGreaterThan(0)
  expect(
    reviewBox.y - outline.width - outline.offset,
    `${viewport.width}px${'pane' in viewport ? '/720 pane' : ''} review outline must clear the sticky toolbar`,
  ).toBeGreaterThanOrEqual(applyBox.y + applyBox.height)
}

test('restores previous settings into a reviewable draft that can be discarded or applied', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 })
  const command = await openSettings(page)
  await command.fill(editedCommand)
  await page
    .getByRole('button', { name: 'Stop waiting for state', exact: true })
    .click()
  await page
    .getByRole('button', { name: 'Add dependency', exact: true })
    .click()
  await page
    .getByRole('combobox', { name: 'Search dependencies', exact: true })
    .fill('agent')
  await page.getByRole('option', { name: /^agent\b/ }).click()
  await page.getByLabel('LOG_LEVEL value', { exact: true }).fill('debug')
  await page
    .getByRole('button', { name: 'Remove III_PORT', exact: true })
    .click()
  await page.getByRole('button', { name: 'Add variable', exact: true }).click()
  await page
    .getByRole('textbox', { name: 'Variable name', exact: true })
    .fill('FEATURE_ENABLED')
  await page.getByLabel('FEATURE_ENABLED value', { exact: true }).fill('yes')
  const yaml = page.getByRole('textbox', {
    name: 'Configuration override YAML',
    exact: true,
  })
  await yaml.focus()
  await yaml.press('ControlOrMeta+A')
  await page.keyboard.type('ui: { theme: light, compact: true }')
  await expect(page.locator('.wk-code .view-lines')).toContainText(
    'ui: { theme: light, compact: true }',
  )
  const save = page.getByRole('button', {
    name: 'Save and restart',
    exact: true,
  })
  await save.click()
  await expect(
    page.getByText('Settings saved for ade', { exact: true }).first(),
  ).toBeVisible()
  await expect(
    page.getByText('Previous saved settings', { exact: true }),
  ).toBeVisible()
  await openPreviousSettings(page)
  const restore = page.getByRole('button', {
    name: 'Restore previous settings',
    exact: true,
  })
  await expect(restore).toBeVisible({ timeout: 1500 })
  const review = page.getByRole('region', { name: 'Review pending changes' })
  for (const viewport of [
    { width: 1000, height: 900 },
    { width: 1656, height: 1314, pane: 720 },
    { width: 1656, height: 1314 },
    { width: 390, height: 844 },
  ]) {
    await page.setViewportSize(viewport)
    await page.evaluate(
      (pane) => {
        if (pane)
          document.documentElement.style.setProperty(
            '--harness-pane-width',
            `${pane}px`,
          )
        else
          document.documentElement.style.removeProperty('--harness-pane-width')
      },
      'pane' in viewport ? viewport.pane : null,
    )
    await openPreviousSettings(page)
    await restore.focus()
    await restore.press('Enter')
    await expect(review).toBeFocused()
    await expect(review).toBeInViewport()
    await assertCredentialReviewReading(
      page,
      review,
      viewport,
      `restore-reading-${viewport.width}${'pane' in viewport ? '-720-pane' : ''}-dark`,
    )
    if (viewport.width !== 390)
      await page.getByRole('button', { name: 'Discard', exact: true }).click()
  }
  await expect(command).toHaveValue('./target/debug/ade')
  await expect(
    page.getByRole('button', { name: 'Stop waiting for state', exact: true }),
  ).toBeVisible()
  await expect(
    page.getByRole('button', { name: 'Stop waiting for agent', exact: true }),
  ).toHaveCount(0)
  await expect(page.getByLabel('LOG_LEVEL value', { exact: true })).toHaveValue(
    'info',
  )
  await expect(page.getByLabel('III_PORT value', { exact: true })).toHaveValue(
    '3113',
  )
  await expect(
    page.getByLabel('FEATURE_ENABLED value', { exact: true }),
  ).toBeDisabled()
  await expect(
    page.getByRole('button', { name: 'Keep FEATURE_ENABLED', exact: true }),
  ).toBeVisible()
  await expect(page.locator('.wk-code .view-lines')).toContainText(
    'theme: system',
  )
  await expect(page.locator('.wk-code .view-lines')).toContainText(
    'compact: false',
  )
  expect(await editCalls(page)).toHaveLength(1)
  await expect(review).toBeVisible()
  await expect(review).toContainText(editedCommand)
  await expect(review).toContainText('./target/debug/ade')
  await page.getByRole('button', { name: 'Discard', exact: true }).click()
  await expect(command).toHaveValue(editedCommand)
  await expect(page.getByLabel('LOG_LEVEL value', { exact: true })).toHaveValue(
    'debug',
  )
  await expect(
    page.getByLabel('FEATURE_ENABLED value', { exact: true }),
  ).toHaveValue('yes')
  await expect(save).toBeDisabled()
  await restore.click()
  await expect(command).toHaveValue('./target/debug/ade')
  await save.click()
  await expect(
    page.getByText('Settings saved for ade', { exact: true }).first(),
  ).toBeVisible()
  await expect(save).toBeDisabled()
  await expect(command).toHaveValue('./target/debug/ade')
  await expect(
    page.getByLabel('FEATURE_ENABLED value', { exact: true }),
  ).toHaveCount(0)
  const writes = await editCalls(page)
  expect(writes).toHaveLength(2)
  expect(writes[1].payload).toMatchObject({
    container: 'ade',
    run: './target/debug/ade',
    start_after: ['state', 'storage'],
    config_override: 'ui:\n  theme: system\n  compact: false',
    environment: {
      set: { III_PORT: '3113', LOG_LEVEL: 'info' },
      unset: ['FEATURE_ENABLED'],
    },
  })
  for (const viewport of [
    { width: 1656, height: 1314 },
    { width: 390, height: 844 },
  ]) {
    await page.setViewportSize(viewport)
    const lightCommand = await openSettings(page, '?theme=light')
    await lightCommand.fill(editedCommand)
    await page
      .getByRole('button', { name: 'Save and restart', exact: true })
      .click()
    await expect(
      page.getByText('Settings saved for ade', { exact: true }).first(),
    ).toBeVisible()
    const lightRestore = page.getByRole('button', {
      name: 'Restore previous settings',
      exact: true,
    })
    await openPreviousSettings(page)
    await lightRestore.focus()
    await lightRestore.press('Enter')
    await expect(review).toBeFocused()
    await expect(review).toBeInViewport()
    await assertCredentialReviewReading(
      page,
      review,
      viewport,
      `restore-reading-${viewport.width}-light`,
    )
    await expect(lightCommand).toHaveValue('./target/debug/ade')
    await expect(
      page.getByLabel('API_TOKEN value', { exact: true }),
    ).toHaveCount(0)
    expect(await editCalls(page)).toHaveLength(1)
  }
})

test('retains each worker previous baseline and confirms before replacing a dirty draft', async ({
  page,
}) => {
  const command = await openSettings(page)
  await command.fill(editedCommand)
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await expect(
    page.getByText('Settings saved for ade', { exact: true }).first(),
  ).toBeVisible()
  await selectWorker(page, 'agent')
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await command.fill('agent local draft')
  await selectWorker(page)
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await expect(command).toHaveValue(editedCommand)
  await command.fill('ade later unsaved draft')
  const restore = page.getByRole('button', {
    name: 'Restore previous settings',
    exact: true,
  })
  await restore.click()
  let confirmation = page.getByRole('alertdialog')
  await expect(confirmation).toBeVisible()
  await confirmation
    .getByRole('button', { name: 'Cancel', exact: true })
    .click()
  await expect(command).toHaveValue('ade later unsaved draft')
  await restore.click()
  confirmation = page.getByRole('alertdialog')
  await confirmation
    .getByRole('button', { name: 'Restore draft', exact: true })
    .click()
  await expect(command).toHaveValue('./target/debug/ade')
  expect(await editCalls(page)).toHaveLength(1)
  await selectWorker(page, 'agent')
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await expect(command).toHaveValue('agent local draft')
  await expect(restore).toHaveCount(0)
  await selectWorker(page)
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await expect(command).toHaveValue('./target/debug/ade')
})

test('restores readable settings without replaying a replaced write-only secret or its mask', async ({
  page,
}) => {
  const command = await openSettings(page)
  const replacement = 'sk-restore-fixture-secret-never-replay'
  await command.fill(editedCommand)
  await page.getByRole('button', { name: 'Replace', exact: true }).click()
  await page.getByLabel('API_TOKEN value', { exact: true }).fill(replacement)
  const save = page.getByRole('button', {
    name: 'Save and restart',
    exact: true,
  })
  await save.click()
  await expect(
    page.getByText('Settings saved for ade', { exact: true }).first(),
  ).toBeVisible()
  await expect(page.getByLabel('API_TOKEN value', { exact: true })).toHaveCount(
    0,
  )
  await page
    .getByRole('button', { name: 'Restore previous settings', exact: true })
    .click()
  await expect(command).toHaveValue('./target/debug/ade')
  await expect(page.getByLabel('API_TOKEN value', { exact: true })).toHaveCount(
    0,
  )
  const review = page.getByRole('region', { name: 'Review pending changes' })
  await expect(review).toBeVisible()
  await expect(review).not.toContainText(replacement)
  expect(await editCalls(page)).toHaveLength(1)
  await save.click()
  await expect(
    page.getByText('Settings saved for ade', { exact: true }).first(),
  ).toBeVisible()
  await expect(save).toBeDisabled()
  const writes = await editCalls(page)
  expect(writes).toHaveLength(2)
  expect(writes[0].payload).toMatchObject({
    environment: { set: { API_TOKEN: replacement } },
  })
  expect(writes[1].payload).not.toHaveProperty('environment')
  await expect(page.getByLabel('API_TOKEN value', { exact: true })).toHaveCount(
    0,
  )
})

test('blocks saving restored dependencies that became cyclic after another client edited the graph', async ({
  page,
}) => {
  const command = await openSettings(page, '?commands=1')
  await command.fill(editedCommand)
  for (const worker of ['state', 'storage']) {
    await page
      .getByRole('button', { name: `Stop waiting for ${worker}`, exact: true })
      .click()
  }
  const save = page.getByRole('button', {
    name: 'Save and restart',
    exact: true,
  })
  await save.click()
  await expect(
    page.getByText('Settings saved for ade', { exact: true }).first(),
  ).toBeVisible()
  const readsBefore = await page.evaluate(
    () => window.__WORKERS_HARNESS__.readsCompleted,
  )
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.externalEdit('state', { start_after: ['ade'] })
    window.__WORKERS_HARNESS__.runCommand('refresh')
  })
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.readsCompleted))
    .toBeGreaterThan(readsBefore)
  await page
    .getByRole('button', { name: 'Restore previous settings', exact: true })
    .click()
  await expect(command).toHaveValue('./target/debug/ade')
  await expect(page.getByText(/ade → state → ade/).first()).toBeVisible()
  await expect(save).toBeDisabled()
  await command.press('ControlOrMeta+s')
  await expect(page.getByRole('alertdialog')).toHaveCount(0)
  expect(await editCalls(page)).toHaveLength(1)
})

test('offers guarded expert review and restore commands that focus a draft preview without writing it', async ({
  page,
}) => {
  const command = await openSettings(page, '?commands=1')
  const registered = (id: string) =>
    page.evaluate(
      (id) =>
        window.__WORKERS_HARNESS__
          .commands()
          .find((command) => command.id === id),
      id,
    )
  for (const id of ['review-settings', 'restore-settings']) {
    await expect
      .poll(() => registered(id), { timeout: 1500 })
      .toMatchObject({ id, enabled: false })
  }
  await command.fill(editedCommand)
  await expect
    .poll(() => registered('review-settings'))
    .toMatchObject({ enabled: true })
  await page.evaluate(() =>
    window.__WORKERS_HARNESS__.runCommand('review-settings'),
  )
  const review = page.getByRole('region', { name: 'Review pending changes' })
  await expect(review).toBeFocused()
  await expect(review).toBeInViewport()
  expect(await editCalls(page)).toEqual([])
  await command.focus()
  await page.evaluate(() =>
    window.__WORKERS_HARNESS__.runCommand('review-settings'),
  )
  await expect(review).toBeFocused()
  await expect(review).toBeInViewport()
  await page.getByRole('button', { name: 'Add variable', exact: true }).click()
  await page
    .getByRole('textbox', { name: 'Variable name', exact: true })
    .fill('API-KEY')
  await expect
    .poll(() => registered('review-settings'))
    .toMatchObject({ enabled: false })
  await page
    .getByRole('button', { name: 'Remove API-KEY', exact: true })
    .click()
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.saveDelay = 500
  })
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await expect
    .poll(() => registered('review-settings'))
    .toMatchObject({ enabled: false })
  await expect(
    page.getByText('Settings saved for ade', { exact: true }).first(),
  ).toBeVisible()
  await expect
    .poll(() => registered('restore-settings'))
    .toMatchObject({ enabled: true })
  await page.evaluate(() =>
    window.__WORKERS_HARNESS__.runCommand('restore-settings'),
  )
  await expect(command).toHaveValue('./target/debug/ade')
  await expect(review).toBeFocused()
  await expect(review).toBeInViewport()
  expect(await editCalls(page)).toHaveLength(1)
  await page.evaluate(() =>
    window.__WORKERS_HARNESS__.runCommand('restore-settings'),
  )
  const confirmation = page.getByRole('alertdialog')
  await expect(confirmation).toBeVisible()
  await confirmation
    .getByRole('button', { name: 'Cancel', exact: true })
    .click()
  await expect(command).toHaveValue('./target/debug/ade')
  expect(await editCalls(page)).toHaveLength(1)
})

test('keeps the editable draft and clear failure detail when a save fails', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1000, height: 900 })
  const command = await openSettings(page)
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.saveMode = 'error'
  })
  await command.fill(editedCommand)
  await scrollSettingsBottom(page)
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  const result = page.getByRole('region', { name: 'Worker settings result' })
  const cause = result.getByText(
    /Mock compose cannot write worker-compose\.yaml/,
  )
  await expect(result).toBeFocused()
  await expect(cause).toBeVisible()
  await expect(cause).toBeInViewport({ ratio: 0.99 })
  await expect(command).toHaveValue(editedCommand)
  await expect(
    page.getByRole('button', { name: 'Save and restart', exact: true }),
  ).toBeEnabled()
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.dirty))
    .toBeTruthy()
  expect(await editCalls(page)).toHaveLength(1)
})

test('blocks direct and transitive startup cycles in the dependency picker for pointer and keyboard users', async ({
  page,
}) => {
  for (const scenario of [
    { name: 'dark desktop', theme: 'dark', width: 1000, height: 900, pane: '' },
    {
      name: 'light desktop',
      theme: 'light',
      width: 1000,
      height: 900,
      pane: '',
    },
    { name: 'dark phone', theme: 'dark', width: 390, height: 844, pane: '' },
    {
      name: 'dark 720px pane',
      theme: 'dark',
      width: 1656,
      height: 1314,
      pane: '&pane=720',
    },
  ]) {
    const theme = scenario.name
    await page.setViewportSize({
      width: scenario.width,
      height: scenario.height,
    })
    await openSettings(
      page,
      `?cycleWorkers=1&commands=1&theme=${scenario.theme}${scenario.pane}`,
    )
    const add = page.getByRole('button', {
      name: 'Add dependency',
      exact: true,
    })
    await add.click()
    const search = page.getByRole('combobox', {
      name: 'Search dependencies',
      exact: true,
    })
    const popup = page.locator('.iii-ui-panel.wk-dependency-picker')
    const popupPresentation = await textPresentation(popup)
    expect
      .soft(popupPresentation.width, `${theme} settled dependency popup width`)
      .toBeGreaterThanOrEqual(Math.min(352, scenario.width - 16))
    const prompt = await search.evaluate(async (element) => {
      await document.fonts.ready
      const style = getComputedStyle(element)
      const placeholder = getComputedStyle(element, '::placeholder')
      const canvas = document.createElement('canvas')
      const context = canvas.getContext('2d')
      if (!context)
        throw new Error('Native prompt measurement requires a canvas')
      context.font = `${placeholder.fontWeight} ${placeholder.fontSize} ${placeholder.fontFamily}`
      return {
        text: element.getAttribute('placeholder'),
        required: context.measureText(element.getAttribute('placeholder') ?? '')
          .width,
        available:
          element.clientWidth -
          Number.parseFloat(style.paddingLeft) -
          Number.parseFloat(style.paddingRight),
        width: element.getBoundingClientRect().width,
      }
    })
    expect(prompt.text).toBe('Search dependencies')
    expect
      .soft(
        prompt.required,
        `${theme} input${prompt.width}px full native prompt must fit`,
      )
      .toBeLessThanOrEqual(prompt.available + 1)
    for (const [worker, chain] of [
      ['state', 'ade → state → ade'],
      ['storage', 'ade → storage → state → ade'],
    ]) {
      await search.fill(worker)
      const option = page.getByRole('option', {
        name: new RegExp(`^${worker}\\b`),
      })
      await expect(option).toHaveAttribute('aria-disabled', 'true', {
        timeout: 1500,
      })
      await expect(option).toContainText(chain)
      const reason = option.getByText(/Would create a startup cycle:/)
      const presentation = await textPresentation(reason)
      await expect(reason).toBeInViewport({ ratio: 0.99 })
      expect
        .soft(
          presentation.scrollWidth,
          `${theme} ${worker} reason must not truncate`,
        )
        .toBeLessThanOrEqual(presentation.clientWidth + 1)
      expect
        .soft(
          presentation.scrollHeight,
          `${theme} ${worker} reason must not clip lines`,
        )
        .toBeLessThanOrEqual(presentation.clientHeight + 1)
      for (const line of presentation.textBounds) {
        expect
          .soft(
            line.right,
            `${theme} ${worker} full cycle chain must fit its reason`,
          )
          .toBeLessThanOrEqual(presentation.right + 1)
      }
      expect
        .soft(
          presentation.contrast,
          `${theme} ${worker} disabled explanation needs readable effective contrast`,
        )
        .toBeGreaterThanOrEqual(4.5)
      const optionBox = await option.boundingBox()
      if (!optionBox)
        throw new Error('Blocked dependencies must remain visible')
      await page.mouse.click(
        optionBox.x + 20,
        optionBox.y + optionBox.height / 2,
      )
      await expect(search).toBeVisible()
      await search.press('ArrowDown')
      await search.press('Enter')
      await expect(search).toBeVisible()
      await expect(
        page.getByRole('button', {
          name: `Stop waiting for ${worker}`,
          exact: true,
        }),
      ).toHaveCount(0)
    }
    for (const worker of ['agent', 'database']) {
      await search.fill(worker)
      await expect(search).toHaveAccessibleDescription('1 option')
      const option = page.getByRole('option', {
        name: new RegExp(`^${worker}\\b`),
      })
      await expect(option).not.toHaveAttribute('aria-disabled', 'true')
      await option.click()
      await expect(
        page.getByRole('button', {
          name: `Stop waiting for ${worker}`,
          exact: true,
        }),
      ).toBeVisible()
      if (worker === 'agent') await add.click()
    }
    await expect(
      page.getByRole('button', { name: 'Save and restart', exact: true }),
    ).toBeEnabled()
    expect(await editCalls(page)).toEqual([])
  }
})

test('adds a dependency through a searchable picker', async ({ page }) => {
  await openSettings(page)
  await page
    .getByRole('button', { name: 'Add dependency', exact: true })
    .click()
  await page
    .getByRole('combobox', { name: 'Search dependencies', exact: true })
    .fill('session')
  await page.getByRole('option', { name: /^session-manager/ }).click()
  await expect(
    page.getByRole('button', {
      name: 'Stop waiting for session-manager',
      exact: true,
    }),
  ).toBeVisible()
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await expect
    .poll(async () => (await editCalls(page))[0]?.payload.start_after)
    .toEqual(['state', 'storage', 'session-manager'])
})

test('saves through the real pane command dispatcher with Mod+S while editing', async ({
  page,
}) => {
  const command = await openSettings(page, '?commands=1')
  await command.fill(editedCommand)
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__WORKERS_HARNESS__
          .commands()
          .find((command) => command.id === 'save-settings'),
      ),
    )
    .toMatchObject({ bindings: ['Mod+S'], enabled: true })
  await command.press('ControlOrMeta+s')
  await expect(
    page.getByText('Settings saved for ade', { exact: true }).first(),
  ).toBeVisible()
  expect(await editCalls(page)).toHaveLength(1)
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.dirty))
    .toBe(false)
})

test('ignores Mod+S while validation fails and prevents a second pending save', async ({
  page,
}) => {
  const command = await openSettings(page, '?commands=1')
  await page.getByRole('button', { name: 'Add variable', exact: true }).click()
  const name = page.getByRole('textbox', { name: 'Variable name', exact: true })
  await name.fill('API-KEY')
  await name.press('ControlOrMeta+s')
  expect(await editCalls(page)).toEqual([])
  await page
    .getByRole('button', { name: 'Remove API-KEY', exact: true })
    .click()
  await command.fill(editedCommand)
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.saveDelay = 500
  })
  await command.press('ControlOrMeta+s')
  await expect.poll(async () => (await editCalls(page)).length).toBe(1)
  await page.keyboard.press('ControlOrMeta+s')
  await expect(
    page.getByText('Settings saved for ade', { exact: true }).first(),
  ).toBeVisible()
  expect(await editCalls(page)).toHaveLength(1)
})

test('masks a secret replacement in the preview and cancellation restores the unchanged secret', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1000, height: 900 })
  const command = await openSettings(page)
  await command.fill(editedCommand)
  await scrollSettingsBottom(page)
  await page.getByRole('button', { name: 'Discard', exact: true }).click()
  await page.getByRole('button', { name: 'Add variable', exact: true }).click()
  await expect(
    page.getByRole('textbox', { name: 'Variable name', exact: true }),
  ).toHaveCount(1)
  await page
    .getByRole('button', { name: 'Remove variable', exact: true })
    .click()
  await page.getByRole('button', { name: 'Replace', exact: true }).click()
  const secret = page.getByLabel('API_TOKEN value', { exact: true })
  await expect(secret).toHaveAttribute('type', 'password')
  const save = page.getByRole('button', {
    name: 'Save and restart',
    exact: true,
  })
  await expect(save).toBeDisabled()
  const replacement = 'sk-fixture-never-echo-this-secret'
  await secret.fill(replacement)
  await page
    .getByRole('button', { name: 'Preview changes', exact: true })
    .click()
  await expect(page.locator('.wk-diff')).toBeVisible()
  await expect(
    page.locator('.wk-diff').getByText(replacement, { exact: false }),
  ).toHaveCount(0)
  expect(
    (await page.locator('.wk-diff').locator('*').allTextContents()).join('\n'),
  ).not.toContain(replacement)
  await page
    .getByRole('button', { name: 'Cancel replacing API_TOKEN', exact: true })
    .click()
  await expect(secret).toHaveCount(0)
  await expect(save).toBeDisabled()
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.dirty))
    .toBe(false)
  await expect(
    page.getByRole('region', { name: 'Review pending changes' }),
  ).toHaveCount(0)
  const cleanPreview = page.getByRole('button', {
    name: 'Preview changes',
    exact: true,
  })
  await expect(cleanPreview).toBeDisabled()
  await expect(cleanPreview).toHaveAttribute('aria-expanded', 'false')
  expect(await editCalls(page)).toEqual([])
})

test('writes a secret only when explicitly replaced and masks it again after saving', async ({
  page,
}) => {
  await openSettings(page)
  await page.getByRole('button', { name: 'Replace', exact: true }).click()
  await page
    .getByLabel('API_TOKEN value', { exact: true })
    .fill('sk-new-fixture-secret')
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await expect(
    page.getByText('Settings saved for ade', { exact: true }).first(),
  ).toBeVisible()
  expect((await editCalls(page))[0].payload.environment).toEqual({
    set: { API_TOKEN: 'sk-new-fixture-secret' },
    unset: [],
  })
  await expect(page.getByLabel('API_TOKEN value', { exact: true })).toHaveCount(
    0,
  )
  await expect(
    page.getByRole('button', { name: 'Replace', exact: true }),
  ).toBeVisible()
  expect(await page.locator('body').textContent()).not.toContain(
    'sk-new-fixture-secret',
  )
})

test('finishes a delayed save for its original worker while retaining another worker draft', async ({
  page,
}) => {
  const command = await openSettings(page)
  await command.fill(editedCommand)
  await selectWorker(page, 'agent')
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await command.fill('cargo run --locked --bin agent')
  await selectWorker(page)
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.saveDelay = 700
  })
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await expect.poll(async () => (await editCalls(page)).length).toBe(1)
  await selectWorker(page, 'agent')
  await expect(command).toHaveValue('cargo run --locked --bin agent')
  await expect(command).toBeEnabled()
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.savedCount))
    .toBe(1)
  await expect
    .poll(() => page.evaluate(() => String(window.__WORKERS_HARNESS__.dirty)))
    .toBe('Unsaved settings: agent')
  await selectWorker(page)
  await expect(command).toHaveValue(editedCommand)
  await expect(
    page.getByRole('button', { name: 'Save and restart', exact: true }),
  ).toBeDisabled()
  await selectWorker(page, 'agent')
  await expect(command).toHaveValue('cargo run --locked --bin agent')
})

test('does not change the active tab when an earlier Settings visit finishes saving', async ({
  page,
}) => {
  const command = await openSettings(page)
  await command.fill(editedCommand)
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.saveDelay = 700
  })
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await selectWorker(page, 'agent')
  await selectWorker(page)
  await page.getByRole('tab', { name: 'Log', exact: true }).click()
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.dirty))
    .toBe(false)
  await expect(
    page.getByRole('tab', { name: 'Log', exact: true }),
  ).toHaveAttribute('aria-selected', 'true')
})

test('recovers a failed declaration read through Retry without submitting a mutation', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await page.goto(`${fixture}?loadFailures=1`)
  await selectWorker(page)
  await page.getByRole('tab', { name: /^Settings/ }).click()
  await expect(
    page.getByText('Settings for ade could not be loaded', { exact: true }),
  ).toBeVisible()
  const retry = page.getByRole('button', { name: 'Retry', exact: true })
  const detail = page.getByText(
    /Mock compose declaration is temporarily unavailable/,
  )
  const retryBox = await retry.boundingBox()
  const detailBox = await detail.boundingBox()
  if (!retryBox || !detailBox)
    throw new Error('Retry and its error detail must be visible')
  expect
    .soft(retryBox.height, 'Phone Retry must provide a 48px target')
    .toBeGreaterThanOrEqual(48)
  expect
    .soft(retryBox.width, 'Phone Retry must fill the detail width')
    .toBeGreaterThanOrEqual(detailBox.width - 1)
  expect
    .soft(retryBox.y, 'Phone Retry belongs below its full detail')
    .toBeGreaterThanOrEqual(detailBox.y + detailBox.height)
  await expect(retry).toBeInViewport({ ratio: 0.99 })
  await retry.click()
  await expect(
    page.getByRole('textbox', { name: 'Command', exact: true }),
  ).toHaveValue('./target/debug/ade')
  expect(await editCalls(page)).toEqual([])
})

test('recognizes a persisted declaration when the compose operation fails during restart', async ({
  page,
}) => {
  const command = await openSettings(page)
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.saveMode = 'restart-error'
  })
  await command.fill(editedCommand)
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await expect(
    page.getByText('Configuration saved; restart failed', { exact: true }),
  ).toBeVisible()
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.dirty))
    .toBe(false)
  await expect(command).toHaveValue(editedCommand)
  await page.getByRole('button', { name: 'Retry restart', exact: true }).click()
  await expect(page.getByText('ade restarted', { exact: true })).toBeVisible()
  expect(await editCalls(page)).toHaveLength(1)
})

test('retains the saved draft after a reload failure and disables fields while retrying the read', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 })
  const command = await openSettings(page)
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.saveMode = 'reload-error'
  })
  await command.fill(editedCommand)
  await scrollSettingsBottom(page)
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await expect(
    page.getByText('Settings saved; declaration could not be reloaded', {
      exact: true,
    }),
  ).toBeVisible()
  const result = page.getByRole('region', { name: 'Worker settings result' })
  const detail = result.getByText(/Mock saved declaration cannot be read/)
  const retry = result.getByRole('button', {
    name: 'Reload saved settings',
    exact: true,
  })
  await expect(result).toBeFocused()
  await expect(detail).toBeInViewport({ ratio: 0.99 })
  await expect(retry).toBeInViewport({ ratio: 0.99 })
  const detailBox = await detail.boundingBox()
  const retryBox = await retry.boundingBox()
  if (!detailBox || !retryBox)
    throw new Error('The recovery detail and action must both be visible')
  expect(retryBox.y).toBeGreaterThanOrEqual(detailBox.y + detailBox.height)
  expect(retryBox.height).toBeGreaterThanOrEqual(48)
  expect(retryBox.width).toBeGreaterThanOrEqual(detailBox.width - 1)
  await expect(command).toHaveValue(editedCommand)
  await expect(
    page.getByRole('button', { name: 'Save and restart', exact: true }),
  ).toBeDisabled()
  await expect(
    page.getByRole('region', { name: 'Apply worker settings', exact: true }),
  ).toContainText('Saved; awaiting verification')
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.dirty))
    .toBe(false)
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.saveMode = 'success'
    window.__WORKERS_HARNESS__.readDelay = 600
  })
  await page
    .getByRole('button', { name: 'Reload saved settings', exact: true })
    .click()
  await expect(command).toBeDisabled()
  await expect(
    page.getByRole('button', { name: 'Reload saved settings', exact: true }),
  ).toBeDisabled()
  await expect(
    page.getByText('Saved settings reloaded', { exact: true }),
  ).toBeVisible()
  await expect(command).toBeEnabled()
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.dirty))
    .toBe(false)
  expect(await editCalls(page)).toHaveLength(1)
})

test('reports a package restart failure separately from its saved configuration', async ({
  page,
}) => {
  const command = await openSettings(page, '?package=ade')
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.restartMode = 'error'
  })
  await command.fill(editedCommand)
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await expect(
    page.getByText('Configuration saved; restart failed', { exact: true }),
  ).toBeVisible()
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.dirty))
    .toBe(false)
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.restartMode = 'success'
  })
  await page.getByRole('button', { name: 'Retry restart', exact: true }).click()
  await expect(page.getByText('ade restarted', { exact: true })).toBeVisible()
  expect(await editCalls(page)).toHaveLength(1)
})

test('recovers when a package save persisted but both restart and declaration reload failed', async ({
  page,
}) => {
  const command = await openSettings(page, '?package=ade')
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.saveMode = 'reload-error'
    window.__WORKERS_HARNESS__.restartMode = 'error'
  })
  await command.fill(editedCommand)
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await expect(
    page.getByText('Configuration saved; restart failed', { exact: true }),
  ).toBeVisible()
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.dirty))
    .toBe(false)
  await expect(command).toHaveValue(editedCommand)
  await expect(
    page.getByRole('button', { name: 'Save and restart', exact: true }),
  ).toBeDisabled()
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.restartMode = 'success'
  })
  await page.getByRole('button', { name: 'Retry restart', exact: true }).click()
  await expect(
    page.getByText('Restart completed; saved settings could not be reloaded', {
      exact: true,
    }),
  ).toBeVisible()
  await expect(command).toHaveValue(editedCommand)
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.saveMode = 'success'
    window.__WORKERS_HARNESS__.readDelay = 500
  })
  await page
    .getByRole('button', { name: 'Reload saved settings', exact: true })
    .click()
  await expect(command).toBeDisabled()
  await expect(
    page.getByText('Saved settings reloaded', { exact: true }),
  ).toBeVisible()
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.dirty))
    .toBe(false)
  await expect(command).toHaveValue(editedCommand)
  expect(await editCalls(page)).toHaveLength(1)
})

test('states when configuration saved but the runtime snapshot could not be refreshed', async ({
  page,
}) => {
  const command = await openSettings(page)
  await page.evaluate(() => {
    window.__WORKERS_HARNESS__.snapshotMode = 'error'
  })
  await command.fill(editedCommand)
  await page
    .getByRole('button', { name: 'Save and restart', exact: true })
    .click()
  await expect(
    page.getByText(/Runtime state could not be refreshed/),
  ).toBeVisible()
  await expect
    .poll(() => page.evaluate(() => window.__WORKERS_HARNESS__.dirty))
    .toBe(false)
  expect(await editCalls(page)).toHaveLength(1)
})

for (const theme of ['dark', 'light']) {
  for (const viewport of [
    { width: 1656, height: 1314 },
    { width: 1000, height: 900 },
    { width: 390, height: 844 },
    { width: 1656, height: 1314, pane: 720 },
  ]) {
    test(`keeps Save visible and the pane within its width in ${theme} at ${viewport.width}${'pane' in viewport ? `/${viewport.pane} pane` : ''}`, async ({
      page,
    }) => {
      await page.setViewportSize(viewport)
      const parameters = `?theme=${theme}&commands=1${'pane' in viewport ? `&pane=${viewport.pane}` : ''}`
      const command = await openSettings(page, parameters)
      await command.fill(editedCommand)
      const save = page.getByRole('button', {
        name: 'Save and restart',
        exact: true,
      })
      await expect(save).toBeInViewport()
      const applybar = page.getByRole('region', {
        name: 'Apply worker settings',
        exact: true,
      })
      const barBounds = await applybar.boundingBox()
      if (!barBounds)
        throw new Error('Apply worker settings must have visible bounds')
      expect(barBounds.height).toBeLessThan(200)
      const note = await applybar.locator('.wk-note').evaluate((element) => ({
        height: element.getBoundingClientRect().height,
        lineHeight: Number.parseFloat(getComputedStyle(element).lineHeight),
      }))
      expect(note.height).toBeLessThanOrEqual(note.lineHeight * 4 + 1)
      await page.locator('.wk-main').evaluate((element) => {
        element.scrollTop = element.scrollHeight
      })
      await expect(save).toBeInViewport()
      const pane = await page.locator('.workers-harness-pane').boundingBox()
      const saveBounds = await save.boundingBox()
      expect(pane).not.toBeNull()
      expect(saveBounds).not.toBeNull()
      if (!pane || !saveBounds)
        throw new Error(
          'The settings pane and Save button must have visible bounds',
        )
      expect(saveBounds.x).toBeGreaterThanOrEqual(pane.x)
      expect(saveBounds.x + saveBounds.width).toBeLessThanOrEqual(
        pane.x + pane.width + 1,
      )
      const overflow = await page.evaluate(
        () =>
          document.documentElement.scrollWidth >
          document.documentElement.clientWidth,
      )
      expect(overflow).toBe(false)
    })
  }
}
