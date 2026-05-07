import type { VercelClient } from './client.js'
import type { Config } from './config.js'
import { imageAllowed } from './config.js'
import { CAPABILITIES, S, SandboxError } from './sandbox.js'

export interface HandlerCtx {
  config: Config
  client: VercelClient
  inFlight: { value: number }
}

function requireStr(input: Record<string, unknown>, key: string): string {
  const v = input[key]
  if (typeof v !== 'string' || v.length === 0) {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, `missing string field "${key}"`)
  }
  return v
}

export async function doCreate(ctx: HandlerCtx, input: Record<string, unknown>) {
  const image = requireStr(input, 'image')
  if (!imageAllowed(ctx.config, image)) {
    throw new SandboxError(S.IMAGE_NOT_ALLOWED, `image not in allowlist: ${image}`)
  }
  const idle =
    typeof input.idle_timeout_secs === 'number' ? input.idle_timeout_secs : ctx.config.default_idle_timeout_secs
  const runtime = typeof input.runtime === 'string' ? input.runtime : ctx.config.default_runtime

  if (ctx.inFlight.value >= ctx.config.max_concurrent_sandboxes) {
    throw new SandboxError(S.CONCURRENCY_CAP, `concurrency cap reached (${ctx.inFlight.value} active)`)
  }
  ctx.inFlight.value += 1
  try {
    const sourceUrl = typeof input.source_url === 'string' ? input.source_url : undefined
    const sourceRev = typeof input.source_revision === 'string' ? input.source_revision : undefined
    const created = await ctx.client.create({
      image,
      idle_timeout_secs: idle,
      runtime,
      source: sourceUrl ? { url: sourceUrl, revision: sourceRev } : undefined,
    })
    return {
      sandbox_id: created.sandbox_id,
      image: created.image,
      started_at: created.started_at,
      capabilities: CAPABILITIES,
    }
  } catch (e) {
    ctx.inFlight.value -= 1
    throw e
  }
}

export async function doExec(ctx: HandlerCtx, input: Record<string, unknown>) {
  const sandbox_id = requireStr(input, 'sandbox_id')
  const cmd = requireStr(input, 'cmd')
  const args = Array.isArray(input.args) ? (input.args as string[]) : []
  const timeout_ms = typeof input.timeout_ms === 'number' ? input.timeout_ms : undefined
  return ctx.client.exec(sandbox_id, cmd, args, timeout_ms)
}

export async function doStop(ctx: HandlerCtx, input: Record<string, unknown>) {
  const sandbox_id = requireStr(input, 'sandbox_id')
  await ctx.client.stop(sandbox_id)
  if (ctx.inFlight.value > 0) ctx.inFlight.value -= 1
  return {}
}

export async function doList(ctx: HandlerCtx, _input: Record<string, unknown>) {
  // Best-effort upstream fetch. Local capacity envelope is always returned
  // so callers can still trust in_flight/cap/remaining when the provider
  // is down.
  let sandboxes: Awaited<ReturnType<VercelClient['list']>> = []
  try {
    sandboxes = await ctx.client.list()
  } catch {
    sandboxes = []
  }
  const cap = ctx.config.max_concurrent_sandboxes
  return {
    sandboxes,
    in_flight: ctx.inFlight.value,
    cap,
    remaining: Math.max(cap - ctx.inFlight.value, 0),
  }
}

export async function doSnapshot(ctx: HandlerCtx, input: Record<string, unknown>) {
  const sandbox_id = requireStr(input, 'sandbox_id')
  const snapshot_id = await ctx.client.snapshot(sandbox_id)
  return { snapshot_id }
}

export async function doExposePort(ctx: HandlerCtx, input: Record<string, unknown>) {
  const sandbox_id = requireStr(input, 'sandbox_id')
  const port = input.port
  if (typeof port !== 'number' || port < 1 || port > 65535) {
    throw new SandboxError(S.PROVIDER_UNAVAILABLE, 'invalid port')
  }
  const url = await ctx.client.exposePort(sandbox_id, port)
  return { url }
}

export async function doFsRead(ctx: HandlerCtx, input: Record<string, unknown>) {
  const sandbox_id = requireStr(input, 'sandbox_id')
  const path = requireStr(input, 'path')
  const bytes = await ctx.client.fsRead(sandbox_id, path)
  return { bytes_base64: Buffer.from(bytes).toString('base64') }
}

export async function doFsWrite(ctx: HandlerCtx, input: Record<string, unknown>) {
  const sandbox_id = requireStr(input, 'sandbox_id')
  const path = requireStr(input, 'path')
  const bytes_b64 = requireStr(input, 'bytes_base64')
  const bytes = Buffer.from(bytes_b64, 'base64')
  const mode = typeof input.mode === 'number' ? input.mode : undefined
  await ctx.client.fsWrite(sandbox_id, path, bytes, mode)
  return {}
}
