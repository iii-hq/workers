// Stable error code space shared with every sandbox provider worker in the
// iii-hq/workers monorepo. S100-S400 mirror the libkrun-backed iii-sandbox
// daemon; S404 and S500-S503 are bridge-/REST-specific extensions.
export const S = {
  IMAGE_NOT_ALLOWED: 'S100',
  CONCURRENCY_CAP: 'S400',
  CAPABILITY_UNSUPPORTED: 'S404',
  RATE_LIMITED: 'S500',
  QUOTA_EXHAUSTED: 'S501',
  PROVIDER_UNAVAILABLE: 'S502',
  AUTH_INVALID: 'S503',
} as const

export type SCode = (typeof S)[keyof typeof S]

export class SandboxError extends Error {
  code: SCode
  constructor(code: SCode, message: string) {
    super(`[${code}] ${message}`)
    this.code = code
    this.name = 'SandboxError'
  }
}

// Capabilities advertised by sandbox::provider::cloudflare::create. CF Sandbox doesn't ship
// snapshot or branch; the bridge exposes only exec, fs, and port surfaces.
export const CAPABILITIES = ['expose_port', 'fs'] as const

// Map a non-2xx HTTP status from the bridge onto the right S-code.
export function mapHttpStatus(status: number, body: string): SandboxError {
  if (status === 401 || status === 403) return new SandboxError(S.AUTH_INVALID, 'bridge auth invalid')
  if (status === 402) return new SandboxError(S.QUOTA_EXHAUSTED, 'quota exhausted')
  if (status === 429) return new SandboxError(S.RATE_LIMITED, 'rate limited by bridge')
  if (status >= 500) return new SandboxError(S.PROVIDER_UNAVAILABLE, `bridge ${status}: ${body}`)
  return new SandboxError(S.PROVIDER_UNAVAILABLE, `bridge unexpected ${status}: ${body}`)
}

export interface CreatedSandbox {
  sandbox_id: string
  image: string
  started_at: number
}

export interface ExecResult {
  stdout: string
  stderr: string
  exit_code: number
  timed_out: boolean
}

export interface SandboxRecord {
  sandbox_id: string
  image: string
  started_at: number
}
