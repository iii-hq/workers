import { type ChildProcess, spawn } from 'node:child_process'
import { createServer } from 'node:net'
import { mkdir, mkdtemp, readFile, rename, rm, writeFile, watch } from 'node:fs/promises'
import path from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import type { Page } from '@playwright/test'
import { test as base, expect } from '@playwright/test'
import { type ISdk, registerWorker } from 'iii-browser-sdk'

interface ReadyManifest {
  schema_version: '1'
  run_id: string
  scenario_id: string
  scenario_slug: string
  driver: 'direct' | 'observe'
  run_root: string
  result_path: string
  engine_url: string
  session: { id: string; title: string }
  model: { id: string; provider: string }
  message: string
  functions: Record<string, string>
  send: Record<string, unknown>
}

export interface RecorderEvent {
  schema_version: '1'
  run_id: string
  sequence: number
  kind: 'target_call' | 'lifecycle'
  function_id: string
  payload: unknown
  received_at: string
}

/** Raw serialized RunEvidence: real ids, checkable against ReadyManifest. */
export interface RunEvidence {
  run_id: string
  session_id: string
  turn_id: string | null
  send_response: unknown
  status: unknown
  transcript: unknown[]
  generations_consumed: number
  generations_total: number
  recorder_events: RecorderEvent[]
}

export interface ObserveResult {
  schema_version: '1'
  scenario_id: string
  classification:
    | 'pass'
    | 'setup_error'
    | 'contract_failure'
    | 'timeout'
    | 'process_crash'
    | 'runner_error'
  failure: string | null
  evidence: RunEvidence | null
  artifacts: string[]
}

interface TurnCompletedEvent {
  session_id: string
  turn_id: string
  status: 'completed' | 'cancelled' | 'failed'
  timestamp: number
}

export interface HarnessStack {
  ready: ReadyManifest
  consoleUrl: string
  start(): Promise<void>
  waitForTurnCompleted(): Promise<TurnCompletedEvent>
  finish(): Promise<ObserveResult>
}

interface FixtureOptions {
  scenario: string
}

interface FixtureValues {
  stack: HarnessStack
}

function required(name: string): string {
  const value = process.env[name]
  if (!value) throw new Error(`${name} is required for Console E2E`)
  return value
}

function workerArgs(): string[] {
  return [
    ['queue', 'QUEUE_BIN'],
    ['iii-directory', 'III_DIRECTORY_BIN'],
    ['session-manager', 'SESSION_MANAGER_BIN'],
    ['context-manager', 'CONTEXT_MANAGER_BIN'],
  ].flatMap(([name, env]) => ['--worker-bin', `${name}=${required(env)}`])
}

async function freeLoopbackPort(): Promise<number> {
  return await new Promise((resolve, reject) => {
    const server = createServer()
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      if (!address || typeof address === 'string') {
        server.close()
        reject(new Error('failed to allocate loopback port'))
        return
      }
      const { port } = address
      server.close((error) => {
        if (error) reject(error)
        else resolve(port)
      })
    })
    server.on('error', reject)
  })
}

async function writeAtomicJson(filePath: string, value: unknown): Promise<void> {
  const parent = path.dirname(filePath)
  const temporary = path.join(
    parent,
    `.${path.basename(filePath)}.${process.pid}.tmp`,
  )
  await writeFile(temporary, `${JSON.stringify(value, null, 2)}\n`, 'utf8')
  await rename(temporary, filePath)
}

async function waitForReady(
  readyFile: string,
  childExit: Promise<{ code: number | null; signal: NodeJS.Signals | null }>,
): Promise<ReadyManifest> {
  const read = async (): Promise<ReadyManifest | null> => {
    try {
      return JSON.parse(await readFile(readyFile, 'utf8')) as ReadyManifest
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code
      if (code === 'ENOENT' || error instanceof SyntaxError) return null
      throw error
    }
  }
  const existing = await read()
  if (existing) return existing

  const parent = path.dirname(readyFile)
  const expectedName = path.basename(readyFile)
  const changes = watch(parent)
  const timeout = delay(70_000).then(() => {
    throw new Error(`timed out waiting for ${readyFile}`)
  })
  const exited = childExit.then(({ code, signal }) => {
    throw new Error(
      `harness-integration exited before ready (code=${String(code)}, signal=${String(signal)})`,
    )
  })
  const appeared = (async () => {
    for await (const event of changes) {
      if (event.filename && event.filename !== expectedName) continue
      const manifest = await read()
      if (manifest) return manifest
    }
    throw new Error(`ready-file watcher closed before ${readyFile} appeared`)
  })()
  try {
    return await Promise.race([appeared, exited, timeout])
  } finally {
    await changes.return?.()
  }
}

async function waitForConsoleHttp(port: number): Promise<void> {
  const deadline = Date.now() + 60_000
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`http://127.0.0.1:${port}/`, {
        redirect: 'manual',
      })
      if (response.ok || (response.status >= 300 && response.status < 400)) {
        return
      }
    } catch {
      // Console still booting.
    }
    await delay(100)
  }
  throw new Error(`console HTTP did not become ready on port ${port}`)
}

function childExit(
  child: ChildProcess,
): Promise<{ code: number | null; signal: NodeJS.Signals | null }> {
  return new Promise((resolve) => {
    child.once('exit', (code, signal) => resolve({ code, signal }))
  })
}

function armCompletion(
  sdk: ISdk,
  ready: ReadyManifest,
): Promise<TurnCompletedEvent> {
  const functionId = `console-e2e::turn-completed::${ready.run_id}`
  let functionRef: ReturnType<ISdk['registerFunction']> | undefined
  let triggerRef: ReturnType<ISdk['registerTrigger']> | undefined
  let timer: NodeJS.Timeout | undefined
  const cleanup = () => {
    if (timer) clearTimeout(timer)
    try {
      triggerRef?.unregister()
    } catch {
      // The stack may already be shutting down.
    }
    try {
      functionRef?.unregister()
    } catch {
      // The stack may already be shutting down.
    }
  }
  const completed = new Promise<TurnCompletedEvent>((resolve, reject) => {
    functionRef = sdk.registerFunction(
      functionId,
      async (payload) => {
        const event = payload as TurnCompletedEvent
        if (event.session_id !== ready.session.id) return null
        cleanup()
        resolve(event)
        return null
      },
      { metadata: { internal: true } },
    )
    triggerRef = sdk.registerTrigger({
      type: 'harness::turn-completed',
      function_id: functionId,
      config: { session_id: ready.session.id },
    })
    timer = setTimeout(() => {
      cleanup()
      reject(new Error('harness::turn-completed was not delivered'))
    }, 60_000)
  })
  return completed
}

function stopChild(child: ChildProcess | undefined): void {
  if (!child) return
  if (child.exitCode !== null || child.signalCode !== null) return
  child.kill('SIGTERM')
}

export const test = base.extend<FixtureValues, FixtureOptions>({
  scenario: ['', { scope: 'worker', option: true }],
  stack: async ({ scenario }, use, testInfo) => {
    if (!scenario) throw new Error('test.use({ scenario }) is required')
    const artifactsRoot = path.resolve(
      process.env.CONSOLE_E2E_ARTIFACTS_DIR ??
        path.join(testInfo.project.outputDir, '..'),
    )
    await mkdir(artifactsRoot, { recursive: true })
    const controlDir = await mkdtemp(path.join(artifactsRoot, 'runner-'))
    const readyFile = path.join(controlDir, 'ready.json')
    const startFile = path.join(controlDir, 'start.json')
    const args = [
      'observe',
      '--scenario',
      scenario,
      '--engine-bin',
      required('III_BIN'),
      '--harness-bin',
      required('HARNESS_BIN'),
      '--artifacts-dir',
      artifactsRoot,
      '--ready-file',
      readyFile,
      ...workerArgs(),
    ]
    const child = spawn(required('HARNESS_INTEGRATION_BIN'), args, {
      stdio: ['pipe', 'pipe', 'pipe'],
    })
    const exit = childExit(child)
    const stdout: Buffer[] = []
    const stderr: Buffer[] = []
    child.stdout.on('data', (chunk: Buffer) => stdout.push(chunk))
    child.stderr.on('data', (chunk: Buffer) => stderr.push(chunk))

    let sdk: ISdk | undefined
    let consoleChild: ChildProcess | undefined
    let consoleExit: Promise<{
      code: number | null
      signal: NodeJS.Signals | null
    }> | undefined
    const consoleStdout: Buffer[] = []
    const consoleStderr: Buffer[] = []
    let finalized: Promise<ObserveResult> | undefined

    const finish = (): Promise<ObserveResult> => {
      if (finalized) return finalized
      finalized = (async () => {
        if (sdk) await sdk.shutdown().catch(() => undefined)
        stopChild(consoleChild)
        if (consoleExit) {
          let consoleExited = await Promise.race([
            consoleExit,
            delay(10_000).then(() => null),
          ])
          if (!consoleExited && consoleChild) {
            consoleChild.kill('SIGKILL')
            consoleExited = await consoleExit
          }
        }
        if (child.exitCode === null && child.signalCode === null) {
          child.kill('SIGTERM')
        }
        let exited = await Promise.race([exit, delay(30_000).then(() => null)])
        if (!exited) {
          child.kill('SIGKILL')
          exited = await exit
        }
        await testInfo.attach('harness-integration.stdout', {
          body: Buffer.concat(stdout),
          contentType: 'text/plain',
        })
        await testInfo.attach('harness-integration.stderr', {
          body: Buffer.concat(stderr),
          contentType: 'text/plain',
        })
        if (consoleStdout.length > 0 || consoleStderr.length > 0) {
          await testInfo.attach('console.stdout', {
            body: Buffer.concat(consoleStdout),
            contentType: 'text/plain',
          })
          await testInfo.attach('console.stderr', {
            body: Buffer.concat(consoleStderr),
            contentType: 'text/plain',
          })
        }
        const result = JSON.parse(
          await readFile(ready.result_path, 'utf8'),
        ) as ObserveResult
        await testInfo.attach('observe-result', {
          body: JSON.stringify(result, null, 2),
          contentType: 'application/json',
        })
        return result
      })()
      return finalized
    }

    let ready!: ReadyManifest
    let consoleUrl!: string
    try {
      ready = await waitForReady(readyFile, exit)
      const httpPort = await freeLoopbackPort()
      consoleUrl = `http://127.0.0.1:${httpPort}`
      const spawnedConsole = spawn(
        required('CONSOLE_BIN'),
        ['--url', ready.engine_url, '--http-port', String(httpPort)],
        { stdio: ['ignore', 'pipe', 'pipe'] },
      )
      consoleChild = spawnedConsole
      consoleExit = childExit(spawnedConsole)
      spawnedConsole.stdout.on('data', (chunk: Buffer) =>
        consoleStdout.push(chunk),
      )
      spawnedConsole.stderr.on('data', (chunk: Buffer) =>
        consoleStderr.push(chunk),
      )
      await Promise.race([
        waitForConsoleHttp(httpPort),
        consoleExit.then(({ code, signal }) => {
          throw new Error(
            `console exited before ready (code=${String(code)}, signal=${String(signal)})`,
          )
        }),
      ])
    } catch (error) {
      stopChild(consoleChild)
      if (child.exitCode === null && child.signalCode === null) {
        child.kill('SIGTERM')
      }
      await exit.catch(() => undefined)
      await consoleExit?.catch(() => undefined)
      await testInfo.attach('harness-integration.stdout', {
        body: Buffer.concat(stdout),
        contentType: 'text/plain',
      })
      await testInfo.attach('harness-integration.stderr', {
        body: Buffer.concat(stderr),
        contentType: 'text/plain',
      })
      if (consoleStdout.length > 0 || consoleStderr.length > 0) {
        await testInfo.attach('console.stdout', {
          body: Buffer.concat(consoleStdout),
          contentType: 'text/plain',
        })
        await testInfo.attach('console.stderr', {
          body: Buffer.concat(consoleStderr),
          contentType: 'text/plain',
        })
      }
      throw error
    }

    const connectedSdk = registerWorker(ready.engine_url)
    sdk = connectedSdk
    let started = false
    const stack: HarnessStack = {
      ready,
      consoleUrl,
      start: async () => {
        if (started) return
        started = true
        await writeAtomicJson(startFile, { schema_version: '1' })
      },
      waitForTurnCompleted: () => armCompletion(connectedSdk, ready),
      finish,
    }

    try {
      await use(stack)
    } finally {
      if (!finalized) {
        await finish().catch(() => undefined)
      }
      await rm(controlDir, { recursive: true, force: true })
    }
  },
})

export { expect }

export async function openSession(
  page: Page,
  stack: HarnessStack,
): Promise<void> {
  await page.goto(stack.consoleUrl)
  const session = page.getByRole('button', {
    name: `open ${stack.ready.session.title}`,
    exact: true,
  })
  await session.click()
  await expect(session).toHaveAttribute('aria-current', 'page')
}

export function expectPassingResult(result: ObserveResult): void {
  expect(result.classification).toBe('pass')
}
