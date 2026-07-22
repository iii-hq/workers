import { type ChildProcess, spawn } from 'node:child_process'
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
  driver: 'direct' | 'playground'
  run_root: string
  result_path: string
  engine_url: string
  console_url: string
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

export interface PlaygroundResult {
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
  trigger(): Promise<unknown>
  waitForTurnCompleted(): Promise<TurnCompletedEvent>
  finish(): Promise<PlaygroundResult>
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

function childExit(
  child: ChildProcess,
): Promise<{ code: number | null; signal: NodeJS.Signals | null }> {
  return new Promise((resolve) => {
    child.once('exit', (code, signal) => resolve({ code, signal }))
  })
}

async function waitForReady(
  readyFile: string,
  exit: Promise<{ code: number | null; signal: NodeJS.Signals | null }>,
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

  const changes = watch(path.dirname(readyFile))
  const timeout = delay(70_000).then(() => {
    throw new Error(`timed out waiting for ${readyFile}`)
  })
  const exited = exit.then(({ code, signal }) => {
    throw new Error(
      `harness-integration exited before ready (code=${String(code)}, signal=${String(signal)})`,
    )
  })
  const appeared = (async () => {
    for await (const event of changes) {
      if (event.filename && event.filename !== path.basename(readyFile))
        continue
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
      functionRef?.unregister()
    } catch {
      // The isolated stack may already be shutting down.
    }
  }
  return new Promise<TurnCompletedEvent>((resolve, reject) => {
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
      'playground',
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
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    const exit = childExit(child)
    const stdout: Buffer[] = []
    const stderr: Buffer[] = []
    child.stdout.on('data', (chunk: Buffer) => stdout.push(chunk))
    child.stderr.on('data', (chunk: Buffer) => stderr.push(chunk))

    let sdk: ISdk | undefined
    let ready: ReadyManifest | undefined
    let finalized: Promise<PlaygroundResult> | undefined
    const attachLogs = async () => {
      await testInfo.attach('harness-integration.stdout', {
        body: Buffer.concat(stdout),
        contentType: 'text/plain',
      })
      await testInfo.attach('harness-integration.stderr', {
        body: Buffer.concat(stderr),
        contentType: 'text/plain',
      })
    }
    const finish = (): Promise<PlaygroundResult> => {
      if (finalized) return finalized
      finalized = (async () => {
        if (!ready)
          throw new Error('playground did not publish a ready manifest')
        if (sdk) await sdk.shutdown().catch(() => undefined)
        if (child.exitCode === null && child.signalCode === null) {
          child.kill('SIGTERM')
        }
        let exited = await Promise.race([exit, delay(30_000).then(() => null)])
        if (!exited) {
          child.kill('SIGKILL')
          exited = await exit
        }
        await attachLogs()
        const result = JSON.parse(
          await readFile(ready.result_path, 'utf8'),
        ) as PlaygroundResult
        await testInfo.attach('playground-result', {
          body: JSON.stringify(result, null, 2),
          contentType: 'application/json',
        })
        return result
      })()
      return finalized
    }

    try {
      ready = await waitForReady(readyFile, exit)
      const manifest = ready
      const connectedSdk = registerWorker(manifest.engine_url)
      sdk = connectedSdk
      const stack: HarnessStack = {
        ready: manifest,
        consoleUrl: manifest.console_url,
        trigger: () =>
          connectedSdk.trigger({
            function_id: 'harness::send',
            payload: manifest.send,
          }),
        waitForTurnCompleted: () => armCompletion(connectedSdk, manifest),
        finish,
      }
      await use(stack)
    } catch (error) {
      if (child.exitCode === null && child.signalCode === null) {
        child.kill('SIGTERM')
      }
      await exit.catch(() => undefined)
      await attachLogs()
      throw error
    } finally {
      if (!finalized && ready) await finish().catch(() => undefined)
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
  // Let the Console settle its initial local-draft selection before changing
  // sessions; otherwise that bootstrap effect can overwrite this click.
  await expect(
    page.locator('[role="button"][aria-current="page"]'),
  ).toHaveCount(1)
  const session = page.getByRole('button', {
    name: `open ${stack.ready.session.title}`,
    exact: true,
  })
  await session.click()
  await expect(session).toHaveAttribute('aria-current', 'page')
  await expect(
    page.locator(`[data-chat-session-id="${stack.ready.session.id}"]`),
  ).toHaveAttribute('data-chat-session-hydrated', 'true')
}

export function expectPassingResult(result: PlaygroundResult): void {
  expect(result.classification).toBe('pass')
}
