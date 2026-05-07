// Narrow Vercel REST client matching the surface documented at
// vercel.com/docs/rest-api/sandboxes/*. Every call lands at
// `https://api.vercel.com/v1/sandboxes/...` and requires `?teamId=&slug=`
// query params plus a `Bearer` token. Snapshot/fs are not yet wired —
// the v2-beta sessions endpoint is the right surface for those and
// lands in a follow-up.

import { type CreatedSandbox, type ExecResult, mapHttpStatus, S, SandboxError, type SandboxRecord } from './sandbox.js'

export interface VercelAuth {
  token: string
  team_id?: string
  project_id?: string
}

interface CreateInput {
  image: string
  idle_timeout_secs: number
  runtime: string
  source?: { url: string; revision?: string; depth?: number; username?: string; password?: string }
  resources?: { vcpus?: number; memory?: number }
  ports?: number[]
  env?: Record<string, string>
}

export class VercelClient {
  api_base: string
  auth: VercelAuth

  constructor(api_base: string, auth: VercelAuth) {
    this.api_base = api_base.replace(/\/$/, '')
    this.auth = auth
  }

  private headers(): HeadersInit {
    return {
      Authorization: `Bearer ${this.auth.token}`,
      'Content-Type': 'application/json',
    }
  }

  private query(): string {
    const params = new URLSearchParams()
    if (this.auth.team_id) params.set('teamId', this.auth.team_id)
    const slug = process.env.VERCEL_TEAM_SLUG
    if (slug) params.set('slug', slug)
    const q = params.toString()
    return q ? `?${q}` : ''
  }

  private async readBody(resp: Response): Promise<string> {
    try {
      return await resp.text()
    } catch {
      return ''
    }
  }

  /**
   * Create a sandbox. POST /v1/sandboxes?teamId=&slug=
   * Body requires `source` (a git ref to clone) and `runtime`. `timeout`
   * is in milliseconds upstream — we convert from idle_timeout_secs.
   */
  async create(input: CreateInput): Promise<CreatedSandbox> {
    const body: Record<string, unknown> = {
      runtime: input.runtime,
      timeout: String(input.idle_timeout_secs * 1000),
    }
    if (input.source) body.source = { type: 'value', ...input.source }
    if (input.resources) body.resources = input.resources
    if (input.ports) body.ports = input.ports
    if (input.env) body.env = input.env
    if (this.auth.project_id) body.projectId = this.auth.project_id

    const resp = await fetch(`${this.api_base}/v1/sandboxes${this.query()}`, {
      method: 'POST',
      headers: this.headers(),
      body: JSON.stringify(body),
    })
    if (!resp.ok) throw mapHttpStatus(resp.status, await this.readBody(resp))
    const parsed = (await resp.json()) as { sandboxId?: string; id?: string; runtime?: string; createdAt?: string }
    const sandbox_id = parsed.sandboxId ?? parsed.id ?? ''
    if (!sandbox_id) {
      throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'vercel response missing sandboxId/id')
    }
    return {
      sandbox_id,
      image: input.image || parsed.runtime || input.runtime,
      started_at: Math.floor(Date.now() / 1000),
    }
  }

  /**
   * POST /v1/sandboxes/{id}/commands?teamId=&slug=
   * Vercel returns a Command object with stdout/stderr after completion;
   * for streaming output use the runtime SDK directly.
   */
  async exec(sandbox_id: string, cmd: string, args: string[], _timeout_ms?: number): Promise<ExecResult> {
    const resp = await fetch(`${this.api_base}/v1/sandboxes/${sandbox_id}/commands${this.query()}`, {
      method: 'POST',
      headers: this.headers(),
      body: JSON.stringify({ command: cmd, args, cwd: '/vercel/sandbox' }),
    })
    if (!resp.ok) throw mapHttpStatus(resp.status, await this.readBody(resp))
    const parsed = (await resp.json()) as {
      stdout?: string
      stderr?: string
      exitCode?: number
      timedOut?: boolean
    }
    return {
      stdout: parsed.stdout ?? '',
      stderr: parsed.stderr ?? '',
      exit_code: parsed.exitCode ?? 0,
      timed_out: parsed.timedOut ?? false,
    }
  }

  /**
   * Stop a sandbox. POST /v1/sandboxes/{id}/stop?teamId=&slug=
   * Vercel uses POST-stop, not DELETE, and surfaces 404 when the
   * sandbox is gone. 404 is treated as idempotent success.
   */
  async stop(sandbox_id: string): Promise<void> {
    const resp = await fetch(`${this.api_base}/v1/sandboxes/${sandbox_id}/stop${this.query()}`, {
      method: 'POST',
      headers: this.headers(),
    })
    if (resp.ok || resp.status === 404 || resp.status === 409) return
    throw mapHttpStatus(resp.status, await this.readBody(resp))
  }

  async list(): Promise<SandboxRecord[]> {
    const resp = await fetch(`${this.api_base}/v1/sandboxes${this.query()}`, {
      method: 'GET',
      headers: this.headers(),
    })
    if (!resp.ok) throw mapHttpStatus(resp.status, await this.readBody(resp))
    const parsed = (await resp.json()) as
      | { sandboxes?: Array<{ id?: string; sandboxId?: string; runtime?: string; createdAt?: string }> }
      | Array<{ id?: string; sandboxId?: string; runtime?: string; createdAt?: string }>
    const items = Array.isArray(parsed) ? parsed : (parsed.sandboxes ?? [])
    return items.map((it) => ({
      sandbox_id: it.sandboxId ?? it.id ?? '',
      image: it.runtime ?? '',
      started_at: it.createdAt ?? '',
    }))
  }

  async snapshot(_sandbox_id: string): Promise<string> {
    // Vercel's snapshot lives under v2-beta sessions; not yet exposed
    // in the canonical /v1 surface. Wire when the v2 endpoints
    // graduate or callers need it.
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: wire vercel snapshot via v2-beta sessions')
  }

  async exposePort(_sandbox_id: string, _port: number): Promise<string> {
    // The runtime SDK derives port URLs deterministically via
    // `sandbox.domain(port)`. Reproducing that requires the sandbox's
    // public domain which isn't exposed via REST today.
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: wire vercel sandbox.domain(port)')
  }

  async fsRead(_sandbox_id: string, _path: string): Promise<Uint8Array> {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: wire vercel v2-beta read-files')
  }

  async fsWrite(_sandbox_id: string, _path: string, _bytes: Uint8Array, _mode?: number): Promise<void> {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: wire vercel v2-beta write-files')
  }
}

export function authFromEnv(cfg: {
  oidc_token_env: string
  fallback_token_env: string
  team_id_env: string
  project_id_env: string
}): VercelAuth {
  const oidc = process.env[cfg.oidc_token_env]
  if (oidc) return { token: oidc }
  const token = process.env[cfg.fallback_token_env]
  if (!token) {
    throw new Error(`sandbox-vercel: neither ${cfg.oidc_token_env} nor ${cfg.fallback_token_env} is set`)
  }
  return {
    token,
    team_id: process.env[cfg.team_id_env],
    project_id: process.env[cfg.project_id_env],
  }
}
