// Narrow HTTP client for the Cloudflare Worker bridge deployed out of
// `bridge/`. Every method posts JSON to a route the bridge exposes. The
// bridge itself is the thing that calls `getSandbox(env.Sandbox, id)` and
// drives the Container.
//
// v0 ships stub bodies so the worker compiles, types match the ABI, and
// concurrency tracking + S-code mapping are exercised by the smoke tests.
// The follow-up commit wires real fetch() calls once the bridge lands.

import { type CreatedSandbox, type ExecResult, S, SandboxError, type SandboxRecord } from './sandbox.js'

export interface BridgeAuth {
  url: string
  token: string
}

export class CfBridgeClient {
  bridge_url: string
  token: string

  constructor(auth: BridgeAuth) {
    this.bridge_url = auth.url.replace(/\/$/, '')
    this.token = auth.token
  }

  async create(_image: string, _idle_timeout_secs: number): Promise<CreatedSandbox> {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: POST $bridge/create')
  }

  async exec(_sandbox_id: string, _cmd: string, _args: string[], _timeout_ms?: number): Promise<ExecResult> {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: POST $bridge/exec')
  }

  async stop(_sandbox_id: string): Promise<void> {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: POST $bridge/stop')
  }

  async list(): Promise<SandboxRecord[]> {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: GET $bridge/list')
  }

  async exposePort(_sandbox_id: string, _port: number): Promise<string> {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: POST $bridge/expose-port')
  }

  async fsRead(_sandbox_id: string, _path: string): Promise<Uint8Array> {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: POST $bridge/fs/read')
  }

  async fsWrite(_sandbox_id: string, _path: string, _bytes: Uint8Array, _mode?: number): Promise<void> {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'TODO: POST $bridge/fs/write')
  }
}

export function authFromEnv(cfg: { bridge_url_env: string; bridge_token_env: string }): BridgeAuth {
  const url = process.env[cfg.bridge_url_env]
  const token = process.env[cfg.bridge_token_env]
  if (!url) throw new Error(`sandbox-cloudflare: ${cfg.bridge_url_env} is not set; deploy bridge/ first`)
  if (!token) throw new Error(`sandbox-cloudflare: ${cfg.bridge_token_env} is not set; share secret with bridge`)
  return { url, token }
}
