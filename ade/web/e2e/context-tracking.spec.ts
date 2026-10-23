import { type ChildProcess, spawn } from 'node:child_process'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import net from 'node:net'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { expect, test } from '@playwright/test'
import { type ISdk, registerWorker } from 'iii-browser-sdk'
import type { ContextTrackingItem } from '../src/components/chat/ContextTracking'

type RpcResults = {
  'console::status': unknown
  'session::list': unknown
  'session::create': { session_id: string }
  'session::append': unknown
  'console::context::save': ContextTrackingItem
  'console::context::list': { items: ContextTrackingItem[] }
  'console::context::remove': { removed: boolean }
  'console::context::inject-guidance': { mutations: { system_prompt: string } }
}

// Opt-in, real isolated engine + session-manager + the Console built from this tree.
// No LLM/network provider needed; invoke the same public functions an agent uses.
test('chat-owned tracking persists, changes scope and renders safely at every pane width', async ({
  page,
}) => {
  test.skip(
    !process.env.CONTEXT_ENGINE_BIN ||
      !process.env.CONTEXT_SESSION_BIN ||
      !process.env.CONTEXT_CONSOLE_BIN,
    'Set CONTEXT_{ENGINE,SESSION,CONSOLE}_BIN to run the isolated stack',
  )
  const engineBin = process.env.CONTEXT_ENGINE_BIN
  const sessionBin = process.env.CONTEXT_SESSION_BIN
  const consoleBin = process.env.CONTEXT_CONSOLE_BIN
  if (!engineBin || !sessionBin || !consoleBin) return
  test.setTimeout(120_000)
  const dir = await mkdtemp(path.join(tmpdir(), 'context-tracking-'))
  const processes: ChildProcess[] = []
  let sdk: ISdk | undefined
  const freePort = () =>
    new Promise<number>((resolve) => {
      const server = net.createServer()
      server.listen(0, '127.0.0.1', () => {
        const port = (server.address() as net.AddressInfo).port
        server.close(() => resolve(port))
      })
    })
  const enginePort = await freePort()
  const httpPort = await freePort()
  const engineUrl = `ws://127.0.0.1:${enginePort}`
  const base = `http://127.0.0.1:${httpPort}`
  const start = (binary: string, args: string[]) => {
    const child = spawn(binary, args, {
      cwd: dir,
      env: {
        ...process.env,
        III_COMPOSE_DIR: dir,
        III_COMPOSE_NAMESPACE: 'default',
        III_CONFIG_NAME: '',
        III_URL: engineUrl,
        III_CONSOLE_ONBOARDING_AUTO_OPEN: 'false',
      },
      stdio: 'ignore',
    })
    processes.push(child)
    return child
  }
  try {
    await mkdir(path.join(dir, 'project'))
    await writeFile(
      path.join(dir, 'engine.yaml'),
      `workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: ${enginePort}\n  - name: configuration\n    config:\n      adapter:\n        name: fs\n        config:\n          directory: ${dir}/config\n`,
    )
    await writeFile(
      path.join(dir, 'sessions.yaml'),
      `adapter:\n  name: fs\n  config:\n    data_dir: ${dir}/sessions\n`,
    )
    await writeFile(
      path.join(dir, 'console.yaml'),
      `http_port: ${httpPort}\nhttp_host: 127.0.0.1\ndata_dir: ${dir}/console\n`,
    )
    start(engineBin, [
      '--config',
      path.join(dir, 'engine.yaml'),
      '--no-update-check',
    ])
    await expect
      .poll(
        async () =>
          new Promise<boolean>((resolve) => {
            const socket = net.connect(enginePort, '127.0.0.1')
            socket.once('connect', () => {
              socket.destroy()
              resolve(true)
            })
            socket.once('error', () => resolve(false))
          }),
      )
      .toBe(true)
    const client = registerWorker(engineUrl)
    sdk = client
    const call = <K extends keyof RpcResults>(
      id: K,
      payload: Record<string, unknown> = {},
    ): Promise<RpcResults[K]> =>
      client.trigger({ function_id: id, payload }) as Promise<RpcResults[K]>
    start(sessionBin, [
      '--url',
      engineUrl,
      '--config',
      path.join(dir, 'sessions.yaml'),
    ])
    const bootConsole = () =>
      start(consoleBin, [
        '--url',
        engineUrl,
        '--config',
        path.join(dir, 'console.yaml'),
      ])
    let consoleProcess = bootConsole()
    await expect
      .poll(async () => {
        try {
          await call('console::status')
          await call('session::list')
          return true
        } catch {
          return false
        }
      })
      .toBe(true)
    const metadata = {
      surface: 'console',
      fs_scope: { root: path.join(dir, 'project') },
    }
    const a = await call('session::create', {
      title: 'Tracking test A',
      metadata,
    })
    const b = await call('session::create', {
      title: 'Tracking test B',
      metadata,
    })
    const snapshot = {
      title: 'Release v1.8',
      summary: '14 of 20 delivered',
      progress: 70,
      items: [
        { label: 'Tests', done: true },
        { label: 'Ship', done: false },
      ],
      source: { label: 'Release board', url: 'https://example.com/release' },
    }
    let item = await call('console::context::save', {
      session_id: a.session_id,
      snapshot,
    })
    expect(
      (await call('console::context::list', { session_id: b.session_id }))
        .items,
    ).toEqual([])
    const hook = await call('console::context::inject-guidance', {
      session_id: a.session_id,
      generate: {
        system_prompt: 'BASE',
        tools: [{ name: 'console::context::save' }],
      },
    })
    expect(hook.mutations.system_prompt).toContain(item.id)
    await call('session::append', {
      session_id: a.session_id,
      message: {
        role: 'user',
        content: [{ type: 'text', text: 'Track the release in Context.' }],
        timestamp: Date.now(),
      },
    })
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto(base)
    await page
      .getByRole('button', { name: 'open Tracking test A', exact: true })
      .getByText('Tracking test A', { exact: true })
      .click()
    const contextButton = page.getByRole('button', {
      name: /^(Context|Side panel)$/i,
    })
    if (await contextButton.count()) await contextButton.first().click()
    const section = page.getByRole('region', { name: 'Tracking', exact: true })
    await expect(section.getByRole('progressbar')).toHaveAttribute(
      'aria-valuenow',
      '70',
    )
    await expect(section).toContainText('Conversation')
    for (const [width, theme] of [
      [1440, 'light'],
      [780, 'dark'],
      [390, 'light'],
    ] as const) {
      await page.setViewportSize({ width, height: 900 })
      await page.emulateMedia({ reducedMotion: 'reduce' })
      await page.evaluate((t) => {
        document.documentElement.dataset.theme = t
      }, theme)
      // A phone may show chat first; switch to its Context screen if necessary.
      const open = page.getByRole('button', {
        name: /^(Context|Side panel)$/i,
      })
      if ((await open.count()) && !(await section.isVisible()))
        await open.first().click()
      await expect(section.getByRole('progressbar')).toBeVisible()
      expect(
        await section.evaluate((el) => el.scrollWidth <= el.clientWidth),
      ).toBe(true)
    }
    await section.getByRole('button', { name: 'Update in chat' }).click()
    // A draft action never saves a new snapshot or starts a background turn.
    expect(
      (await call('console::context::list', { session_id: a.session_id }))
        .items[0].updated_at,
    ).toBe(item.updated_at)
    item = await call('console::context::save', {
      session_id: a.session_id,
      id: item.id,
      scope: 'project',
      snapshot: { ...snapshot, progress: 80 },
    })
    expect(
      (await call('console::context::list', { session_id: b.session_id }))
        .items[0].progress,
    ).toBe(80)
    // Restart the actual Console, not just the React component.
    consoleProcess.kill('SIGTERM')
    await new Promise((resolve) => consoleProcess.once('exit', resolve))
    consoleProcess = bootConsole()
    await expect
      .poll(async () => {
        try {
          return (
            await call('console::context::list', { session_id: a.session_id })
          ).items[0]?.progress
        } catch {
          return null
        }
      })
      .toBe(80)
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.reload()
    await page
      .getByRole('button', { name: 'open Tracking test A', exact: true })
      .getByText('Tracking test A', { exact: true })
      .click()
    await expect(
      page.locator(`[data-chat-session-id="${a.session_id}"]`),
    ).toHaveAttribute('data-chat-session-hydrated', 'true')
    const reopen = page.getByRole('button', {
      name: /^(Context|Side panel)$/i,
    })
    if (!(await section.isVisible())) await reopen.first().click()
    await expect(section.getByRole('progressbar')).toHaveAttribute(
      'aria-valuenow',
      '80',
    )
    expect(
      await call('console::context::remove', {
        session_id: a.session_id,
        id: item.id,
      }),
    ).toEqual({ removed: true })
    await page.reload()
    await page
      .getByRole('button', { name: 'open Tracking test A', exact: true })
      .getByText('Tracking test A', { exact: true })
      .click()
    await expect(
      page.locator(`[data-chat-session-id="${a.session_id}"]`),
    ).toHaveAttribute('data-chat-session-hydrated', 'true')
    const reopenEmpty = page.getByRole('button', {
      name: /^(Context|Side panel)$/i,
    })
    if (!(await section.isVisible())) await reopenEmpty.first().click()
    await expect(section).toContainText('Ask in chat')
  } finally {
    await sdk?.shutdown().catch(() => {})
    for (const child of processes.reverse()) {
      if (child.exitCode === null) child.kill('SIGTERM')
    }
    await Promise.all(
      processes.map((child) =>
        child.exitCode !== null
          ? Promise.resolve()
          : new Promise((resolve) => {
              child.once('exit', resolve)
              setTimeout(() => {
                child.kill('SIGKILL')
                resolve(null)
              }, 3000).unref()
            }),
      ),
    )
    await rm(dir, { recursive: true, force: true })
  }
})
