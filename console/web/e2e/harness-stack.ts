import { type ChildProcessWithoutNullStreams, spawn } from 'node:child_process'
import { mkdir, mkdtemp, readFile, rm, watch } from 'node:fs/promises'
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
  driver: 'direct' | 'console'
  run_root: string
  result_path: string
  console_url: string
  engine_url: string
  session: { id: string; title: string }
  model: { id: string; provider: string }
  message: string
  functions: Record<string, string>
  send?: Record<string, unknown>
}

interface InvariantResult {
  id: string
  passed: boolean
  expected: unknown
  actual: unknown
  evidence_refs: string[]
}

export interface ServeResult {
  schema_version: '1'
  scenario_id: string
  classification:
    | 'pass'
    | 'setup_error'
    | 'contract_failure'
    | 'timeout'
    | 'process_crash'
    | 'runner_error'
  invariants: InvariantResult[]
  skipped_invariants: string[]
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
  trigger<T>(functionId: string, payload: unknown): Promise<T>
  waitForTurnCompleted(): Promise<TurnCompletedEvent>
  finish(): Promise<ServeResult>
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

function childExit(
  child: ChildProcessWithoutNullStreams,
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
    const args = [
      'serve',
      '--scenario',
      scenario,
      '--engine-bin',
      required('III_BIN'),
      '--harness-bin',
      required('HARNESS_BIN'),
      '--console-bin',
      required('CONSOLE_BIN'),
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
    let finalized: Promise<ServeResult> | undefined
    const finish = (): Promise<ServeResult> => {
      if (finalized) return finalized
      finalized = (async () => {
        if (sdk) await sdk.shutdown().catch(() => undefined)
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
        const result = JSON.parse(
          await readFile(ready.result_path, 'utf8'),
        ) as ServeResult
        await testInfo.attach('serve-result', {
          body: JSON.stringify(result, null, 2),
          contentType: 'application/json',
        })
        return result
      })()
      return finalized
    }

    let ready!: ReadyManifest
    try {
      ready = await waitForReady(readyFile, exit)
    } catch (error) {
      if (child.exitCode === null && child.signalCode === null) {
        child.kill('SIGTERM')
      }
      await exit.catch(() => undefined)
      await testInfo.attach('harness-integration.stdout', {
        body: Buffer.concat(stdout),
        contentType: 'text/plain',
      })
      await testInfo.attach('harness-integration.stderr', {
        body: Buffer.concat(stderr),
        contentType: 'text/plain',
      })
      throw error
    }
    const connectedSdk = registerWorker(ready.engine_url)
    sdk = connectedSdk
    const stack: HarnessStack = {
      ready,
      trigger: <T>(functionId: string, payload: unknown) =>
        connectedSdk.trigger<unknown, T>({
          function_id: functionId,
          payload,
          timeoutMs: 30_000,
        }),
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
  ready: ReadyManifest,
): Promise<void> {
  await page.goto(ready.console_url)
  const session = page.getByRole('button', {
    name: `open ${ready.session.title}`,
    exact: true,
  })
  await session.click()
  await expect(session).toHaveAttribute('aria-current', 'page')
}

export function expectPassingResult(result: ServeResult): void {
  expect(result.classification).toBe('pass')
  expect(result.skipped_invariants).toEqual(['send.flags'])
  expect(result.invariants.every((invariant) => invariant.passed)).toBe(true)
}
