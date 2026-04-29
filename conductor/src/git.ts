import { spawn } from 'node:child_process'

export interface GitResult {
  ok: boolean
  stdout: string
  stderr: string
  code: number | null
}

export function runCmd(cwd: string, bin: string, args: string[], timeoutMs?: number): Promise<GitResult> {
  return new Promise((resolve) => {
    const child = spawn(bin, args, { cwd, stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = ''
    let stderr = ''
    let timer: NodeJS.Timeout | undefined
    if (timeoutMs && timeoutMs > 0) {
      timer = setTimeout(() => child.kill('SIGKILL'), timeoutMs)
    }
    child.stdout.on('data', (b) => {
      stdout += b.toString()
    })
    child.stderr.on('data', (b) => {
      stderr += b.toString()
    })
    child.on('close', (code) => {
      if (timer) clearTimeout(timer)
      resolve({ ok: code === 0, stdout, stderr, code })
    })
    child.on('error', (err) => {
      if (timer) clearTimeout(timer)
      resolve({ ok: false, stdout, stderr: stderr + String(err), code: null })
    })
  })
}

export const git = (cwd: string, args: string[]) => runCmd(cwd, 'git', args)

export async function createWorktree(
  repoCwd: string,
  branch: string,
  root: string,
): Promise<{ ok: boolean; path?: string; reason?: string }> {
  const path = `${root}/${branch.replace(/[^a-zA-Z0-9_-]/g, '-')}`
  const r = await git(repoCwd, ['worktree', 'add', '-b', branch, path])
  if (!r.ok) return { ok: false, reason: r.stderr.trim() }
  return { ok: true, path }
}

export async function removeWorktree(repoCwd: string, path: string): Promise<{ ok: boolean; reason?: string }> {
  const r = await git(repoCwd, ['worktree', 'remove', '--force', path])
  if (!r.ok) return { ok: false, reason: r.stderr.trim() }
  return { ok: true }
}

export async function diffAgainst(repoCwd: string, baseRef: string): Promise<string> {
  const r = await git(repoCwd, ['diff', baseRef, '--', '.'])
  return r.stdout
}

export async function currentBranch(repoCwd: string): Promise<string> {
  const r = await git(repoCwd, ['rev-parse', '--abbrev-ref', 'HEAD'])
  return r.stdout.trim()
}
