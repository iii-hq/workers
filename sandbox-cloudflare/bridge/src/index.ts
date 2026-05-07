// Cloudflare Worker bridge for sandbox-cloudflare. The iii worker side (parent
// dir) talks to this Worker over HTTPS; this Worker calls @cloudflare/sandbox's
// `getSandbox(env.Sandbox, id)` and drives the underlying Container
// Durable Object.
//
// Auth: shared bearer token, set via `wrangler secret put CLOUDFLARE_BRIDGE_TOKEN`.
//
// SDK shapes verified via context7 against /cloudflare/sandbox-sdk:
//   sandbox.exec(cmd, {timeout, cwd, env, signal}) → {stdout, stderr, exitCode, success}
//   sandbox.writeFile(path, content)
//   sandbox.readFile(path) → {content}
//   sandbox.exposePort(port, {hostname, name, token}) → {url, port, name}
//   sandbox.destroy()

import { getSandbox, proxyToSandbox, type Sandbox } from '@cloudflare/sandbox'

interface Env {
  Sandbox: DurableObjectNamespace<Sandbox>
  CLOUDFLARE_BRIDGE_TOKEN: string
  PREVIEW_HOSTNAME?: string
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

function unauthorized(): Response {
  return jsonResponse({ error: 'unauthorized' }, 401)
}

function bridgeError(status: number, message: string): Response {
  return jsonResponse({ error: message }, status)
}

async function readJson<T = Record<string, unknown>>(req: Request): Promise<T> {
  try {
    return (await req.json()) as T
  } catch {
    return {} as T
  }
}

interface CreatePayload {
  sandbox_id?: string
  image?: string
  idle_timeout_secs?: number
}

interface ExecPayload {
  sandbox_id: string
  cmd: string
  args?: string[]
  cwd?: string
  env?: Record<string, string>
  timeout_ms?: number
}

interface IdPayload {
  sandbox_id: string
}

interface ExposePortPayload {
  sandbox_id: string
  port: number
  name?: string
  token?: string
}

interface FsReadPayload {
  sandbox_id: string
  path: string
}

interface FsWritePayload {
  sandbox_id: string
  path: string
  bytes_base64: string
  mode?: number
}

function buildExecCommand(cmd: string, args: string[] | undefined): string {
  if (!args || args.length === 0) return cmd
  // Naively quote args; sandbox.exec runs the resulting string in a shell.
  const quoted = args.map((a) => `'${a.replace(/'/g, "'\\''")}'`)
  return [cmd, ...quoted].join(' ')
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    // Required for preview URLs (sandbox.exposePort routing).
    const proxyResponse = await proxyToSandbox(req, env)
    if (proxyResponse) return proxyResponse

    const auth = req.headers.get('Authorization') ?? ''
    if (!auth.startsWith('Bearer ') || auth.slice(7) !== env.CLOUDFLARE_BRIDGE_TOKEN) {
      return unauthorized()
    }

    const url = new URL(req.url)
    const route = `${req.method} ${url.pathname}`

    try {
      switch (route) {
        case 'POST /create': {
          const payload = await readJson<CreatePayload>(req)
          const id = payload.sandbox_id ?? crypto.randomUUID()
          const sandbox = getSandbox(env.Sandbox, id, {
            sleepAfter: payload.idle_timeout_secs ? `${payload.idle_timeout_secs}s` : '5m',
          })
          // Touch the sandbox so the container is provisioned. exec("true")
          // is the cheapest way to force the DO to wake the container.
          await sandbox.exec('true')
          return jsonResponse({
            sandbox_id: id,
            image: payload.image ?? 'default',
            started_at: new Date().toISOString(),
          })
        }
        case 'POST /exec': {
          const p = await readJson<ExecPayload>(req)
          if (!p.sandbox_id || !p.cmd) return bridgeError(400, 'missing sandbox_id or cmd')
          const sandbox = getSandbox(env.Sandbox, p.sandbox_id)
          const command = buildExecCommand(p.cmd, p.args)
          const result = await sandbox.exec(command, {
            timeout: p.timeout_ms,
            cwd: p.cwd,
            env: p.env,
          })
          return jsonResponse({
            stdout: result.stdout,
            stderr: result.stderr ?? '',
            exit_code: result.exitCode,
            timed_out: false,
            success: result.success,
          })
        }
        case 'POST /stop': {
          const p = await readJson<IdPayload>(req)
          if (!p.sandbox_id) return bridgeError(400, 'missing sandbox_id')
          const sandbox = getSandbox(env.Sandbox, p.sandbox_id)
          await sandbox.destroy()
          return jsonResponse({})
        }
        case 'GET /list': {
          // The SDK does not expose a list primitive yet. The iii worker
          // side reconciles in_flight via its local counter when the
          // upstream answer is unavailable, so we surface 501 explicitly
          // and the worker falls back to local accounting.
          return bridgeError(501, 'cf sandbox sdk has no list primitive')
        }
        case 'POST /expose-port': {
          const p = await readJson<ExposePortPayload>(req)
          if (!p.sandbox_id || !p.port) return bridgeError(400, 'missing sandbox_id or port')
          if (!env.PREVIEW_HOSTNAME) {
            return bridgeError(500, 'PREVIEW_HOSTNAME var not set; expose-port requires a custom domain on the bridge')
          }
          const sandbox = getSandbox(env.Sandbox, p.sandbox_id)
          // The SDK's exposePort options vary across versions — older
          // releases accept `token` for stable URLs while newer ones use
          // a different knob. We pass only the universally-supported
          // fields and let `name` carry caller intent.
          const _token = p.token
          const result = await sandbox.exposePort(p.port, {
            hostname: env.PREVIEW_HOSTNAME,
            name: p.name,
          })
          return jsonResponse({ url: result.url })
        }
        case 'POST /fs/read': {
          const p = await readJson<FsReadPayload>(req)
          if (!p.sandbox_id || !p.path) return bridgeError(400, 'missing sandbox_id or path')
          const sandbox = getSandbox(env.Sandbox, p.sandbox_id)
          const file = await sandbox.readFile(p.path)
          // Caller expects base64 bytes; readFile returns string content.
          const bytes_base64 = btoa(unescape(encodeURIComponent(file.content)))
          return jsonResponse({ bytes_base64 })
        }
        case 'POST /fs/write': {
          const p = await readJson<FsWritePayload>(req)
          if (!p.sandbox_id || !p.path || !p.bytes_base64) {
            return bridgeError(400, 'missing sandbox_id, path, or bytes_base64')
          }
          const sandbox = getSandbox(env.Sandbox, p.sandbox_id)
          const decoded = decodeURIComponent(escape(atob(p.bytes_base64)))
          await sandbox.writeFile(p.path, decoded)
          return jsonResponse({})
        }
        default:
          return bridgeError(404, `unknown route: ${route}`)
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err)
      return bridgeError(502, `bridge error: ${message}`)
    }
  },
}

// The Durable Object class is re-exported from @cloudflare/sandbox so
// wrangler can find it via `class_name: "Sandbox"`.
export { Sandbox } from '@cloudflare/sandbox'
