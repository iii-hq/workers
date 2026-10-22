import { type ChildProcess, spawn } from 'node:child_process'
import { createWriteStream } from 'node:fs'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import net from 'node:net'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { test as base, expect, type Page } from '@playwright/test'
import { type ISdk, registerWorker } from 'iii-browser-sdk'
import type { ContextTrackingItem } from '../src/components/chat/ContextTracking'

type Results = {
  'engine::functions::info': {
    request_schema: Record<string, unknown>
    response_schema: Record<string, unknown>
  }
  'console::status': unknown
  'session::list': unknown
  'session::create': { session_id: string }
  'session::append': unknown
  'session::set-meta': unknown
  'console::context::save': ContextTrackingItem
  'console::context::list': { items: ContextTrackingItem[] }
  'console::context::remove': { removed: boolean }
  'console::context::inject-guidance': { mutations: { system_prompt?: string } }
}
export const snapshot = {
  title: 'Release adversarial',
  summary: 'Verified initial snapshot',
  progress: 20,
  items: [{ label: 'First task', done: false }],
  source: { label: 'Source board', url: 'https://example.com/release' },
}
export interface Stack {
  dir: string
  base: string
  project: string
  otherProject: string
  file: string
  call<K extends keyof Results>(
    id: K,
    payload?: Record<string, unknown>,
  ): Promise<Results[K]>
  create(title: string, root?: string): Promise<string>
  open(page: Page, title: string): Promise<void>
  disk(): Promise<ContextTrackingItem[]>
}
async function freePort() {
  return new Promise<number>((resolve, reject) => {
    const server = net.createServer()
    server.on('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const p = (server.address() as net.AddressInfo).port
      server.close(() => resolve(p))
    })
  })
}
export const test = base.extend<{ stack: Stack }>({
  // biome-ignore lint/correctness/noEmptyPattern: Playwright requires destructured fixture dependencies; this fixture needs none.
  stack: async ({}, use, info) => {
    const engine = process.env.CONTEXT_ENGINE_BIN
    const session = process.env.CONTEXT_SESSION_BIN
    const consoleBin = process.env.CONTEXT_CONSOLE_BIN
    if (!engine || !session || !consoleBin)
      throw new Error(
        'Real adversarial tests require CONTEXT_{ENGINE,SESSION,CONSOLE}_BIN; refusing to use mocks or silently skip',
      )
    const dir = await mkdtemp(path.join(tmpdir(), 'context-adversarial-'))
    const enginePort = await freePort()
    const httpPort = await freePort()
    const url = `ws://127.0.0.1:${enginePort}`
    const baseUrl = `http://127.0.0.1:${httpPort}`
    const processes: ChildProcess[] = []
    const streams: ReturnType<typeof createWriteStream>[] = []
    let sdk: ISdk | undefined
    const start = (name: string, binary: string, args: string[]) => {
      const log = createWriteStream(info.outputPath(`${name}.log`))
      streams.push(log)
      const child = spawn(binary, args, {
        cwd: dir,
        env: {
          ...process.env,
          III_COMPOSE_DIR: dir,
          III_COMPOSE_NAMESPACE: 'default',
          III_CONFIG_NAME: '',
          III_URL: url,
          III_CONSOLE_ONBOARDING_AUTO_OPEN: 'false',
        },
        stdio: ['ignore', 'pipe', 'pipe'],
      })
      child.stdout?.pipe(log, { end: false })
      child.stderr?.pipe(log, { end: false })
      processes.push(child)
    }
    try {
      await mkdir(info.outputDir, { recursive: true })
      const project = path.join(dir, 'project')
      const otherProject = path.join(dir, 'other-project')
      await mkdir(project)
      await mkdir(otherProject)
      await writeFile(
        path.join(dir, 'engine.yaml'),
        `workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: ${enginePort}\n  - name: configuration\n    config:\n      adapter:\n        name: fs\n        config:\n          directory: ${dir}/configuration\n`,
      )
      await writeFile(
        path.join(dir, 'session.yaml'),
        `adapter:\n  name: fs\n  config:\n    data_dir: ${dir}/sessions\n`,
      )
      await writeFile(
        path.join(dir, 'console.yaml'),
        `http_port: ${httpPort}\nhttp_host: 127.0.0.1\ndata_dir: ${dir}/console\n`,
      )
      start('engine', engine, [
        '--config',
        path.join(dir, 'engine.yaml'),
        '--no-update-check',
      ])
      await expect
        .poll(
          () =>
            new Promise<boolean>((resolve) => {
              const s = net.connect(enginePort, '127.0.0.1')
              s.once('connect', () => {
                s.destroy()
                resolve(true)
              })
              s.once('error', () => resolve(false))
            }),
        )
        .toBe(true)
      const client = registerWorker(url)
      sdk = client
      const call = <K extends keyof Results>(
        id: K,
        payload: Record<string, unknown> = {},
      ): Promise<Results[K]> =>
        client.trigger({
          function_id: id,
          payload,
          timeoutMs: 5000,
        }) as Promise<Results[K]>
      start('session', session, [
        '--url',
        url,
        '--config',
        path.join(dir, 'session.yaml'),
      ])
      start('console', consoleBin, [
        '--url',
        url,
        '--config',
        path.join(dir, 'console.yaml'),
      ])
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
      const file = path.join(dir, 'console', 'context-tracking.json')
      await use({
        dir,
        base: baseUrl,
        project,
        otherProject,
        file,
        call,
        create: async (title, root = project) => {
          const { session_id } = await call('session::create', {
            title,
            metadata: { surface: 'console', fs_scope: { root } },
          })
          await call('session::append', {
            session_id,
            message: {
              role: 'user',
              content: [{ type: 'text', text: 'Track this release.' }],
              timestamp: Date.now(),
            },
          })
          return session_id
        },
        open: async (page, title) => {
          await page.setViewportSize({ width: 1440, height: 1000 })
          await page.goto(baseUrl)
          await page
            .getByRole('button', { name: `open ${title}`, exact: true })
            .getByText(title, { exact: true })
            .click()
          const section = page.getByRole('region', {
            name: 'Tracking',
            exact: true,
          })
          if (!(await section.isVisible()))
            await page
              .getByRole('button', { name: /^(Context|Side panel)$/i })
              .first()
              .click()
          await expect(section).toBeVisible()
        },
        disk: async () => JSON.parse(await readFile(file, 'utf8')),
      })
    } finally {
      await sdk?.shutdown().catch(() => {})
      for (const child of processes.reverse())
        if (child.exitCode === null) child.kill('SIGTERM')
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
      for (const stream of streams) stream.end()
      await rm(dir, { recursive: true, force: true })
    }
  },
})
export { expect }
