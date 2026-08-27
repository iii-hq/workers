import { type ChildProcess, spawn } from 'node:child_process'
import { watch } from 'node:fs'
import { readFile } from 'node:fs/promises'
import http, { type IncomingMessage, type ServerResponse } from 'node:http'
import net from 'node:net'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'
import { uiPage, uiStyles } from 'virtual:aspire-dashboard-ui'
import { type IIIClient, registerWorker } from 'iii-sdk'
import { type ChangedReason, createChangedFeed } from './changed.js'
import { type Config, loadConfig, otlpApiKey, toRuntime } from './config.js'
import {
  bindConfigTrigger,
  CONFIG_ID,
  fetchRuntime,
  registerAspireDashboardConfig,
  watchConfiguration,
} from './configuration.js'
import { assertPortsFree, processExited, singleFlight, waitForHttp } from './lifecycle.js'

const { values } = parseArgs({
  options: {
    config: { type: 'string', default: './config.yaml' },
    url: { type: 'string' },
  },
  strict: false,
})

const seed = await loadConfig(String(values.config))
const url =
  (values.url ? String(values.url) : undefined) ?? process.env.III_URL ?? process.env.III_ENGINE_URL ?? seed.engine_url
const holder: { current: Config } = { current: { ...seed, engine_url: url } }

const iii = registerWorker(url, {
  workerName: 'aspire-dashboard',
  workerDescription: 'Microsoft Aspire Dashboard for iii observability: web UI plus OTLP/gRPC setup helpers.',
})

type DashboardState = 'stopped' | 'starting' | 'running' | 'failed'

type DashboardProcess = {
  process: ChildProcess
  started_at: string
  exit_code: number | null
  state: DashboardState
  last_error: string | null
}

let dashboard: DashboardProcess | null = null

const fields = {
  state: { type: 'string', enum: ['stopped', 'starting', 'running', 'failed'] },
  dashboard_url: { type: 'string' },
  proxy_url: { type: 'string' },
  otlp_endpoint: { type: 'string' },
  otlp_http_endpoint: { type: 'string' },
  otlp_secure: { type: 'boolean' },
  pid: { type: ['integer', 'null'] },
  exit_code: { type: ['integer', 'null'] },
  last_error: { type: ['string', 'null'] },
}

function schema(properties: Record<string, unknown>, required: string[] = []) {
  return { type: 'object' as const, properties, required, additionalProperties: false }
}

function hostForUrl(host: string) {
  return host === '0.0.0.0' || host === '::' ? '127.0.0.1' : host
}

function dashboardUrl(config = holder.current) {
  const host = hostForUrl(config.bind_host)
  return `http://${host.includes(':') ? `[${host}]` : host}:${config.dashboard_port}/`
}

function proxyUrl(config = holder.current) {
  const host = hostForUrl(config.bind_host)
  return `http://${host.includes(':') ? `[${host}]` : host}:${config.proxy_port}/`
}

function otlpEndpoint(config = holder.current) {
  const host = hostForUrl(config.bind_host)
  return `http://${host.includes(':') ? `[${host}]` : host}:${config.otlp_port}`
}

function otlpHttpEndpoint(config = holder.current) {
  const host = hostForUrl(config.bind_host)
  return `http://${host.includes(':') ? `[${host}]` : host}:${config.otlp_http_port}`
}

function publicDashboard() {
  return {
    state: dashboard?.state ?? 'stopped',
    dashboard_url: dashboardUrl(),
    proxy_url: proxyUrl(),
    otlp_endpoint: otlpEndpoint(),
    otlp_http_endpoint: otlpHttpEndpoint(),
    otlp_secure: holder.current.secure_otlp,
    pid: dashboard?.process.pid ?? null,
    exit_code: dashboard?.exit_code ?? null,
    last_error: dashboard?.last_error ?? null,
  }
}

const CHANGED_TRIGGER = 'aspire-dashboard::changed'

/**
 * Console pages subscribe to this instead of polling `aspire-dashboard::status`,
 * so a page re-reads only when there is something new to read.
 */
const changed = createChangedFeed(publicDashboard, (binding, event) => {
  void iii
    .trigger({
      function_id: binding.function_id,
      payload: event,
      timeoutMs: 10_000,
      ...(binding.namespace ? { namespace: binding.namespace } : {}),
    })
    .catch((err) => console.error(`[aspire-dashboard] ${binding.function_id} rejected a change event: ${String(err)}`))
})

const emitChanged = (reason: ChangedReason) => changed.emit(reason)

iii.registerTriggerType<Record<string, never>>(
  {
    id: CHANGED_TRIGGER,
    description:
      "Fires when the Aspire Dashboard process changes state, when this worker's own configuration is updated, or when iii-observability configuration is updated. Payload: reason (dashboard|observability) and the dashboard snapshot that aspire-dashboard::status also reports. Bind with an empty config.",
  },
  {
    async registerTrigger({ id, function_id, namespace }) {
      changed.bind(id, { function_id, namespace })
    },
    async unregisterTrigger({ id }) {
      changed.unbind(id)
    },
  },
)

type ProxyState = {
  server: http.Server
  host: string
  port: number
}

let proxy: ProxyState | null = null

// Upgraded sockets outlive the request that created them, and `server.close()`
// neither destroys them nor fires its callback while one is open. A single live
// dashboard websocket would otherwise stall proxy replacement and shutdown.
const upgradedSockets = new Set<net.Socket>()

const droppedProxyHeaders = new Set([
  'connection',
  'content-security-policy',
  'content-security-policy-report-only',
  'keep-alive',
  'proxy-authenticate',
  'proxy-authorization',
  'te',
  'trailer',
  'transfer-encoding',
  'upgrade',
  'x-frame-options',
])

function copyProxyHeaders(source: http.IncomingHttpHeaders) {
  const headers: Record<string, string | string[]> = {}
  for (const [key, value] of Object.entries(source)) {
    if (value == null || droppedProxyHeaders.has(key.toLowerCase())) continue
    headers[key] = value
  }
  return headers
}

function proxyRequest(req: IncomingMessage, res: ServerResponse) {
  const config = holder.current
  const target = http.request(
    {
      host: hostForUrl(config.bind_host),
      port: config.dashboard_port,
      method: req.method,
      path: req.url ?? '/',
      headers: { ...copyProxyHeaders(req.headers), host: `${hostForUrl(config.bind_host)}:${config.dashboard_port}` },
    },
    (upstream) => {
      res.writeHead(upstream.statusCode ?? 502, upstream.statusMessage, copyProxyHeaders(upstream.headers))
      upstream.pipe(res)
    },
  )
  target.on('error', (err) => {
    if (res.headersSent) {
      res.destroy(err)
      return
    }
    res.writeHead(502, { 'content-type': 'text/plain; charset=utf-8' })
    res.end(`Aspire Dashboard proxy error: ${err.message}`)
  })
  req.pipe(target)
}

function proxyUpgrade(req: IncomingMessage, socket: net.Socket, head: Buffer) {
  const config = holder.current
  upgradedSockets.add(socket)
  socket.once('close', () => upgradedSockets.delete(socket))
  const upstream = net.connect(config.dashboard_port, hostForUrl(config.bind_host), () => {
    const headers = copyProxyHeaders(req.headers)
    headers.host = `${hostForUrl(config.bind_host)}:${config.dashboard_port}`
    if (req.headers.upgrade) {
      headers.connection = 'Upgrade'
      headers.upgrade = req.headers.upgrade
    }
    upstream.write(`${req.method ?? 'GET'} ${req.url ?? '/'} HTTP/${req.httpVersion}\r\n`)
    for (const [key, value] of Object.entries(headers)) {
      const values = Array.isArray(value) ? value : [value]
      for (const item of values) upstream.write(`${key}: ${item}\r\n`)
    }
    upstream.write('\r\n')
    if (head.length > 0) upstream.write(head)
    upstream.pipe(socket)
    socket.pipe(upstream)
  })
  upstream.on('error', () => socket.destroy())
  socket.on('error', () => upstream.destroy())
}

async function closeProxy() {
  if (!proxy) return
  for (const socket of upgradedSockets) socket.destroy()
  upgradedSockets.clear()
  await new Promise<void>((done) => proxy?.server.close(() => done()))
  proxy = null
}

async function ensureProxy() {
  const config = holder.current
  if (proxy && proxy.host === config.bind_host && proxy.port === config.proxy_port) return
  await closeProxy()
  const server = http.createServer(proxyRequest)
  server.on('upgrade', proxyUpgrade)
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject)
    server.listen(config.proxy_port, config.bind_host, () => {
      server.off('error', reject)
      resolve()
    })
  })
  proxy = { server, host: config.bind_host, port: config.proxy_port }
}

function pipeWithBackpressure(source: NodeJS.ReadableStream | null, dest: NodeJS.WritableStream, prefix: string) {
  source?.on('data', (chunk) => {
    if (!dest.write(`${prefix}${chunk}`)) source.pause()
  })
  dest.on('drain', () => source?.resume())
}

async function killProcess(child: ChildProcess, graceMs: number) {
  if (processExited(child)) return
  child.kill('SIGTERM')
  const exited = await new Promise<boolean>((resolve) => {
    const timer = setTimeout(() => resolve(false), graceMs)
    child.once('exit', () => {
      clearTimeout(timer)
      resolve(true)
    })
  })
  if (!exited) child.kill('SIGKILL')
}

async function spawnDashboard() {
  const existing = dashboard
  if (existing && existing.state === 'running' && !processExited(existing.process)) {
    return publicDashboard()
  }

  await ensureProxy()
  await assertPortsFree(
    [holder.current.dashboard_port, holder.current.otlp_port, holder.current.otlp_http_port],
    holder.current.bind_host,
  )

  const config = holder.current
  const listenHost = config.bind_host.includes(':') ? `[${config.bind_host}]` : config.bind_host

  const args = [
    ...config.aspire_command.slice(1),
    '--frontend-url',
    `http://${listenHost}:${config.dashboard_port}`,
    '--otlp-grpc-url',
    `http://${listenHost}:${config.otlp_port}`,
    '--otlp-http-url',
    `http://${listenHost}:${config.otlp_http_port}`,
    // Unsecures the frontend and, unless overridden below, OTLP too — the CLI has no separate
    // frontend-only flag, and env vars don't reach the dashboard process through the CLI's spawn
    // chain, so OTLP auth has to be forced back on via additional args (after `--`), not env.
    '--allow-anonymous',
  ]
  if (config.secure_otlp) {
    args.push('--', '--Dashboard:Otlp:AuthMode=ApiKey', `--Dashboard:Otlp:PrimaryApiKey=${otlpApiKey(config)}`)
  }

  const child = spawn(config.aspire_command[0], args, { stdio: ['ignore', 'pipe', 'pipe'], detached: false })

  dashboard = {
    process: child,
    started_at: new Date().toISOString(),
    exit_code: null,
    state: 'starting',
    last_error: null,
  }

  emitChanged('dashboard')

  let spawnFailed = false
  child.once('error', (err) => {
    spawnFailed = true
    if (!dashboard || dashboard.process !== child) return
    dashboard.state = 'failed'
    dashboard.last_error = `Failed to start Aspire Dashboard: ${String(err)}`
    emitChanged('dashboard')
  })
  child.once('exit', (code) => {
    if (!dashboard || dashboard.process !== child) return
    dashboard.exit_code = code
    dashboard.state = code === 0 ? 'stopped' : 'failed'
    emitChanged('dashboard')
  })
  pipeWithBackpressure(child.stderr, process.stderr, '[aspire-dashboard] ')
  pipeWithBackpressure(child.stdout, process.stdout, '[aspire-dashboard] ')

  const outcome = await waitForHttp({
    url: dashboardUrl(config),
    timeoutMs: config.start_timeout_ms,
    exited: () => processExited(child) || spawnFailed,
  })
  if (outcome !== 'ready') {
    const message = spawnFailed
      ? (dashboard.last_error ?? 'Aspire Dashboard failed to start')
      : outcome === 'exited'
        ? `Aspire Dashboard exited before becoming ready (code ${dashboard.exit_code})`
        : `Aspire Dashboard did not become ready within ${config.start_timeout_ms}ms`
    dashboard.state = 'failed'
    dashboard.last_error = message
    await killProcess(child, config.stop_grace_ms)
    emitChanged('dashboard')
    throw new Error(message)
  }

  dashboard.state = 'running'
  emitChanged('dashboard')
  return publicDashboard()
}

// Boot auto-start and an `aspire-dashboard::start` trigger can arrive together.
const startDashboard = singleFlight(spawnDashboard)

async function stopDashboard() {
  if (dashboard) await killProcess(dashboard.process, holder.current.stop_grace_ms)
  if (dashboard) {
    dashboard.state = 'stopped'
    dashboard.exit_code = dashboard.exit_code ?? 0
  }
  emitChanged('dashboard')
  return publicDashboard()
}

type ObservabilityConfig = Record<string, unknown>

type ObservabilityStatus = {
  registered: boolean
  configured: boolean
  endpoint_matches: boolean
  endpoint: string | null
  desired_endpoint: string
  traces_exporter: string | null
  logs_exporter: string | null
  metrics_exporter: string | null
  traces_preserve_local: boolean
  logs_preserve_local: boolean
  metrics_preserve_local: boolean
  metrics_can_preserve_local_with_otlp: boolean
  otlp_secure: boolean
  otlp_api_key_configured: boolean
  warnings: string[]
  config: ObservabilityConfig | null
}

async function getObservabilityConfig(client: IIIClient): Promise<ObservabilityConfig | null> {
  try {
    const result = await client.trigger<unknown, { value?: unknown }>({
      function_id: 'configuration::get',
      namespace: 'default',
      payload: { id: 'iii-observability', raw: false },
      timeoutMs: 5_000,
    })
    return result.value && typeof result.value === 'object' ? (result.value as ObservabilityConfig) : null
  } catch {
    return null
  }
}

function stringField(config: ObservabilityConfig | null, key: string) {
  const value = config?.[key]
  return typeof value === 'string' ? value : null
}

function observabilityStatus(config: ObservabilityConfig | null): ObservabilityStatus {
  const desired = otlpEndpoint()
  const endpoint = stringField(config, 'endpoint')
  const traces = stringField(config, 'exporter')
  const logs = stringField(config, 'logs_exporter')
  const metrics = stringField(config, 'metrics_exporter')
  const endpointMatches = endpoint === desired
  const tracesConfigured = traces === 'otlp' || traces === 'both'
  const logsConfigured = logs === 'otlp' || logs === 'both'
  const configuredApiKey = stringField(config, 'otlp_api_key')
  const headers = config?.otlp_headers && typeof config.otlp_headers === 'object' ? config.otlp_headers : null
  const configuredHeader = headers && 'x-otlp-api-key' in headers ? headers['x-otlp-api-key'] : null
  const apiKeyMatches =
    !holder.current.secure_otlp ||
    configuredApiKey === otlpApiKey(holder.current) ||
    configuredHeader === otlpApiKey(holder.current)
  const warnings: string[] = []
  if (!config) warnings.push('iii-observability is not registered with the configuration worker.')
  if (holder.current.secure_otlp) {
    warnings.push(
      'secure_otlp is on, but iii-observability cannot authenticate to a secure OTLP endpoint: its exporter sends no gRPC metadata, and its config schema rejects unknown fields such as otlp_api_key. Set secure_otlp: false, or pass the key to the engine process as OTEL_EXPORTER_OTLP_HEADERS=x-otlp-api-key=<key>.',
    )
  }
  if (config && !endpointMatches) {
    warnings.push(
      'endpoint, exporter, and metrics_exporter are restart-tier in iii-observability: the configuration write takes effect at the next engine start, not immediately.',
    )
  }
  if (config && metrics === 'memory') {
    warnings.push(
      'iii-observability currently cannot export metrics to OTLP and keep its local metric store at the same time; its schema supports metrics_exporter: memory or otlp, not both.',
    )
  }
  return {
    registered: Boolean(config),
    configured: Boolean(config && endpointMatches && tracesConfigured && logsConfigured && apiKeyMatches),
    endpoint_matches: endpointMatches,
    endpoint,
    desired_endpoint: desired,
    traces_exporter: traces,
    logs_exporter: logs,
    metrics_exporter: metrics,
    traces_preserve_local: traces === 'both' || traces === 'memory',
    logs_preserve_local: logs === 'both' || logs === 'memory',
    metrics_preserve_local: metrics === 'memory',
    metrics_can_preserve_local_with_otlp: false,
    otlp_secure: holder.current.secure_otlp,
    otlp_api_key_configured: apiKeyMatches,
    warnings,
    config,
  }
}

async function getStatus() {
  await ensureProxy()
  const config = await getObservabilityConfig(iii)
  const dashboardHealthy = await fetch(dashboardUrl(), { redirect: 'manual', signal: AbortSignal.timeout(3_000) })
    .then((res) => res.status >= 200 && res.status < 400)
    .catch(() => false)
  return {
    dashboard: publicDashboard(),
    dashboard_healthy: dashboardHealthy,
    observability: observabilityStatus(config),
  }
}

async function configureObservability(input: { include_metrics?: boolean } = {}) {
  const current = await getObservabilityConfig(iii)
  if (!current) throw new Error('iii-observability configuration is not registered')
  const includeMetrics = input.include_metrics === true
  // No otlp_api_key here: ObservabilityWorkerConfig is deny_unknown_fields, so
  // its registered schema is additionalProperties:false and configuration::set
  // rejects the whole write. The key has to reach the engine process as
  // OTEL_EXPORTER_OTLP_HEADERS instead — see the secure_otlp warning.
  const next: ObservabilityConfig = {
    ...current,
    enabled: true,
    endpoint: otlpEndpoint(),
    exporter: 'both',
    logs_enabled: true,
    logs_exporter: 'both',
  }
  if (includeMetrics) {
    next.metrics_enabled = true
    next.metrics_exporter = 'otlp'
  }
  await iii.trigger({
    function_id: 'configuration::set',
    namespace: 'default',
    payload: { id: 'iii-observability', value: next },
    timeoutMs: 5_000,
  })
  return {
    changed: true,
    include_metrics: includeMetrics,
    observability: observabilityStatus(next),
  }
}

iii.registerFunction('aspire-dashboard::start', startDashboard, {
  description: 'Start or reuse the standalone Aspire Dashboard process managed by this worker.',
  request_format: schema({}),
  response_format: schema(fields, ['state', 'dashboard_url', 'proxy_url', 'otlp_endpoint', 'otlp_http_endpoint']),
})

iii.registerFunction('aspire-dashboard::stop', stopDashboard, {
  description: 'Stop the Aspire Dashboard process managed by this worker.',
  request_format: schema({}),
  response_format: schema(fields, ['state', 'dashboard_url', 'proxy_url', 'otlp_endpoint', 'otlp_http_endpoint']),
})

iii.registerFunction('aspire-dashboard::status', getStatus, {
  description: 'Report Aspire Dashboard process health and whether iii-observability exports OTLP/gRPC to it.',
  request_format: schema({}),
  response_format: { type: 'object', additionalProperties: true },
})

iii.registerFunction('aspire-dashboard::configure-observability', configureObservability, {
  description:
    'Update iii-observability to export traces/logs to this dashboard; pass include_metrics=true to also switch metrics to OTLP.',
  request_format: schema({ include_metrics: { type: 'boolean' } }),
  response_format: { type: 'object', additionalProperties: true },
})

type UiAsset = {
  file: string
  type: 'console:script' | 'console:style'
  content_type: string
  content: string
}

const uiAssets: Record<string, UiAsset> = {
  'aspire-dashboard/page.js': {
    file: 'page.js',
    type: 'console:script',
    content_type: 'text/javascript',
    content: uiPage,
  },
  'aspire-dashboard/styles.css': {
    file: 'styles.css',
    type: 'console:style',
    content_type: 'text/css',
    content: uiStyles,
  },
}

const uiWatch = process.env.III_ASPIRE_DASHBOARD_UI_WATCH
const uiWatchEnabled = Boolean(uiWatch)
const uiWatchDir =
  uiWatchEnabled && uiWatch !== '1' && uiWatch !== 'true'
    ? (uiWatch as string)
    : join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'ui', 'dist')

async function uiContent(path: string) {
  const asset = uiAssets[path]
  if (!asset) throw new Error(`unknown ui asset: ${path}`)
  const content = uiWatchEnabled ? await readFile(join(uiWatchDir, asset.file), 'utf8') : asset.content
  return { content, content_type: asset.content_type }
}

iii.registerFunction('aspire-dashboard::ui-content', (input: { path: string }) => uiContent(input.path), {
  description: 'Serve the injectable Aspire Dashboard Console page assets.',
  metadata: { internal: true },
  request_format: schema({ path: { type: 'string' } }, ['path']),
  response_format: schema({ content: { type: 'string' }, content_type: { type: 'string' } }, [
    'content',
    'content_type',
  ]),
})

function registerUiAsset(path: string) {
  return iii.registerTrigger({
    type: uiAssets[path].type,
    function_id: 'aspire-dashboard::ui-content',
    config: { path },
  })
}

const uiTriggers = new Map(Object.keys(uiAssets).map((path) => [path, registerUiAsset(path)]))

if (uiWatchEnabled) {
  const pending = new Map<string, NodeJS.Timeout>()
  watch(uiWatchDir, (_event, file) => {
    const path = Object.keys(uiAssets).find((key) => uiAssets[key].file === file)
    if (!path) return
    clearTimeout(pending.get(path))
    pending.set(
      path,
      setTimeout(() => {
        const previous = uiTriggers.get(path)
        uiTriggers.set(path, registerUiAsset(path))
        previous?.unregister()
        console.error(`[aspire-dashboard] reloaded ui asset ${path}`)
      }, 150),
    )
  })
  console.error(`[aspire-dashboard] serving ui assets from ${uiWatchDir}`)
}

try {
  await registerAspireDashboardConfig(iii, holder.current)
} catch (err) {
  console.warn(`configuration::register failed; continuing with the seed: ${String(err)}`)
}

await bindConfigTrigger(iii, async () => {
  const runtime = await fetchRuntime(iii)
  if (runtime) holder.current = { engine_url: url, ...runtime }
  // The ports and secure_otlp all show up in the status a page renders.
  emitChanged('dashboard')
})

// One process-lifetime watch on iii-observability, relayed into the page's
// single subscription. The pages get their observability updates from this
// worker rather than binding a `configuration` trigger per tab.
watchConfiguration(
  iii,
  'iii-observability',
  'aspire-dashboard::on-observability-config-change',
  'Internal: tell bound Console pages that iii-observability configuration changed.',
  async () => emitChanged('observability'),
)

// otlp_api_key is optional in the schema, so a first boot registers it
// unset. otlpApiKey() then falls back to a per-process random value that a
// worker restart regenerates, drifting from whatever key is already baked
// into a running Aspire process's argv (spawned once, from the OLD key) and
// whatever configure-observability last wrote into iii-observability's
// config (from the OLD key too) — every restart quietly breaks OTLP auth.
// Persist the generated key once so every later boot reads the same value
// back (configuration::register's initial_value is seed-only, so this
// write sticks across restarts).
if (holder.current.secure_otlp && !holder.current.otlp_api_key) {
  const generated = otlpApiKey(holder.current)
  holder.current = { ...holder.current, otlp_api_key: generated }
  await iii
    .trigger({
      function_id: 'configuration::set',
      namespace: 'default',
      payload: { id: CONFIG_ID, value: toRuntime(holder.current) },
      timeoutMs: 5_000,
    })
    .catch((err) => console.warn(`failed to persist generated otlp_api_key: ${String(err)}`))
}

await ensureProxy().catch((err) => console.warn(`Aspire Dashboard proxy failed to start: ${err}`))

if (holder.current.auto_start) {
  void startDashboard().catch((err) => console.warn(`Aspire Dashboard auto-start failed: ${err}`))
}

async function shutdown() {
  await stopDashboard()
  await closeProxy()
  await iii.shutdown()
}

process.on('SIGTERM', () => void shutdown())
process.on('SIGINT', () => void shutdown())
