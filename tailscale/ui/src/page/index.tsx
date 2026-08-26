import {
  Badge,
  Button,
  Card,
  CardBody,
  CardHeader,
  Chip,
  ConfirmDialog,
  type Host,
  IconButton,
  Input,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  SegmentedControl,
  StatusDot,
  StatusPanel,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useState } from 'react'
import { Check, Copy, ExternalLink, Globe, QrCode, RefreshCw, Square } from './icons'

type Props = PageRenderProps & { host: Host }

type Mode = 'serve' | 'funnel'

type Route = { mode: Mode; host: string; port: number; path: string; target: string; url: string }

type Status = {
  installed: boolean
  version?: string | null
  backend_state?: string | null
  online: boolean
  hostname?: string | null
  dns_name?: string | null
  tailscale_ips: string[]
  health: string[]
  funnel_allowed: boolean
  routes: Route[]
  error?: string | null
}

type Configuration = { allow_funnel: boolean; default_https_port: number; console_url: string }

type Share = {
  stage: 'authorization_required' | 'ready'
  mode: Mode
  public: boolean
  url: string
  qr_svg: string
  authorization_url?: string | null
  target: string
  https_port: number
  path: string
}

const modeOptions = [
  { value: 'serve' as const, label: 'Tailnet only', icon: false as const },
  { value: 'funnel' as const, label: 'Public internet', icon: false as const },
]

function describe(cause: unknown): string {
  const text = (() => {
    if (cause instanceof Error) return cause.message
    if (cause && typeof cause === 'object') {
      const message = (cause as { message?: unknown }).message
      if (typeof message === 'string') return message
      try {
        return JSON.stringify(cause)
      } catch {
        return String(cause)
      }
    }
    return String(cause)
  })()
  return text.replace(/^handler error:\s*/i, '')
}

function qrDataUrl(svg: string) {
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`
}

export function TailscalePage({ host, onRequestClose, commands }: Props) {
  const [status, setStatus] = useState<Status | null>(null)
  const [configuration, setConfiguration] = useState<Configuration | null>(null)
  const [share, setShare] = useState<Share | null>(null)
  const [mode, setMode] = useState<Mode>('serve')
  const [port, setPort] = useState('443')
  const [path, setPath] = useState('/')
  const [confirming, setConfirming] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)

  const refresh = useCallback(async () => {
    try {
      const [nextStatus, nextConfiguration] = await Promise.all([
        host.iii.trigger<Status>('tailscale::status', {}),
        host.iii.trigger<Configuration>('tailscale::configuration', {}),
      ])
      setStatus(nextStatus)
      setConfiguration(nextConfiguration)
      setPort((current) => (current === '443' ? String(nextConfiguration.default_https_port) : current))
      setError(null)
    } catch (cause) {
      setError(describe(cause))
    }
  }, [host])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const createShare = useCallback(
    async (confirmPublic: boolean) => {
      setBusy(true)
      setError(null)
      try {
        const next = await host.iii.trigger<Share>('tailscale::share', {
          mode,
          https_port: Number(port),
          path,
          confirm_public: mode === 'funnel' && confirmPublic,
        })
        setShare(next)
        await refresh()
      } catch (cause) {
        setError(describe(cause))
      } finally {
        setBusy(false)
      }
    },
    [host, mode, port, path, refresh],
  )

  const requestShare = useCallback(() => {
    if (mode === 'funnel') setConfirming(true)
    else void createShare(false)
  }, [mode, createShare])

  const stopShare = useCallback(async () => {
    if (!share || share.stage !== 'ready') return
    setBusy(true)
    setError(null)
    try {
      await host.iii.trigger('tailscale::share::stop', {
        mode: share.mode,
        https_port: share.https_port,
        path: share.path,
      })
      setShare(null)
      await refresh()
    } catch (cause) {
      setError(describe(cause))
    } finally {
      setBusy(false)
    }
  }, [host, share, refresh])

  const [loginUrl, setLoginUrl] = useState<string | null>(null)
  const [stoppingRoute, setStoppingRoute] = useState<string | null>(null)

  const connect = useCallback(async () => {
    setBusy(true)
    setError(null)
    try {
      const result = await host.iii.trigger<{ connected: boolean; authorization_url?: string | null }>('tailscale::connect', {})
      setLoginUrl(result.authorization_url ?? null)
      await refresh()
    } catch (cause) {
      setError(describe(cause))
    } finally {
      setBusy(false)
    }
  }, [host, refresh])

  const disconnect = useCallback(async () => {
    setBusy(true)
    setError(null)
    try {
      await host.iii.trigger('tailscale::disconnect', {})
      setShare(null)
      await refresh()
    } catch (cause) {
      setError(describe(cause))
    } finally {
      setBusy(false)
    }
  }, [host, refresh])

  const stopRoute = useCallback(
    async (route: Route) => {
      const key = `${route.host}:${route.port}${route.path}`
      setStoppingRoute(key)
      setError(null)
      try {
        await host.iii.trigger('tailscale::share::stop', { mode: route.mode, https_port: route.port, path: route.path })
        if (share && share.https_port === route.port && share.path === route.path) setShare(null)
        await refresh()
      } catch (cause) {
        setError(describe(cause))
      } finally {
        setStoppingRoute(null)
      }
    },
    [host, share, refresh],
  )

  const copyLink = useCallback(async () => {
    if (!share) return
    await navigator.clipboard.writeText(share.url)
    setCopied(true)
    window.setTimeout(() => setCopied(false), 1600)
  }, [share])

  const openLink = useCallback(() => {
    if (share) window.open(share.url, '_blank', 'noopener')
  }, [share])

  const online = status?.online ?? false
  const funnelLocked = mode === 'funnel' && configuration !== null && !configuration.allow_funnel
  const canCreate = online && !busy && !funnelLocked && Number.isInteger(Number(port)) && Number(port) > 0
  const authorizationRequired = share?.stage === 'authorization_required'
  const routeLive = share?.stage === 'ready'

  useEffect(
    () =>
      commands?.register([
        { id: 'refresh', title: 'Refresh status', shortcut: 'R', run: () => void refresh() },
        { id: 'create', title: 'Create link', shortcut: 'N', enabled: () => canCreate, run: requestShare },
        { id: 'copy', title: 'Copy link', shortcut: 'C', enabled: () => share !== null, run: () => void copyLink() },
        { id: 'open', title: 'Open link in a browser tab', shortcut: 'O', enabled: () => share !== null, run: openLink },
        { id: 'stop', title: 'Stop route', shortcut: 'X', enabled: () => routeLive && !busy, run: () => void stopShare() },
      ]),
    [commands, refresh, requestShare, copyLink, openLink, stopShare, canCreate, share, routeLive, busy],
  )

  const connectionLabel = !status
    ? 'Checking'
    : !status.installed
      ? 'Tailscale CLI not found'
      : online
        ? 'Connected'
        : (status.backend_state ?? 'Not running')

  return (
    <PageShell className="ts-shell">
      <PageHeader
        icon={<Globe />}
        title="Tailscale"
        description={
          status?.dns_name ? (
            <>
              <span className="ts-mono">{status.dns_name}</span>
              {status.routes.length > 0 && ` · ${status.routes.length} ${status.routes.length === 1 ? 'route' : 'routes'} live`}
            </>
          ) : (
            'Share the Console over your tailnet'
          )
        }
        actions={
          <IconButton label="Refresh status" variant="ghost" onClick={() => void refresh()}>
            <RefreshCw />
          </IconButton>
        }
        onClose={onRequestClose}
      />
      <PageBody>
        <PageMain className="ts-main">
          {error && <StatusPanel variant="alert" headline="Tailscale request failed" detail={error} />}
          {status?.error && !error && <StatusPanel variant="warn" headline="Tailscale is not available" detail={status.error} />}
          {status && status.installed && !status.error && !online && (
            <StatusPanel
              variant="warn"
              headline="Tailscale is not connected"
              detail={
                status.health.length
                  ? status.health.join(' · ')
                  : `The client reports ${status.backend_state ?? 'no state'}. Connect Tailscale on this machine, then refresh.`
              }
            />
          )}

          <div className="ts-columns">
            <Card className="ts-card">
              <CardHeader className="ts-card-header">
                <span className="ts-card-title">
                  <StatusDot tone={online ? 'accent' : 'warn'} pulse={online} />
                  Connection
                </span>
                <Badge variant={online ? 'ok' : 'warn'}>{connectionLabel}</Badge>
              </CardHeader>
              <CardBody>
                <dl className="ts-facts">
                  <div>
                    <dt>Device</dt>
                    <dd>{status?.hostname ?? '–'}</dd>
                  </div>
                  <div>
                    <dt>MagicDNS</dt>
                    <dd className="ts-mono">{status?.dns_name ?? '–'}</dd>
                  </div>
                  <div>
                    <dt>Tailscale IPs</dt>
                    <dd className="ts-mono">{status?.tailscale_ips.length ? status.tailscale_ips.join(' · ') : '–'}</dd>
                  </div>
                  <div>
                    <dt>Version</dt>
                    <dd className="ts-mono">{status?.version ?? '–'}</dd>
                  </div>
                  <div>
                    <dt>Funnel</dt>
                    <dd>
                      {status?.funnel_allowed ? 'Enabled for this node' : 'Not enabled for this node'}
                      {configuration && !configuration.allow_funnel ? ' · locked in the worker configuration' : ''}
                    </dd>
                  </div>
                  {status?.health.length ? (
                    <div>
                      <dt>Health</dt>
                      <dd>{status.health.join(' · ')}</dd>
                    </div>
                  ) : null}
                </dl>
                {loginUrl && (
                  <StatusPanel
                    variant="info"
                    headline="This node needs a Tailscale sign-in"
                    detail="Open the sign-in page, finish the login, then connect again."
                  />
                )}
                <div className="ts-actions">
                  {status?.installed && !online && (
                    <Button variant="primary" disabled={busy} onClick={() => void connect()}>
                      {busy ? 'Connecting…' : 'Connect to tailnet'}
                    </Button>
                  )}
                  {loginUrl && (
                    <Button variant="ghost" onClick={() => window.open(loginUrl, '_blank', 'noopener')}>
                      Open Tailscale sign-in
                    </Button>
                  )}
                  {online && (
                    <Button variant="ghost" disabled={busy} onClick={() => void disconnect()}>
                      {busy ? 'Working…' : 'Disconnect from tailnet'}
                    </Button>
                  )}
                </div>
              </CardBody>
            </Card>

            <Card className="ts-card">
              <CardHeader className="ts-card-header">
                <span className="ts-card-title">Share the Console</span>
              </CardHeader>
              <CardBody className="ts-form">
                <SegmentedControl variant="radio" aria-label="Visibility" value={mode} onChange={setMode} options={modeOptions} />
                <div className="ts-fields">
                  <label className="ts-field">
                    <span>HTTPS port</span>
                    <Input value={port} onChange={setPort} inputMode="numeric" aria-label="HTTPS port" />
                  </label>
                  <label className="ts-field">
                    <span>Path</span>
                    <Input value={path} onChange={setPath} aria-label="Path" />
                  </label>
                </div>
                <p className="ts-note">
                  {mode === 'serve'
                    ? 'Opens on any of your devices that are signed into this tailnet (phone or laptop with the Tailscale app), with no extra login. Each request carries Tailscale identity headers.'
                    : 'For devices that are not on your tailnet. Anyone with the link can open the Console; Funnel uses port 443, 8443, or 10000 and carries no identity headers.'}
                </p>
                {funnelLocked && (
                  <StatusPanel
                    variant="warn"
                    headline="Funnel is locked"
                    detail="Set allow_funnel to true in the tailscale worker configuration to publish the Console publicly."
                  />
                )}
                <div className="ts-actions">
                  <Button variant="primary" disabled={!canCreate} data-autofocus="" onClick={requestShare}>
                    {busy ? 'Working…' : mode === 'funnel' ? 'Create public link…' : 'Create link'}
                  </Button>
                </div>
              </CardBody>
            </Card>
          </div>

          {share && (
            <Card className="ts-card ts-link-card">
              <CardHeader className="ts-card-header">
                <span className="ts-card-title">
                  <QrCode />
                  {authorizationRequired ? 'Public link not available yet' : 'Open on another device'}
                </span>
                <Chip tone={share.public ? 'danger' : authorizationRequired ? 'warning' : 'neutral'}>
                  {authorizationRequired ? 'Admin step' : share.public ? 'Public' : 'Tailnet'}
                </Chip>
              </CardHeader>
              <CardBody className="ts-link-body">
                {authorizationRequired ? (
                  <>
                    <StatusPanel
                      variant="warn"
                      headline="Your tailnet does not allow Funnel on this device"
                      detail="Tailscale requires a tailnet admin to allow it once. Open Tailscale, approve Funnel for this device, then check again. If the link is only for your own devices, use Tailnet only instead; it needs no approval."
                    />
                    <div className="ts-actions">
                      <Button variant="primary" onClick={openLink}>
                        Approve in Tailscale
                      </Button>
                      <Button variant="ghost" disabled={busy} onClick={() => void createShare(true)}>
                        {busy ? 'Checking…' : 'Check again'}
                      </Button>
                      <Button
                        variant="ghost"
                        onClick={() => {
                          setShare(null)
                          setMode('serve')
                        }}
                      >
                        Use Tailnet only
                      </Button>
                    </div>
                  </>
                ) : (
                  <>
                    <img className="ts-qr" alt={`QR code for ${share.url}`} src={qrDataUrl(share.qr_svg)} />
                    <code className="ts-url">{share.url}</code>
                    <p className="ts-note">
                      {share.public
                        ? 'Opens for anyone with the link.'
                        : 'Opens on any device signed into your tailnet; other devices are refused by Tailscale.'}
                    </p>
                    <div className="ts-actions">
                      <IconButton label={copied ? 'Copied' : 'Copy link'} variant="ghost" onClick={() => void copyLink()}>
                        {copied ? <Check /> : <Copy />}
                      </IconButton>
                      <IconButton label="Open link in a browser tab" variant="ghost" onClick={openLink}>
                        <ExternalLink />
                      </IconButton>
                      <IconButton label="Stop route" variant="ghost" disabled={busy} onClick={() => void stopShare()}>
                        <Square />
                      </IconButton>
                    </div>
                  </>
                )}
              </CardBody>
            </Card>
          )}

          <Card className="ts-card">
            <CardHeader className="ts-card-header">
              <span className="ts-card-title">Active routes</span>
              <Chip tone="neutral">{status?.routes.length ?? 0}</Chip>
            </CardHeader>
            <CardBody>
              {status?.routes.length ? (
                <TableViewport>
                  <TableFrame>
                    <Table density="compact">
                      <TableHeader>
                        <TableRow>
                          <TableHead>Visibility</TableHead>
                          <TableHead>URL</TableHead>
                          <TableHead>Target</TableHead>
                          <TableHead className="ts-row-actions-head">
                            <span className="ts-visually-hidden">Actions</span>
                          </TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {status.routes.map((route) => {
                          const key = `${route.host}:${route.port}${route.path}`
                          return (
                            <TableRow key={key}>
                              <TableCell>{route.mode === 'funnel' ? 'Public' : 'Tailnet'}</TableCell>
                              <TableCell className="ts-mono">{route.url}</TableCell>
                              <TableCell className="ts-mono">{route.target}</TableCell>
                              <TableCell className="ts-row-actions">
                                <IconButton
                                  label={`Stop ${route.url}`}
                                  variant="ghost"
                                  disabled={stoppingRoute !== null}
                                  aria-busy={stoppingRoute === key}
                                  onClick={() => void stopRoute(route)}
                                >
                                  <Square />
                                </IconButton>
                              </TableCell>
                            </TableRow>
                          )
                        })}
                      </TableBody>
                    </Table>
                  </TableFrame>
                </TableViewport>
              ) : (
                <p className="ts-note">No routes are published. Routes the worker did not create are never reset.</p>
              )}
            </CardBody>
          </Card>
        </PageMain>
      </PageBody>

      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title="Publish the Console to the internet?"
        description="Anyone with the link can open this Console until the route is stopped. Funnel carries no Tailscale identity headers, so the Console cannot tell who is connecting."
        confirmLabel="Publish publicly"
        onConfirm={() => {
          setConfirming(false)
          void createShare(true)
        }}
      />
    </PageShell>
  )
}
