// Cloudflare Worker bridge for sandbox-cf. The iii worker side (in this
// crate's parent dir) talks to this Worker over HTTPS; this Worker calls
// `getSandbox(env.Sandbox, id)` on the @cloudflare/sandbox SDK and drives
// the underlying Container Durable Object.
//
// Auth: shared bearer token, set via `wrangler secret put CF_BRIDGE_TOKEN`.
//
// v0 ships the route shell + auth check; the actual getSandbox() calls are
// stubbed in TODO comments and return 501. The follow-up commit wires the
// SDK once we've validated the deploy story end to end.

interface Env {
  Sandbox: DurableObjectNamespace
  CF_BRIDGE_TOKEN: string
}

function unauthorized(): Response {
  return new Response(JSON.stringify({ error: 'unauthorized' }), {
    status: 401,
    headers: { 'Content-Type': 'application/json' },
  })
}

function notImplemented(route: string): Response {
  return new Response(JSON.stringify({ error: `[S502] TODO: wire ${route} -> getSandbox().*` }), {
    status: 501,
    headers: { 'Content-Type': 'application/json' },
  })
}

async function readJson(req: Request): Promise<Record<string, unknown>> {
  try {
    return (await req.json()) as Record<string, unknown>
  } catch {
    return {}
  }
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const auth = req.headers.get('Authorization') ?? ''
    if (!auth.startsWith('Bearer ') || auth.slice(7) !== env.CF_BRIDGE_TOKEN) {
      return unauthorized()
    }

    const url = new URL(req.url)
    const route = `${req.method} ${url.pathname}`

    switch (route) {
      case 'POST /create': {
        const _payload = await readJson(req)
        // TODO: const sandbox = getSandbox(env.Sandbox, payload.sandbox_id ?? crypto.randomUUID())
        // TODO: register the sandbox.exec / writeFile / readFile bindings before first use
        return notImplemented('POST /create')
      }
      case 'POST /exec': {
        const _payload = await readJson(req)
        // TODO: const r = await getSandbox(env.Sandbox, payload.sandbox_id).exec(payload.cmd, { ...opts })
        return notImplemented('POST /exec')
      }
      case 'POST /stop': {
        const _payload = await readJson(req)
        // TODO: await getSandbox(env.Sandbox, payload.sandbox_id).destroy()
        return notImplemented('POST /stop')
      }
      case 'GET /list': {
        // TODO: enumerate live sandbox ids; SDK does not currently expose
        // a list primitive — see follow-up issue.
        return notImplemented('GET /list')
      }
      case 'POST /expose-port': {
        const _payload = await readJson(req)
        // TODO: getSandbox(env.Sandbox, payload.sandbox_id).exposePort(payload.port, { ... })
        return notImplemented('POST /expose-port')
      }
      case 'POST /fs/read': {
        const _payload = await readJson(req)
        // TODO: getSandbox(env.Sandbox, payload.sandbox_id).readFile(payload.path)
        return notImplemented('POST /fs/read')
      }
      case 'POST /fs/write': {
        const _payload = await readJson(req)
        // TODO: getSandbox(env.Sandbox, payload.sandbox_id).writeFile(payload.path, decoded, { mode })
        return notImplemented('POST /fs/write')
      }
      default:
        return new Response(JSON.stringify({ error: `unknown route: ${route}` }), {
          status: 404,
          headers: { 'Content-Type': 'application/json' },
        })
    }
  },
}

// The Durable Object class is re-exported from @cloudflare/sandbox so
// wrangler can find it via `class_name: "Sandbox"`.
export { Sandbox } from '@cloudflare/sandbox'
