import type { Locator, Page, TestInfo } from '@playwright/test'
import { registerWorker } from 'iii-browser-sdk'
import { expect, test } from './shell-terminal-stack'

async function openShell(page: Page, consoleUrl: string): Promise<void> {
  await page.goto(`${consoleUrl}/#/ext/shell`, { waitUntil: 'networkidle' })
  const shellTabs = page.getByRole('tab', { name: /^shell(?: close shell)?$/ })
  if ((await shellTabs.count()) === 0) {
    await page.getByRole('button', { name: 'new tab', exact: true }).click()
    await page
      .getByRole('option', { name: /shell Interactive shell sessions/ })
      .click()
  } else {
    await shellTabs.last().click()
  }
  const open = page.getByRole('button', { name: 'Open terminal', exact: true })
  await open.click()
  await expect(page.locator('.shui-terminal-state').first()).toHaveText('ready')
}

async function runInPane(
  page: Page,
  pane: Locator,
  command: string,
): Promise<void> {
  await pane.locator('.xterm-helper-textarea').focus()
  await page.keyboard.type(command, { delay: 1 })
  await page.keyboard.press('Enter')
}

function markerFromText(text: string, prefix: string): string | null {
  const match = text.match(
    new RegExp(`${prefix.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}([^\\s]+)`),
  )
  return match?.[1] ?? null
}

async function markerValue(pane: Locator, prefix: string): Promise<string> {
  await expect
    .poll(async () =>
      markerFromText(await pane.locator('.xterm-rows').innerText(), prefix),
    )
    .not.toBeNull()
  const value = markerFromText(
    await pane.locator('.xterm-rows').innerText(),
    prefix,
  )
  if (value === null) throw new Error(`terminal marker ${prefix} was not found`)
  return value
}

async function captureEvidence(
  page: Page,
  testInfo: TestInfo,
  name: string,
): Promise<void> {
  const outputPath = testInfo.outputPath(`${name}.png`)
  await page.screenshot({ path: outputPath })
  await testInfo.attach(name, { path: outputPath, contentType: 'image/png' })
}

test('runs multi-terminal PTYs, replay, tmux, Claude, and cleanup', async ({
  page,
  shellStack,
}, testInfo) => {
  const consoleErrors: string[] = []
  const failedRequests: string[] = []
  const badResponses: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })
  page.on('requestfailed', (request) => {
    failedRequests.push(
      `${request.method()} ${request.url()}: ${request.failure()?.errorText ?? 'failed'}`,
    )
  })
  page.on('response', (response) => {
    if (response.status() >= 400 && !response.url().endsWith('/favicon.ico')) {
      badResponses.push(`${response.status()} ${response.url()}`)
    }
  })

  await openShell(page, shellStack.consoleUrl)
  await page.getByRole('button', { name: 'Dock terminal on right' }).click()
  await expect(page.locator('.shui-terminal-panel')).toHaveAttribute(
    'data-terminal-dock',
    'right',
  )
  await expect(
    page.getByRole('separator', { name: 'Resize right terminal' }),
  ).toBeVisible()
  await page
    .getByRole('button', { name: 'Open terminal as an editor tab' })
    .click()
  await expect(page.locator('.shui-terminal-panel')).toHaveAttribute(
    'data-terminal-dock',
    'editor',
  )
  await captureEvidence(page, testInfo, 'task8-docking')
  await page.getByRole('button', { name: 'Dock terminal at bottom' }).click()
  await expect(page.locator('.shui-terminal-panel')).toHaveAttribute(
    'data-terminal-dock',
    'bottom',
  )
  const terminalResize = page.getByRole('separator', {
    name: 'Resize bottom terminal',
  })
  const terminalResizeBox = await terminalResize.boundingBox()
  if (!terminalResizeBox)
    throw new Error('terminal resize handle has no bounds')
  await page.mouse.move(
    terminalResizeBox.x + terminalResizeBox.width / 2,
    terminalResizeBox.y + terminalResizeBox.height / 2,
  )
  await page.mouse.down()
  await page.mouse.move(
    terminalResizeBox.x + terminalResizeBox.width / 2,
    terminalResizeBox.y + terminalResizeBox.height / 2 - 220,
  )
  await page.mouse.up()
  await page.getByRole('button', { name: 'New terminal' }).click()
  await expect(page.locator('.shui-terminal-state')).toHaveText(['ready'])
  await page.getByRole('button', { name: 'Split down' }).click()
  await expect(page.locator('[data-terminal-pane-id]')).toHaveCount(2)
  await expect(page.locator('.shui-terminal-state')).toHaveText([
    'ready',
    'ready',
  ])
  await page.getByRole('button', { name: 'Split right' }).first().click()
  await expect(page.locator('[data-terminal-pane-id]')).toHaveCount(3)
  await expect(page.locator('.shui-terminal-state')).toHaveText([
    'ready',
    'ready',
    'ready',
  ])

  let panes = page.locator('[data-terminal-pane-id]')
  for (let index = 0; index < 3; index += 1) {
    await expect(panes.nth(index).locator('.xterm-rows')).toContainText(
      'iii-e2e',
    )
  }
  for (let index = 0; index < 3; index += 1) {
    const pane = panes.nth(index)
    await runInPane(
      page,
      pane,
      `sleep 0.2; printf '%s%s%s\\n' '__PANE_' '${index}' '__'; sleep 0.2`,
    )
    await expect(pane.locator('.xterm-rows')).toContainText(`__PANE_${index}__`)
  }
  await captureEvidence(page, testInfo, 'task8-tabs-splits')

  const terminalTabs = page.getByRole('tab', { name: /^zsh [12]$/ })
  await expect(terminalTabs).toHaveCount(2)
  await terminalTabs.first().click()
  await expect(page.locator('.shui-terminal-state')).toHaveText(['ready'])
  let tabPane = page.locator('[data-terminal-pane-id]').first()
  await expect(tabPane.locator('.xterm-rows')).toContainText('iii-e2e')
  await runInPane(
    page,
    tabPane,
    `printf '%s%s%s\\n' '__TAB_ONE_PID_' 'BEFORE__:' "$$"; sleep 0.2`,
  )
  const tabOnePid = await markerValue(tabPane, '__TAB_ONE_PID_BEFORE__:')

  await terminalTabs.last().click()
  await expect(page.locator('.shui-terminal-state')).toHaveText([
    'ready',
    'ready',
    'ready',
  ])
  panes = page.locator('[data-terminal-pane-id]')
  await expect(panes.first().locator('.xterm-rows')).toContainText('__PANE_0__')
  await runInPane(
    page,
    panes.first(),
    `printf '%s%s%s\\n' '__TAB_TWO_PID_' 'BEFORE__:' "$$"; sleep 0.2`,
  )
  const tabTwoPid = await markerValue(panes.first(), '__TAB_TWO_PID_BEFORE__:')

  await terminalTabs.first().click()
  await expect(page.locator('.shui-terminal-state')).toHaveText(['ready'])
  tabPane = page.locator('[data-terminal-pane-id]').first()
  await expect(tabPane.locator('.xterm-rows')).toContainText(
    '__TAB_ONE_PID_BEFORE__',
  )
  await runInPane(
    page,
    tabPane,
    `printf '%s%s%s\\n' '__TAB_ONE_PID_' 'AFTER__:' "$$"; sleep 0.2`,
  )
  expect(await markerValue(tabPane, '__TAB_ONE_PID_AFTER__:')).toBe(tabOnePid)

  await terminalTabs.last().click()
  await expect(page.locator('.shui-terminal-state')).toHaveText([
    'ready',
    'ready',
    'ready',
  ])
  panes = page.locator('[data-terminal-pane-id]')
  await runInPane(
    page,
    panes.first(),
    `printf '%s%s%s\\n' '__TAB_TWO_PID_' 'AFTER__:' "$$"; sleep 0.2`,
  )
  expect(await markerValue(panes.first(), '__TAB_TWO_PID_AFTER__:')).toBe(
    tabTwoPid,
  )

  const resizedPane = panes.first()
  await runInPane(
    page,
    resizedPane,
    `stty size | awk '{print "__SIZE_" "BEFORE__:" $1 "x" $2}'`,
  )
  const sizeBefore = await markerValue(resizedPane, '__SIZE_BEFORE__:')
  const separator = page
    .locator('[role="separator"][aria-orientation="vertical"]')
    .first()
  const separatorBox = await separator.boundingBox()
  if (!separatorBox) throw new Error('terminal split separator has no bounds')
  await page.mouse.move(
    separatorBox.x + separatorBox.width / 2,
    separatorBox.y + separatorBox.height / 2,
  )
  await page.mouse.down()
  await page.mouse.move(
    separatorBox.x + separatorBox.width / 2 + 80,
    separatorBox.y + separatorBox.height / 2,
  )
  await page.mouse.up()
  await runInPane(
    page,
    resizedPane,
    `stty size | awk '{print "__SIZE_" "AFTER__:" $1 "x" $2}'`,
  )
  expect(await markerValue(resizedPane, '__SIZE_AFTER__:')).not.toBe(sizeBefore)

  await runInPane(
    page,
    resizedPane,
    `for i in {1..120}; do printf '%s%03d\\n' '__SCROLL_LINE__:' "$i"; done; sleep 0.5; printf '%s%s%d\\n' '__SCROLL_' 'DONE__:' 120; sleep 0.2`,
  )
  await expect(resizedPane.locator('.xterm-rows')).toContainText(
    '__SCROLL_DONE__:120',
  )
  await resizedPane.locator('.xterm').hover()
  await page.mouse.wheel(0, -2400)
  const jump = resizedPane.getByRole('button', {
    name: 'Jump to latest output',
  })
  await expect(jump).toBeVisible()
  await captureEvidence(page, testInfo, 'task8-jump-paused')
  await jump.click()
  await expect(jump).toBeHidden()
  await captureEvidence(page, testInfo, 'task8-jump-latest')

  await runInPane(
    page,
    resizedPane,
    `printf '%s%s%s\\n' '__PID_' 'BEFORE__:' "$$"; sleep 0.2`,
  )
  const pidBefore = await markerValue(resizedPane, '__PID_BEFORE__:')
  await page.waitForTimeout(800)
  await runInPane(
    page,
    resizedPane,
    `sh -c 'printf "%s%s\\n" "__REPLAY_" "ARMED__"; sleep 1; printf "%s%s\\n" "__REPLAY_" "OK__"' &`,
  )
  await expect(resizedPane.locator('.xterm-rows')).toContainText(
    '__REPLAY_ARMED__',
  )
  await page.reload({ waitUntil: 'networkidle' })
  await page
    .getByRole('tab', { name: /^shell(?: close shell)?$/ })
    .last()
    .click()
  await expect(page.locator('[data-terminal-pane-id]')).toHaveCount(3)
  await expect(page.locator('.shui-terminal-state')).toHaveText([
    'ready',
    'ready',
    'ready',
  ])
  panes = page.locator('[data-terminal-pane-id]')
  const reloadedPane = panes.first()
  await expect(reloadedPane.locator('.xterm-rows')).toContainText(
    '__REPLAY_OK__',
  )
  await runInPane(
    page,
    reloadedPane,
    `printf '%s%s%s\\n' '__PID_' 'AFTER__:' "$$"; sleep 0.2`,
  )
  expect(await markerValue(reloadedPane, '__PID_AFTER__:')).toBe(pidBefore)
  const replayText = await reloadedPane.locator('.xterm-rows').innerText()
  expect(replayText.match(/__REPLAY_OK__/g)).toHaveLength(1)
  await captureEvidence(page, testInfo, 'task8-reload-replay')

  const tmuxSocket = shellStack.createTmuxSocket()
  const tmux = `tmux -L ${tmuxSocket}`
  await runInPane(
    page,
    reloadedPane,
    `${tmux} new-session -s terminal-acceptance`,
  )
  await page.waitForTimeout(500)
  await page.keyboard.press('Control+b')
  await page.keyboard.insertText('%')
  await page.waitForTimeout(300)
  await page.keyboard.type(`printf '%s%s\\n' '__TMUX_' 'RIGHT__'; sleep 0.2`, {
    delay: 1,
  })
  await page.keyboard.press('Enter')
  await page.waitForTimeout(300)
  await page.keyboard.press('Control+b')
  await page.keyboard.insertText('o')
  await page.waitForTimeout(200)
  await page.keyboard.type(`printf '%s%s\\n' '__TMUX_' 'LEFT__'; sleep 0.2`, {
    delay: 1,
  })
  await page.keyboard.press('Enter')
  await page.waitForTimeout(300)
  await expect(reloadedPane.locator('.xterm-rows')).toContainText(
    '__TMUX_LEFT__',
  )
  await expect(reloadedPane.locator('.xterm-rows')).toContainText(
    '__TMUX_RIGHT__',
  )
  await captureEvidence(page, testInfo, 'task8-tmux-attached')
  await page.keyboard.press('Control+b')
  await page.keyboard.insertText('d')
  await runInPane(
    page,
    reloadedPane,
    `${tmux} capture-pane -pt terminal-acceptance:0.0; ${tmux} capture-pane -pt terminal-acceptance:0.1; sleep 0.2`,
  )
  await runInPane(
    page,
    reloadedPane,
    `${tmux} attach-session -t terminal-acceptance`,
  )
  await page.waitForTimeout(300)
  await page.keyboard.type(
    `printf '%s%s\\n' '__TMUX_' 'REATTACHED__'; sleep 0.2`,
    {
      delay: 1,
    },
  )
  await page.keyboard.press('Enter')
  await page.waitForTimeout(300)
  await expect(reloadedPane.locator('.xterm-rows')).toContainText(
    '__TMUX_REATTACHED__',
  )
  await page.keyboard.press('Control+b')
  await page.keyboard.insertText('d')
  await runInPane(
    page,
    reloadedPane,
    `${tmux} capture-pane -pt terminal-acceptance:0.0; ${tmux} capture-pane -pt terminal-acceptance:0.1; ${tmux} kill-server`,
  )

  await runInPane(
    page,
    reloadedPane,
    `claude --version >/tmp/iii-claude-version 2>&1; code=$?; printf '%s%s%d\\n' '__CLAUDE_' 'EXIT__:' "$code"; sed -n '1p' /tmp/iii-claude-version`,
  )
  expect(await markerValue(reloadedPane, '__CLAUDE_EXIT__:')).toBe('0')

  const closingPane = panes.last()
  const closingPaneId = await closingPane.getAttribute('data-terminal-pane-id')
  if (!closingPaneId) throw new Error('closing pane has no pane ID')
  const lease = await page.evaluate((paneId) => {
    for (const key of Object.keys(localStorage)) {
      if (!key.startsWith('iii::shell-ui::terminal-leases::')) continue
      const value = localStorage.getItem(key)
      if (!value) continue
      const candidate = (
        JSON.parse(value) as Array<{
          paneId: string
          sessionId: string
          reconnectToken: string
        }>
      ).find((entry) => entry.paneId === paneId)
      if (candidate) return candidate
    }
    return null
  }, closingPaneId)
  expect(lease).not.toBeNull()
  await closingPane.getByRole('button', { name: 'Close terminal pane' }).click()
  await expect(page.locator('[data-terminal-pane-id]')).toHaveCount(2)

  if (!lease) throw new Error('closed pane lease was not found')
  const probe = registerWorker(shellStack.engineUrl)
  try {
    const closedAttachError = await probe
      .trigger({
        function_id: 'shell::pty::attach',
        payload: {
          session_id: lease.sessionId,
          reconnect_token: lease.reconnectToken,
          output_function_id: `iii::shell-ui::pty-output::console-probe-${Date.now()}`,
          cols: 80,
          rows: 24,
          after_sequence: 0,
        },
      })
      .catch((error: unknown) => error)
    expect(
      closedAttachError instanceof Error
        ? closedAttachError.message
        : String(closedAttachError),
    ).toContain('terminal session does not exist')
  } finally {
    await probe.shutdown()
  }

  const newTerminal = page.getByRole('button', { name: 'New terminal' })
  let createdTerminals = 0
  for (
    let index = 0;
    index < 20 && !(await newTerminal.isDisabled());
    index += 1
  ) {
    await newTerminal.click()
    await expect(page.locator('.shui-terminal-state')).toHaveText(['ready'])
    createdTerminals += 1
  }
  expect(createdTerminals).toBe(13)
  await expect(newTerminal).toBeDisabled()
  await captureEvidence(page, testInfo, 'task8-session-cap')
  await page
    .getByRole('button', { name: 'Close disconnected terminals' })
    .click()
  await expect(newTerminal).toBeEnabled({ timeout: 30_000 })
  await expect(page.locator('.shui-terminal-tab')).toHaveCount(1, {
    timeout: 30_000,
  })
  for (let index = 0; index < 15; index += 1) {
    await newTerminal.click()
    await expect(page.locator('.shui-terminal-state')).toHaveText(['ready'])
  }
  await expect(newTerminal).toBeDisabled()
  await captureEvidence(page, testInfo, 'task8-permit-refill')

  expect(consoleErrors).toEqual([])
  expect(failedRequests).toEqual([])
  expect(badResponses).toEqual([])
})
