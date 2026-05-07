// Narrow Vercel REST client. Holds the base URL + auth token and a small
// helper for building requests. Endpoint paths and bodies are still stubbed
// pending a verified pass against the live Vercel Sandbox REST surface;
// every call below returns SandboxError(S502) with a TODO marker.

import { type CreatedSandbox, type ExecResult, S, SandboxError, type SandboxRecord } from './sandbox.js'

export interface VercelAuth {
  token: string
  team_id?: string
  project_id?: string
}

export class VercelClient {
  api_base: string
  auth: VercelAuth

  constructor(api_base: string, auth: VercelAuth) {
    this.api_base = api_base.replace(/\/$/, '')
    this.auth = auth
  }

  // headers() will be re-introduced when the real REST surface lands. For
  // now the stub paths short-circuit before any HTTP request is built, so
  // we omit it to keep the lint surface clean.
  async create(_image: string, _idle_timeout_secs: number, _runtime: string): Promise<CreatedSandbox> {
    // TODO: POST /v1/sandboxes (or whichever Vercel ships); parse response;
    // map non-2xx via mapHttpStatus.
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: wire Vercel Sandbox create')
  }

  async exec(_sandbox_id: string, _cmd: string, _args: string[], _timeout_ms?: number): Promise<ExecResult> {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: wire Vercel runCommand')
  }

  async stop(_sandbox_id: string): Promise<void> {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: wire Vercel sandbox.stop')
  }

  async list(): Promise<SandboxRecord[]> {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: wire Vercel list')
  }

  async snapshot(_sandbox_id: string): Promise<string> {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: wire Vercel snapshot')
  }

  async exposePort(_sandbox_id: string, _port: number): Promise<string> {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: wire Vercel sandbox.domain(port)')
  }

  async fsRead(_sandbox_id: string, _path: string): Promise<Uint8Array> {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: wire Vercel fs read')
  }

  async fsWrite(_sandbox_id: string, _path: string, _bytes: Uint8Array, _mode?: number): Promise<void> {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: wire Vercel fs write')
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
