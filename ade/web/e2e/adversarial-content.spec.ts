import { createServer } from 'node:http'
import type { AddressInfo } from 'node:net'
import type { Locator, Page } from '@playwright/test'
import { expect, expectPassingResult, openSession, test } from './harness-stack'

const TOOL_ARGUMENT =
  'TOOL_ATTACK_SENTINEL </code><img src="x" onerror="globalThis.__consoleToolXss=1"><script>globalThis.__consoleToolXss=2</script>'
const USER_PAYLOAD =
  'USER_ATTACK_SENTINEL <img src="https://hostile.invalid/user.png" onerror="globalThis.__consoleUserXss=1"> [unsafe user link](javascript:globalThis.__consoleUserXss=2)'

const MARKERS = [
  '__consoleUserXss',
  '__consoleToolXss',
  '__consoleResultXss',
  '__consoleAssistantXss',
]

test.use({ scenario: 'adversarial-content-rendering' })

async function startHostileOrigin(): Promise<{
  url: string
  close: () => Promise<void>
}> {
  const server = createServer((_request, response) => {
    response.writeHead(200, { 'content-type': 'text/html' })
    response.end('<!doctype html><title>hostile origin</title>')
  })
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
  const address = server.address() as AddressInfo
  return {
    url: `http://127.0.0.1:${address.port}/`,
    close: () =>
      new Promise<void>((resolve, reject) => {
        server.close((error) => (error ? reject(error) : resolve()))
      }),
  }
}

async function expectNoExecutablePayload(
  page: Page,
  transcript: Locator,
): Promise<void> {
  const markerTypes = await page.evaluate((markers) => {
    const scope = globalThis as unknown as Record<string, unknown>
    return markers.map((marker) => typeof scope[marker])
  }, MARKERS)
  expect(markerTypes).toEqual(MARKERS.map(() => 'undefined'))
  await expect(
    transcript.locator(
      'script, iframe, object, embed, [onerror], [onload], img[src="x"], img[src^="data:"]',
    ),
  ).toHaveCount(0)
}

test('keeps hostile user, model, and function content inert after reload', async ({
  page,
  stack,
}) => {
  const hostileRequests: string[] = []
  page.on('request', (request) => {
    if (new URL(request.url()).hostname === 'hostile.invalid') {
      hostileRequests.push(request.url())
    }
  })
  await page.route('https://hostile.invalid/**', (route) => route.abort())
  await page.addInitScript((markers) => {
    const scope = globalThis as unknown as Record<string, unknown>
    for (const marker of markers) delete scope[marker]
  }, MARKERS)

  const completed = stack.waitForTurnCompleted()
  await stack.trigger()
  expect(await completed).toMatchObject({ status: 'completed' })

  await openSession(page, stack)
  const transcript = page.locator(
    `[data-chat-session-id="${stack.ready.session.id}"]`,
  )
  const assistant = transcript.locator('[data-message-role="assistant"]', {
    hasText: 'ASSISTANT_ATTACK_SENTINEL',
  })
  await expect(assistant).toHaveCount(1)
  const unsafeLink = assistant.getByText('unsafe assistant link', {
    exact: true,
  })
  await expect(unsafeLink).toHaveCount(1)
  const unsafeHref = (await unsafeLink.getAttribute('href')) ?? ''
  expect(unsafeHref).not.toMatch(/^(?:javascript|data):/i)
  const unsafeImage = assistant.getByRole('img', {
    name: 'unsafe data image',
    exact: true,
  })
  await expect(unsafeImage).toHaveCount(1)
  const unsafeSrc = (await unsafeImage.getAttribute('src')) ?? ''
  expect(unsafeSrc).not.toMatch(/^(?:javascript|data):/i)
  const safeLink = assistant.getByRole('link', {
    name: 'safe external link',
    exact: true,
  })
  await expect(safeLink).toHaveAttribute(
    'href',
    'https://example.com/adversarial-content',
  )
  await expect(safeLink).toHaveAttribute('target', '_blank')
  await expect(safeLink).toHaveAttribute('rel', 'noopener noreferrer')

  const functionId = stack.ready.functions.adversarial_echo
  expect(functionId).toBeTruthy()
  const card = transcript.locator('[data-message-role="function-call"]', {
    hasText: functionId,
  })
  await expect(card).toHaveCount(1)
  await card.getByRole('button', { name: functionId }).click()
  const requestPane = card.locator('[data-function-pane="request"]')
  const responsePane = card.locator('[data-function-pane="response"]')
  await expect(requestPane).toContainText(TOOL_ARGUMENT)
  await expect(responsePane).toContainText('TOOL_RESULT_SENTINEL')
  await expectNoExecutablePayload(page, transcript)
  expect(hostileRequests).toEqual([])

  const secondCompleted = stack.waitForTurnCompleted()
  const composer = page.getByLabel('message composer')
  await composer.pressSequentially(USER_PAYLOAD)
  await expect(composer).toHaveText(USER_PAYLOAD)
  await page.getByRole('button', { name: 'send message' }).click()
  expect(await secondCompleted).toMatchObject({ status: 'completed' })
  await expect(
    transcript.locator('[data-message-role="user"]', {
      hasText: 'USER_ATTACK_SENTINEL',
    }),
  ).toHaveCount(1)
  await expect(
    transcript.locator('[data-message-role="assistant"]', {
      hasText: 'USER_ATTACK_ACK',
    }),
  ).toHaveCount(1)
  await expectNoExecutablePayload(page, transcript)
  expect(hostileRequests).toEqual([])

  await page.reload()
  await page
    .getByRole('button', {
      name: `open ${stack.ready.session.title}`,
      exact: true,
    })
    .click()
  await expect(
    page.locator('[data-message-role="assistant"]', {
      hasText: 'ASSISTANT_ATTACK_SENTINEL',
    }),
  ).toHaveCount(1)
  await expectNoExecutablePayload(page, transcript)
  expect(hostileRequests).toEqual([])

  const hostileOrigin = await startHostileOrigin()
  try {
    await page.goto(hostileOrigin.url)
    const websocketUrl = `${stack.consoleUrl.replace(/^http/, 'ws')}/ws`
    const outcome = await page.evaluate(
      (url) =>
        new Promise<'opened' | 'rejected' | 'timeout'>((resolve) => {
          const socket = new WebSocket(url)
          const timer = setTimeout(() => resolve('timeout'), 5_000)
          socket.addEventListener(
            'open',
            () => {
              clearTimeout(timer)
              socket.close()
              resolve('opened')
            },
            { once: true },
          )
          socket.addEventListener(
            'error',
            () => {
              clearTimeout(timer)
              resolve('rejected')
            },
            { once: true },
          )
        }),
      websocketUrl,
    )
    expect(outcome).toBe('rejected')
  } finally {
    await hostileOrigin.close()
  }

  expectPassingResult(await stack.finish())
})
