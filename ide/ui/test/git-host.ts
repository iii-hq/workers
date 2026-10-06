/* A host that answers the page's `shell::exec`, `shell::workspace::validate`
   and `coder::read-file` with the local git and file system, for tests
   against real repositories. It lives outside `src`: it drives git through
   node APIs, and the page's tsconfig (DOM only) type-checks `src`. */

import { spawnSync } from 'node:child_process'
import { existsSync, readFileSync, realpathSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import type { Host } from '@iii-dev/console-ui'

// No inherited GIT_* variables and no system or global config; tests point
// HOME and XDG_CONFIG_HOME (git's default ignore and attributes files) into
// their own folder.
export const env: Record<string, string | undefined> = {
  ...Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('GIT_'))),
  GIT_CONFIG_GLOBAL: '/dev/null',
  GIT_CONFIG_NOSYSTEM: '1',
  GIT_AUTHOR_NAME: 't',
  GIT_AUTHOR_EMAIL: 't@example.com',
  GIT_COMMITTER_NAME: 't',
  GIT_COMMITTER_EMAIL: 't@example.com',
  LC_ALL: 'C',
}

export function sh(cwd: string, ...args: string[]): string {
  const out = spawnSync('git', args, { cwd, env, encoding: 'utf8' })
  if (out.status !== 0) throw new Error(`git ${args.join(' ')}: ${out.stderr}`)
  return out.stdout.trim()
}

export function commit(cwd: string, file: string, content: string, message: string) {
  writeFileSync(join(cwd, file), content)
  sh(cwd, 'add', file)
  sh(cwd, 'commit', '-q', '-m', message)
}

let afterGit: ((args: string[], cwd: string) => void) | null = null

/** Runs after each git the page starts, to change things between its steps. */
export function setAfterGit(hook: ((args: string[], cwd: string) => void) | null) {
  afterGit = hook
}

interface Payload {
  command?: string
  args?: string[]
  cwd?: string
  path?: string
  paths?: string[]
  stdin?: string
  env?: Record<string, string>
}

export const host = {
  iii: {
    trigger: async (fn: string, payload: Payload) => {
      if (fn === 'coder::read-file') {
        return {
          results: (payload.paths ?? []).map((path) =>
            existsSync(path) ? { path, success: true, content: readFileSync(path, 'utf8') } : { path, success: false },
          ),
        }
      }
      if (fn === 'shell::workspace::validate') {
        if (!payload.path || !existsSync(payload.path)) throw new Error(`no such folder: ${payload.path}`)
        return { path: realpathSync(payload.path) }
      }
      const args = payload.args ?? []
      const out = spawnSync(payload.command ?? '', args, {
        cwd: payload.cwd,
        env: { ...env, ...payload.env },
        encoding: 'utf8',
        input: payload.stdin,
      })
      afterGit?.(args, payload.cwd ?? '')
      return {
        exit_code: out.status,
        stdout: out.stdout,
        stderr: out.stderr,
        timed_out: false,
        stdout_truncated: false,
        stderr_truncated: false,
      }
    },
  },
} as unknown as Host
