import {
  Badge,
  Button,
  Card,
  CardBody,
  CardHeader,
  CodeHighlight,
  EmptyState,
  type Host,
  IconButton,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  StatusPanel,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useId, useMemo, useState } from 'react'

type DashboardState = 'stopped' | 'starting' | 'running' | 'failed'

type Dashboard = {
  state: DashboardState
  dashboard_url: string
  proxy_url: string
  otlp_endpoint: string
  otlp_http_endpoint: string
  otlp_secure: boolean
  pid: number | null
  exit_code: number | null
  last_error: string | null
}

type Observability = {
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
}

type Status = {
  dashboard: Dashboard
  dashboard_healthy: boolean
  observability: Observability
}

type Props = PageRenderProps & { host: Host }

type Phase = 'loading' | 'ready' | 'error'

const timeoutMs = 130_000

/**
 * Base id for this page's browser-local change handler. The `iii::` prefix
 * keeps the per-event invocations span-suppressed, so a busy dashboard does
 * not fill the trace feed with its own change notifications.
 */
const EVENTS_FN = 'iii::aspire-dashboard-ui::changed'
const EVENT_DEBOUNCE_MS = 80

function IconGauge({ className }: { className?: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      aria-hidden="true"
    >
      <path d="M4 14a8 8 0 1 1 16 0" />
      <path d="M12 14l4-4" />
      <path d="M8 18h8" />
    </svg>
  )
}

function IconRefresh() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
      <path d="M20 12a8 8 0 0 1-13.6 5.7" />
      <path d="M4 12A8 8 0 0 1 17.6 6.3" />
      <path d="M17 2v5h5" />
      <path d="M7 22v-5H2" />
    </svg>
  )
}

function IconExternal() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
      <path d="M14 4h6v6" />
      <path d="M10 14L20 4" />
      <path d="M20 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h5" />
    </svg>
  )
}

function describe(cause: unknown): string {
  if (cause instanceof Error) return cause.message
  if (cause && typeof cause === 'object' && 'message' in cause) {
    const message = (cause as { message?: unknown }).message
    if (typeof message === 'string') return message
  }
  return String(cause)
}

function configExample(endpoint: string, includeMetrics = false) {
  const metricsPatch = includeMetrics ? ' | .metrics_enabled=true | .metrics_exporter="otlp"' : ''
  return `iii trigger configuration::set --json "$(iii trigger configuration::get id=iii-observability | jq --arg endpoint '${endpoint}' '.value | .enabled=true | .endpoint=$endpoint | .exporter="both" | .logs_enabled=true | .logs_exporter="both"${metricsPatch} | {id:"iii-observability", value:.}')"`
}

/**
 * Status feed for the page: one seed read, then a re-read only when the worker
 * says something moved. `aspire-dashboard::changed` fires on dashboard process
 * transitions, on this worker's own configuration changes, and on
 * iii-observability configuration changes, which the worker relays so no tab
 * has to hold a `configuration` trigger of its own.
 *
 * There is no interval anywhere. Reconnects and tab-visibility changes re-seed
 * instead, because those are the two moments a page can have missed an event.
 */
function useStatus(host: Host) {
  const [phase, setPhase] = useState<Phase>('loading')
  const [status, setStatus] = useState<Status | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [bound, setBound] = useState(false)

  // `status` must stay out of these deps. `refresh` sets it, so depending on it
  // would give every response a new callback identity, re-running every effect
  // below on each read — which is what turned the old interval into a poll
  // paced by round-trip latency.
  const refresh = useCallback(async () => {
    try {
      const next = await host.iii.trigger<Status>('aspire-dashboard::status', {}, { timeoutMs: 10_000 })
      setStatus(next)
      setError(null)
      setPhase('ready')
    } catch (cause) {
      setError(describe(cause))
      setPhase('error')
    }
  }, [host])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')

  useEffect(() => {
    const localFnId = `${EVENTS_FN}::${instanceId}`
    const offs: Array<() => void> = []
    let timer: number | undefined
    // Lifecycle events arrive in bursts — a start emits `starting` then
    // `running` — and one re-read serves the whole burst.
    const ping = () => {
      if (timer !== undefined) window.clearTimeout(timer)
      timer = window.setTimeout(() => void refresh(), EVENT_DEBOUNCE_MS)
    }
    try {
      offs.push(host.iii.on(localFnId, ping))
      offs.push(
        host.iii.registerTrigger({
          type: 'aspire-dashboard::changed',
          function_id: `${localFnId}::${host.iii.browserId}`,
          config: {},
        }),
      )
      setBound(true)
    } catch {
      for (const off of offs) off()
      offs.length = 0
      setBound(false)
    }
    return () => {
      if (timer !== undefined) window.clearTimeout(timer)
      setBound(false)
      for (const off of offs) off()
    }
  }, [host, refresh, instanceId])

  useEffect(() => {
    try {
      return host.iii.addConnectionStateListener((state) => {
        if (state === 'connected') void refresh()
      })
    } catch {
      return undefined
    }
  }, [host, refresh])

  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === 'visible') void refresh()
    }
    document.addEventListener('visibilitychange', onVisible)
    return () => document.removeEventListener('visibilitychange', onVisible)
  }, [refresh])

  return { phase, status, error, bound, refresh, setStatus, setError }
}

function StatusBadge({ status }: { status: Status | null }) {
  if (!status) return <Badge>loading</Badge>
  if (!status.dashboard_healthy) return <Badge variant="warn">dashboard offline</Badge>
  if (!status.observability.configured) return <Badge variant="warn">otel not configured</Badge>
  return <Badge variant="ok">receiving otel</Badge>
}

export default function setup(host: Host) {
  host.pages.register({
    id: 'aspire-dashboard',
    title: 'Aspire',
    render: (props) => <AspireDashboardPage host={host} {...props} />,
  })

  host.commands?.register('aspire-dashboard', [
    {
      id: 'open',
      title: 'Open Aspire Dashboard',
      detail: 'View OpenTelemetry data from iii-observability',
      keywords: ['otel', 'observability', 'traces', 'logs', 'metrics'],
      run: () => host.panels?.open({ pageId: 'aspire-dashboard', context: {} }),
    },
  ])
}

function AspireDashboardPage({ host, onRequestClose, commands }: Props) {
  const { phase, status, error, bound, refresh, setStatus, setError } = useStatus(host)
  const [busy, setBusy] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)

  const running = status?.dashboard_healthy === true
  const configured = status?.observability.configured === true
  const dashboardUrl = status?.dashboard.dashboard_url
  const proxyUrl = status?.dashboard.proxy_url

  const start = useCallback(async () => {
    setBusy('Starting dashboard')
    setError(null)
    try {
      await host.iii.trigger('aspire-dashboard::start', {}, { timeoutMs })
      await refresh()
    } catch (cause) {
      setError(describe(cause))
    } finally {
      setBusy(null)
    }
  }, [host, refresh, setError])

  const stop = useCallback(async () => {
    setBusy('Stopping dashboard')
    setError(null)
    try {
      await host.iii.trigger('aspire-dashboard::stop', {}, { timeoutMs: 20_000 })
      await refresh()
    } catch (cause) {
      setError(describe(cause))
    } finally {
      setBusy(null)
    }
  }, [host, refresh, setError])

  const configure = useCallback(
    async (includeMetrics: boolean) => {
      setBusy(includeMetrics ? 'Configuring traces, logs, and metrics' : 'Configuring traces and logs')
      setError(null)
      try {
        await host.iii.trigger(
          'aspire-dashboard::configure-observability',
          { include_metrics: includeMetrics },
          { timeoutMs: 10_000 },
        )
        const next = await host.iii.trigger<Status>('aspire-dashboard::status', {}, { timeoutMs: 10_000 })
        setStatus(next)
        setNotice(
          'iii-observability is configured. Its endpoint and exporter settings are restart-tier, so restart the engine before telemetry reaches this dashboard.',
        )
      } catch (cause) {
        setError(describe(cause))
      } finally {
        setBusy(null)
      }
    },
    [host, setError, setStatus],
  )

  const openExternal = useCallback(() => {
    if (dashboardUrl) window.open(dashboardUrl, '_blank', 'noopener')
  }, [dashboardUrl])

  useEffect(
    () =>
      commands?.register([
        { id: 'refresh', title: 'Refresh status', shortcut: 'R', run: () => void refresh() },
        { id: 'start', title: 'Start dashboard', shortcut: 'S', enabled: () => !running, run: () => void start() },
        {
          id: 'open-external',
          title: 'Open in a browser tab',
          shortcut: 'O',
          enabled: () => Boolean(dashboardUrl),
          run: openExternal,
        },
        { id: 'stop', title: 'Stop dashboard', shortcut: 'X', enabled: () => running, run: () => void stop() },
      ]),
    [commands, refresh, running, start, dashboardUrl, openExternal, stop],
  )

  const actions = useMemo(
    () => (
      <>
        <IconButton label="Refresh status" variant="ghost" onClick={() => void refresh()}>
          <IconRefresh />
        </IconButton>
        <IconButton label="Open in browser" variant="ghost" onClick={openExternal} disabled={!dashboardUrl}>
          <IconExternal />
        </IconButton>
        {running ? (
          <Button variant="ghost" onClick={() => void stop()} disabled={Boolean(busy)}>
            Stop
          </Button>
        ) : (
          <Button variant="primary" onClick={() => void start()} disabled={Boolean(busy)}>
            Start
          </Button>
        )}
      </>
    ),
    [refresh, openExternal, dashboardUrl, running, busy, stop, start],
  )

  return (
    <PageShell className="aspire-dashboard-shell">
      <PageHeader
        icon={<IconGauge />}
        title="Aspire Dashboard"
        description="OpenTelemetry traces, logs, and metrics from iii"
        actions={actions}
        onClose={onRequestClose}
      >
        <StatusBadge status={status} />
      </PageHeader>
      <PageMain className="aspire-dashboard-main">
        {phase === 'loading' && !status ? (
          <EmptyState
            icon={IconGauge}
            title="Loading dashboard status"
            description="Checking Aspire Dashboard and iii-observability."
          />
        ) : (
          <>
            {(error || phase === 'error') && (
              <StatusPanel
                variant="alert"
                headline="Aspire Dashboard needs attention"
                detail={error ?? 'Status could not be loaded.'}
              />
            )}
            {busy && (
              <StatusPanel
                variant="info"
                headline={busy}
                detail="This may take a moment on first run while npx fetches the Aspire CLI."
              />
            )}
            {notice && !busy && <StatusPanel variant="info" headline="Restart the engine to apply" detail={notice} />}
            {!bound && phase !== 'loading' && (
              <StatusPanel
                variant="warn"
                headline="Live updates are not bound"
                detail="This page could not subscribe to aspire-dashboard::changed, so it will not notice changes on its own. Use Refresh, or reopen the page once the worker is up."
              />
            )}
            {!running && status && <StartCard status={status} onStart={() => void start()} />}
            {running && !configured && status && (
              <ConfigureCard status={status} onConfigure={configure} disabled={Boolean(busy)} />
            )}
            {running && configured && proxyUrl && (
              <iframe className="aspire-dashboard-frame" src={proxyUrl} title="Aspire Dashboard" data-autofocus="" />
            )}
          </>
        )}
      </PageMain>
    </PageShell>
  )
}

function StartCard({ status, onStart }: { status: Status; onStart: () => void }) {
  return (
    <Card className="aspire-dashboard-card">
      <CardHeader>
        <div>
          <h2>Start the dashboard</h2>
          <p>
            The worker runs the standalone Aspire Dashboard as a local process and publishes standard OTLP ports. The
            ports bind to loopback and OTLP ingestion is unsecured by default, because iii-observability cannot send an
            OTLP API key.
          </p>
        </div>
      </CardHeader>
      <CardBody>
        {status.dashboard.last_error && (
          <StatusPanel variant="alert" headline="Last start failed" detail={status.dashboard.last_error} />
        )}
        <dl className="aspire-dashboard-facts">
          <div>
            <dt>Web UI</dt>
            <dd>{status.dashboard.dashboard_url}</dd>
          </div>
          <div>
            <dt>Console proxy</dt>
            <dd>{status.dashboard.proxy_url}</dd>
          </div>
          <div>
            <dt>OTLP/gRPC</dt>
            <dd>{status.dashboard.otlp_endpoint}</dd>
          </div>
          <div>
            <dt>OTLP/HTTP</dt>
            <dd>{status.dashboard.otlp_http_endpoint}</dd>
          </div>
          <div>
            <dt>OTLP security</dt>
            <dd>{status.dashboard.otlp_secure ? 'API key required' : 'disabled'}</dd>
          </div>
        </dl>
        <Button variant="primary" data-autofocus="" onClick={onStart}>
          Start Aspire Dashboard
        </Button>
      </CardBody>
    </Card>
  )
}

function ConfigureCard({
  status,
  onConfigure,
  disabled,
}: {
  status: Status
  onConfigure: (includeMetrics: boolean) => void
  disabled: boolean
}) {
  const endpoint = status.dashboard.otlp_endpoint
  const obs = status.observability
  return (
    <div className="aspire-dashboard-setup">
      <StatusPanel
        variant="warn"
        headline="iii-observability is not exporting to Aspire Dashboard yet"
        detail="Aspire is running, but iii-observability must point its OTLP/gRPC exporter at this worker before data appears. Those settings are restart-tier, so the engine must restart after the change."
      />
      <Card className="aspire-dashboard-card">
        <CardHeader>
          <div>
            <h2>One-click configuration</h2>
            <p>These buttons call configuration::set for iii-observability. Nothing changes until you click.</p>
          </div>
        </CardHeader>
        <CardBody>
          <div className="aspire-dashboard-actions">
            <Button variant="primary" disabled={disabled || !obs.registered} onClick={() => onConfigure(false)}>
              Export traces and logs
            </Button>
            <Button variant="ghost" disabled={disabled || !obs.registered} onClick={() => onConfigure(true)}>
              Export traces, logs, and metrics
            </Button>
          </div>
          <p className="aspire-dashboard-note">
            Traces and logs use <code>both</code>, so iii-observability keeps its local stores while also exporting to
            Aspire. Metrics are different in the current iii-observability schema: <code>metrics_exporter</code> is
            either <code>memory</code> or{' '}
            <code>otlp</code>. Choose the metrics button only if you want live metrics in Aspire and accept switching
            iii-observability metrics away from local memory storage.
          </p>
          {obs.warnings.length > 0 && (
            <ul className="aspire-dashboard-warnings">
              {obs.warnings.map((warning) => (
                <li key={warning}>{warning}</li>
              ))}
            </ul>
          )}
        </CardBody>
      </Card>
      <Card className="aspire-dashboard-card">
        <CardHeader>
          <div>
            <h2>Manual steps</h2>
            <p>
              Run this command from a terminal connected to the same iii engine to preserve local trace/log storage and
              export them over OTLP/gRPC. It writes the same fields as the one-click button. Restart the engine
              afterwards, because <code>endpoint</code> and <code>exporter</code> only apply at the next start.
            </p>
          </div>
        </CardHeader>
        <CardBody>
          <CodeHighlight code={configExample(endpoint)} language="shell" wrap />
          <p className="aspire-dashboard-note">
            For metrics too, set <code>metrics_enabled: true</code> and <code>metrics_exporter: "otlp"</code>; the
            current worker UI exposes that as the secondary button above.
          </p>
        </CardBody>
      </Card>
      <Card className="aspire-dashboard-card">
        <CardHeader>
          <h2>Current iii-observability state</h2>
        </CardHeader>
        <CardBody>
          <dl className="aspire-dashboard-facts">
            <div>
              <dt>Configured endpoint</dt>
              <dd>{obs.endpoint ?? 'not set'}</dd>
            </div>
            <div>
              <dt>Desired endpoint</dt>
              <dd>{obs.desired_endpoint}</dd>
            </div>
            <div>
              <dt>Traces exporter</dt>
              <dd>{obs.traces_exporter ?? 'not set'}</dd>
            </div>
            <div>
              <dt>Logs exporter</dt>
              <dd>{obs.logs_exporter ?? 'not set'}</dd>
            </div>
            <div>
              <dt>OTLP API key</dt>
              <dd>
                {obs.otlp_secure ? (obs.otlp_api_key_configured ? 'configured' : 'not configured') : 'not required'}
              </dd>
            </div>
            <div>
              <dt>Metrics exporter</dt>
              <dd>{obs.metrics_exporter ?? 'not set'}</dd>
            </div>
          </dl>
        </CardBody>
      </Card>
    </div>
  )
}
