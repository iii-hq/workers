import { type ChildProcess, spawn } from 'node:child_process'
import { constants } from 'node:fs'
import { access, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { connect, createServer } from 'node:net'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { test as base, expect } from '@playwright/test'

export interface ShellStack {
  consoleUrl: string
  engineUrl: string
  root: string
  createTmuxSocket(): string
}

interface ShellFixtures {
  shellStack: ShellStack
}

interface ProcessLog {
  label: string
  text: string
}

const MAX_PROCESS_LOG_BYTES = 256 * 1024

function required(name: string): string {
  const value = process.env[name]
  if (!value) throw new Error(`${name} is required for shell terminal E2E`)
  return value
}

async function reservePort(): Promise<number> {
  const server = createServer()
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
  const address = server.address()
  if (!address || typeof address === 'string') {
    server.close()
    throw new Error('failed to reserve loopback port')
  }
  const port = address.port
  await new Promise<void>((resolve, reject) =>
    server.close((error) => (error ? reject(error) : resolve())),
  )
  return port
}

function childExit(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null)
    return Promise.resolve()
  return new Promise((resolve) => child.once('exit', () => resolve()))
}

async function waitForTcp(
  port: number,
  children: ChildProcess[],
  timeoutMs = 30_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const exited = children.find(
      (child) => child.exitCode !== null || child.signalCode !== null,
    )
    if (exited)
      throw new Error('isolated shell stack process exited before ready')
    const ready = await new Promise<boolean>((resolve) => {
      const socket = connect({ host: '127.0.0.1', port })
      socket.once('connect', () => {
        socket.destroy()
        resolve(true)
      })
      socket.once('error', () => {
        socket.destroy()
        resolve(false)
      })
    })
    if (ready) return
    await delay(100)
  }
  throw new Error(`timed out waiting for TCP 127.0.0.1:${port}`)
}

async function waitForHttp(
  url: string,
  children: ChildProcess[],
  timeoutMs = 30_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const exited = children.find(
      (child) => child.exitCode !== null || child.signalCode !== null,
    )
    if (exited)
      throw new Error('isolated shell stack process exited before ready')
    try {
      const response = await fetch(url)
      if (response.ok) return
    } catch {
      await delay(100)
      continue
    }
    await delay(100)
  }
  throw new Error(`timed out waiting for ${url}`)
}

async function stopChildren(children: ChildProcess[]): Promise<void> {
  for (const child of [...children].reverse()) {
    if (child.exitCode === null && child.signalCode === null)
      child.kill('SIGTERM')
  }
  await Promise.all(
    children.map(async (child) => {
      if (child.exitCode !== null || child.signalCode !== null) return
      const exited = childExit(child)
      const graceful = await Promise.race([
        exited.then(() => true),
        delay(5_000).then(() => false),
      ])
      if (!graceful && child.exitCode === null && child.signalCode === null) {
        child.kill('SIGKILL')
        await exited
      }
    }),
  )
}

async function killTmuxServer(socket: string, tmuxRoot: string): Promise<void> {
  const child = spawn('tmux', ['-L', socket, 'kill-server'], {
    env: { ...process.env, TMUX_TMPDIR: tmuxRoot },
    stdio: 'ignore',
  })
  await childExit(child)
}

export const test = base.extend<ShellFixtures>({
  shellStack: async ({ browserName: _browserName }, use, testInfo) => {
    const root = await mkdtemp(path.join(tmpdir(), 'shell-terminal-e2e-'))
    const runRoot = path.join(root, 'workspace')
    const tmuxRoot = await mkdtemp('/tmp/t8-tmux-')
    const zshRoot = path.join(root, 'zsh')
    const systemZshPath = '/bin/zsh'
    const zshBinRoot = path.join(root, 'bin')
    const zshPath = path.join(zshBinRoot, 'zsh')
    await access(systemZshPath, constants.X_OK)
    await mkdir(runRoot, { recursive: true })
    await mkdir(zshRoot, { recursive: true })
    await mkdir(zshBinRoot, { recursive: true })
    await writeFile(zshPath, `#!/bin/sh\nexec ${systemZshPath} -d "$@"\n`, {
      mode: 0o755,
    })
    await writeFile(path.join(zshRoot, '.zshrc'), "PROMPT='iii-e2e% '\n")
    const enginePort = await reservePort()
    const consolePort = await reservePort()
    const engineConfig = path.join(root, 'engine.yaml')
    const consoleConfig = path.join(root, 'console.yaml')
    const shellConfig = path.join(root, 'shell.yaml')
    await writeFile(
      engineConfig,
      `workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: ${enginePort}\n  - name: iii-observability\n`,
    )
    await writeFile(consoleConfig, `http_port: ${consolePort}\n`)
    await writeFile(
      shellConfig,
      `working_dir: ${JSON.stringify(runRoot)}\nenv:\n  inherit: true\nfs:\n  host_roots:\n    - ${JSON.stringify(runRoot)}\n  allow_unjailed: false\n`,
    )

    const engineUrl = `ws://127.0.0.1:${enginePort}`
    const consoleUrl = `http://127.0.0.1:${consolePort}`
    const children: ChildProcess[] = []
    const logs: ProcessLog[] = []
    const tmuxSockets = new Set<string>()
    const spawnChild = (
      label: string,
      command: string,
      args: string[],
      env: NodeJS.ProcessEnv = process.env,
    ) => {
      const log = { label, text: '' }
      const child = spawn(command, args, {
        cwd: runRoot,
        env,
        stdio: ['ignore', 'pipe', 'pipe'],
      })
      const capture = (chunk: Buffer | string) => {
        log.text = `${log.text}${chunk.toString()}`.slice(
          -MAX_PROCESS_LOG_BYTES,
        )
      }
      child.stdout?.on('data', capture)
      child.stderr?.on('data', capture)
      children.push(child)
      logs.push(log)
      return child
    }

    try {
      spawnChild('engine', required('III_BIN'), [
        '--no-update-check',
        '-c',
        engineConfig,
      ])
      await waitForTcp(enginePort, children)
      spawnChild(
        'console',
        required('CONSOLE_BIN'),
        ['--config', consoleConfig, '--http-port', String(consolePort)],
        { ...process.env, III_URL: engineUrl },
      )
      await waitForHttp(consoleUrl, children)
      const shellEnv: NodeJS.ProcessEnv = {
        ...process.env,
        III_URL: engineUrl,
        SHELL: zshPath,
        TMUX_TMPDIR: tmuxRoot,
        ZDOTDIR: zshRoot,
      }
      delete shellEnv.III_SHELL_UI_WATCH
      delete shellEnv.TMUX
      delete shellEnv.TMUX_PANE
      spawnChild(
        'shell',
        required('SHELL_BIN'),
        ['--config', shellConfig],
        shellEnv,
      )
      await waitForHttp(`${consoleUrl}/ui/shell/page.js`, children)

      await use({
        consoleUrl,
        engineUrl,
        root: runRoot,
        createTmuxSocket() {
          const socket = `t8-${Math.random().toString(36).slice(2, 8)}`
          tmuxSockets.add(socket)
          return socket
        },
      })
    } finally {
      await stopChildren(children)
      for (const socket of tmuxSockets) {
        await killTmuxServer(socket, tmuxRoot)
      }
      if (testInfo.status !== testInfo.expectedStatus) {
        for (const log of logs) {
          await testInfo.attach(`${log.label}-log`, {
            body: log.text,
            contentType: 'text/plain',
          })
        }
      }
      await rm(root, { recursive: true, force: true })
      await rm(tmuxRoot, { recursive: true, force: true })
    }
  },
})

export { expect }
