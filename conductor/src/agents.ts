import { runCmd } from './git.js'
import type { AgentKind, AgentSpec } from './types.js'

const DEFAULT_BIN: Record<Exclude<AgentKind, 'remote'>, string> = {
  claude: 'claude',
  codex: 'codex',
  gemini: 'gemini',
  aider: 'aider',
  cursor: 'cursor-agent',
  amp: 'amp',
  opencode: 'opencode',
  qwen: 'qwen',
}

export interface AgentRun {
  ok: boolean
  exitCode: number | null
  stdout: string
  stderr: string
  reason?: string
}

function buildArgs(spec: AgentSpec): string[] {
  if (spec.args && spec.args.length > 0) return spec.args
  if (spec.kind === 'claude') {
    const args = ['--print']
    if (spec.worktree) args.push('--worktree')
    if (spec.prompt) args.push(spec.prompt)
    return args
  }
  if (spec.kind === 'codex') {
    return spec.prompt ? ['exec', spec.prompt] : ['exec']
  }
  if (spec.kind === 'gemini') {
    return spec.prompt ? ['--prompt', spec.prompt] : []
  }
  return spec.prompt ? [spec.prompt] : []
}

export async function runLocalAgent(spec: AgentSpec, cwd: string, timeoutMs?: number): Promise<AgentRun> {
  if (spec.kind === 'remote') {
    return { ok: false, exitCode: null, stdout: '', stderr: '', reason: 'use trigger for remote agent' }
  }
  const bin = spec.bin ?? DEFAULT_BIN[spec.kind]
  if (!bin) return { ok: false, exitCode: null, stdout: '', stderr: '', reason: `no binary for ${spec.kind}` }
  const args = buildArgs(spec)
  const r = await runCmd(cwd, bin, args, timeoutMs)
  return { ok: r.ok, exitCode: r.code, stdout: r.stdout, stderr: r.stderr, reason: r.ok ? undefined : r.stderr.trim() }
}
