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
  Select,
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
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { Activity, Check, Copy, ExternalLink, Globe, QrCode, RefreshCw, Send, Square } from './icons'

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
  magic_dns_suffix?: string | null
  tailnet?: string | null
  tailscale_ips: string[]
  health: string[]
  funnel_allowed: boolean
  peer_count: number
  online_peer_count: number
  exit_node?: string | null
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

type Peer = {
  id: string
  hostname: string
  dns_name: string
  os?: string | null
  tailscale_ips: string[]
  online: boolean
  active: boolean
  exit_node: boolean
  exit_node_option: boolean
  tags: string[]
  last_seen?: string | null
  relay?: string | null
  rx_bytes: number
  tx_bytes: number
  taildrop_target: boolean
  ingress: boolean
}

type Ping = { target: string; direct: boolean; replies: { via: string; latency_ms?: number | null }[] }

type Netcheck = {
  udp: boolean
  ipv4: boolean
  ipv6: boolean
  mapping_varies_by_dest_ip?: boolean | null
  upnp?: boolean | null
  pmp?: boolean | null
  pcp?: boolean | null
  preferred_derp?: number | null
  region_latency_ms: { region: number; latency_ms: number }[]
  global_v4?: string | null
  global_v6?: string | null
  captive_portal?: boolean | null
}

type DnsStatus = {
  magic_dns: boolean
  magic_dns_suffix?: string | null
  resolvers: string[]
  search_domains: string[]
  split_dns_routes: { domain: string; resolvers: string[] }[]
  cert_domains: string[]
}

type Prefs = {
  accept_routes: boolean
  accept_dns: boolean
  exit_node_id?: string | null
  exit_node_allow_lan_access: boolean
  ssh: boolean
  webclient: boolean
  shields_up: boolean
  hostname?: string | null
  advertise_routes: string[]
  advertise_exit_node: boolean
  auto_update_check: boolean
  auto_update_apply: boolean
}

type ExitNodes = { exit_nodes: Peer[]; current?: string | null }

type FileTarget = { ip: string; name: string }

type Account = { id: string; account: string; tailnet: string; nickname?: string | null; selected: boolean }

type LockStatus = { enabled: boolean; node_key?: string | null; node_signed?: boolean | null }

type Version = { version: string; long?: string | null; os_variant?: string | null; upstream?: string | null }

type Tab = 'share' | 'devices' | 'network' | 'settings' | 'files' | 'account'

const modeOptions = [
  { value: 'serve' as const, label: 'Tailnet only', icon: false as const },
  { value: 'funnel' as const, label: 'Public internet', icon: false as const },
]

const conflictOptions = [
  { value: 'skip', label: 'Skip existing files' },
  { value: 'overwrite', label: 'Overwrite existing files' },
  { value: 'rename', label: 'Rename incoming files' },
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

function bytes(n: number) {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`
}

function yesNo(value: boolean | null | undefined) {
  if (value == null) return 'unknown'
  return value ? 'yes' : 'no'
}

export function TailscalePage({ host, onRequestClose, commands }: Props) {
  const [tab, setTab] = useState<Tab>('share')
  const [status, setStatus] = useState<Status | null>(null)
  const [configuration, setConfiguration] = useState<Configuration | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const [loginUrl, setLoginUrl] = useState<string | null>(null)

  const trigger = useCallback(
    <T,>(fn: string, payload: Record<string, unknown> = {}, timeoutMs = 60_000) =>
      host.iii.trigger<T>(fn, payload, { timeoutMs }),
    [host],
  )

  const [refreshing, setRefreshing] = useState(false)
  const [updatedAt, setUpdatedAt] = useState<number | null>(null)
  const [, setTick] = useState(0)

  const refresh = useCallback(async () => {
    setRefreshing(true)
    try {
      const [nextStatus, nextConfiguration] = await Promise.all([
        trigger<Status>('tailscale::status'),
        trigger<Configuration>('tailscale::configuration'),
      ])
      setStatus(nextStatus)
      setConfiguration(nextConfiguration)
      setUpdatedAt(Date.now())
      setError(null)
    } catch (cause) {
      setError(describe(cause))
    } finally {
      setRefreshing(false)
    }
  }, [trigger])

  useEffect(() => {
    if (updatedAt === null) return
    const timer = window.setInterval(() => setTick((t) => t + 1), 15_000)
    return () => window.clearInterval(timer)
  }, [updatedAt])

  const updatedLabel = (() => {
    if (updatedAt === null) return null
    const seconds = Math.max(0, Math.round((Date.now() - updatedAt) / 1000))
    if (seconds < 10) return 'updated just now'
    if (seconds < 60) return `updated ${seconds}s ago`
    return `updated ${Math.round(seconds / 60)} min ago`
  })()

  useEffect(() => {
    void refresh()
  }, [refresh])

  const act = useCallback(
    async (work: () => Promise<string | null | undefined>) => {
      setBusy(true)
      setError(null)
      setNotice(null)
      try {
        const message = await work()
        if (message) setNotice(message)
        await refresh()
      } catch (cause) {
        setError(describe(cause))
      } finally {
        setBusy(false)
      }
    },
    [refresh],
  )

  const online = status?.online ?? false

  const connect = useCallback(
    () =>
      act(async () => {
        const result = await trigger<{ connected: boolean; authorization_url?: string | null }>('tailscale::connect')
        setLoginUrl(result.authorization_url ?? null)
        return result.connected ? 'Connected to the tailnet.' : null
      }),
    [act, trigger],
  )

  const disconnect = useCallback(
    () =>
      act(async () => {
        await trigger('tailscale::disconnect')
        return 'Disconnected from the tailnet.'
      }),
    [act, trigger],
  )

  const connectionLabel = !status
    ? 'Checking'
    : !status.installed
      ? 'Tailscale CLI not found'
      : online
        ? 'Connected'
        : (status.backend_state ?? 'Not running')

  const [share, setShare] = useState<Share | null>(null)
  const [mode, setMode] = useState<Mode>('serve')
  const [port, setPort] = useState('443')
  const [path, setPath] = useState('/')
  const [target, setTarget] = useState('')
  const [confirming, setConfirming] = useState(false)
  const [copied, setCopied] = useState(false)
  const [stoppingRoute, setStoppingRoute] = useState<string | null>(null)

  useEffect(() => {
    if (configuration && port === '443') setPort(String(configuration.default_https_port))
  }, [configuration, port])

  const funnelLocked = mode === 'funnel' && configuration !== null && !configuration.allow_funnel
  const canCreate = online && !busy && !funnelLocked && Number.isInteger(Number(port)) && Number(port) > 0
  const authorizationRequired = share?.stage === 'authorization_required'
  const routeLive = share?.stage === 'ready'

  const createShare = useCallback(
    (confirmPublic: boolean) =>
      act(async () => {
        const custom = target.trim()
        const payload = {
          mode,
          https_port: Number(port),
          path,
          confirm_public: mode === 'funnel' && confirmPublic,
          ...(custom ? { target: custom } : {}),
        }
        const next = await trigger<Share>(custom ? 'tailscale::serve::add' : 'tailscale::share', payload)
        setShare(next)
        return null
      }),
    [act, trigger, mode, port, path, target],
  )

  const requestShare = useCallback(() => {
    if (mode === 'funnel') setConfirming(true)
    else void createShare(false)
  }, [mode, createShare])

  const stopRoute = useCallback(
    (route: Route) =>
      act(async () => {
        const key = `${route.host}:${route.port}${route.path}`
        setStoppingRoute(key)
        try {
          const custom = share && share.https_port === route.port && share.path === route.path ? null : route
          await trigger(custom ? 'tailscale::serve::remove' : 'tailscale::share::stop', {
            mode: route.mode,
            https_port: route.port,
            path: route.path,
          })
          if (share && share.https_port === route.port && share.path === route.path) setShare(null)
        } finally {
          setStoppingRoute(null)
        }
        return route.mode === 'funnel' ? 'Public access removed; the tailnet route stays.' : 'Route stopped.'
      }),
    [act, trigger, share],
  )

  const stopShare = useCallback(() => {
    if (!share || share.stage !== 'ready') return
    const route = status?.routes.find((r) => r.port === share.https_port && r.path === share.path)
    if (route) void stopRoute(route)
  }, [share, status, stopRoute])

  const copyLink = useCallback(async () => {
    if (!share) return
    await navigator.clipboard.writeText(share.url)
    setCopied(true)
    window.setTimeout(() => setCopied(false), 1600)
  }, [share])

  const openLink = useCallback(() => {
    if (share) window.open(share.url, '_blank', 'noopener')
  }, [share])

  const [peers, setPeers] = useState<Peer[] | null>(null)
  const [onlineOnly, setOnlineOnly] = useState(false)
  const [includeIngress, setIncludeIngress] = useState(false)
  const [hiddenIngress, setHiddenIngress] = useState(0)
  const [pings, setPings] = useState<Record<string, Ping | 'pending' | string>>({})

  const loadPeers = useCallback(
    async (include = includeIngress) => {
      try {
        const result = await trigger<{ peers: Peer[]; hidden_ingress_count: number }>('tailscale::peers::list', {
          include_ingress: include,
        })
        setPeers(result.peers)
        setHiddenIngress(result.hidden_ingress_count)
      } catch (cause) {
        setError(describe(cause))
      }
    },
    [trigger, includeIngress],
  )

  const toggleIngress = useCallback(() => {
    const next = !includeIngress
    setIncludeIngress(next)
    void loadPeers(next)
  }, [includeIngress, loadPeers])

  useEffect(() => {
    if (tab === 'devices' && peers === null) void loadPeers()
  }, [tab, peers, loadPeers])

  const pingPeer = useCallback(
    async (peer: Peer) => {
      setPings((prev) => ({ ...prev, [peer.id]: 'pending' }))
      try {
        const result = await trigger<Ping>('tailscale::ping', { target: peer.dns_name || peer.tailscale_ips[0], count: 3 })
        setPings((prev) => ({ ...prev, [peer.id]: result }))
      } catch (cause) {
        setPings((prev) => ({ ...prev, [peer.id]: describe(cause) }))
      }
    },
    [trigger],
  )

  const [netcheck, setNetcheck] = useState<Netcheck | null>(null)
  const [dns, setDns] = useState<DnsStatus | null>(null)
  const [exitNodes, setExitNodes] = useState<ExitNodes | null>(null)
  const [exitChoice, setExitChoice] = useState<string>('')
  const [suggestion, setSuggestion] = useState<string | null>(null)

  const loadNetwork = useCallback(async () => {
    try {
      const [nextDns, nextExit] = await Promise.all([
        trigger<DnsStatus>('tailscale::dns::status'),
        trigger<ExitNodes>('tailscale::exit-node::list'),
      ])
      setDns(nextDns)
      setExitNodes(nextExit)
      setExitChoice(nextExit.current ?? '')
    } catch (cause) {
      setError(describe(cause))
    }
  }, [trigger])

  useEffect(() => {
    if (tab === 'network' && dns === null) void loadNetwork()
  }, [tab, dns, loadNetwork])

  const runNetcheck = useCallback(
    () =>
      act(async () => {
        setNetcheck(await trigger<Netcheck>('tailscale::netcheck', {}, 90_000))
        return null
      }),
    [act, trigger],
  )

  const applyExitNode = useCallback(
    () =>
      act(async () => {
        await trigger('tailscale::exit-node::set', { exit_node: exitChoice })
        await loadNetwork()
        return exitChoice ? `Routing internet traffic through ${exitChoice}.` : 'Exit node cleared.'
      }),
    [act, trigger, exitChoice, loadNetwork],
  )

  const suggestExitNode = useCallback(
    () =>
      act(async () => {
        const result = await trigger<{ suggestion?: string | null; message: string }>('tailscale::exit-node::suggest')
        setSuggestion(result.suggestion ?? result.message)
        if (result.suggestion) setExitChoice(result.suggestion)
        return null
      }),
    [act, trigger],
  )

  const [prefs, setPrefs] = useState<Prefs | null>(null)
  const [hostnameDraft, setHostnameDraft] = useState('')
  const [routesDraft, setRoutesDraft] = useState('')

  const loadPrefs = useCallback(async () => {
    try {
      const next = await trigger<Prefs>('tailscale::prefs::get')
      setPrefs(next)
      setHostnameDraft(next.hostname ?? '')
      setRoutesDraft(next.advertise_routes.join(', '))
    } catch (cause) {
      setError(describe(cause))
    }
  }, [trigger])

  useEffect(() => {
    if (tab === 'settings' && prefs === null) void loadPrefs()
  }, [tab, prefs, loadPrefs])

  const setPref = useCallback(
    (patch: Record<string, unknown>, message: string) =>
      act(async () => {
        setPrefs(await trigger<Prefs>('tailscale::prefs::set', patch))
        return message
      }),
    [act, trigger],
  )

  const [fileTargets, setFileTargets] = useState<FileTarget[] | null>(null)
  const [sendPath, setSendPath] = useState('')
  const [sendTarget, setSendTarget] = useState<string | undefined>(undefined)
  const [receiveDir, setReceiveDir] = useState('')
  const [conflict, setConflict] = useState('skip')
  const [certDomain, setCertDomain] = useState<string | undefined>(undefined)
  const [certDir, setCertDir] = useState('')

  const loadFiles = useCallback(async () => {
    try {
      const [targets, nextDns] = await Promise.all([
        trigger<{ targets: FileTarget[] }>('tailscale::file::targets'),
        dns ? Promise.resolve(dns) : trigger<DnsStatus>('tailscale::dns::status'),
      ])
      setFileTargets(targets.targets)
      setDns(nextDns)
      if (!certDomain && nextDns.cert_domains[0]) setCertDomain(nextDns.cert_domains[0])
    } catch (cause) {
      setError(describe(cause))
    }
  }, [trigger, dns, certDomain])

  useEffect(() => {
    if (tab === 'files' && fileTargets === null) void loadFiles()
  }, [tab, fileTargets, loadFiles])

  const sendFile = useCallback(
    () =>
      act(async () => {
        const result = await trigger<{ output: string }>('tailscale::file::send', {
          paths: [sendPath.trim()],
          target: sendTarget,
        })
        return result.output || `Sent ${sendPath.trim()} to ${sendTarget}.`
      }),
    [act, trigger, sendPath, sendTarget],
  )

  const receiveFiles = useCallback(
    () =>
      act(async () => {
        const result = await trigger<{ output: string }>('tailscale::file::receive', {
          directory: receiveDir.trim(),
          conflict,
        })
        return result.output || `Inbox moved into ${receiveDir.trim()}.`
      }),
    [act, trigger, receiveDir, conflict],
  )

  const fetchCert = useCallback(
    () =>
      act(async () => {
        const dir = certDir.trim().replace(/\/$/, '')
        const result = await trigger<{ cert_file: string; key_file: string }>(
          'tailscale::cert',
          { domain: certDomain, cert_file: `${dir}/${certDomain}.crt`, key_file: `${dir}/${certDomain}.key` },
          120_000,
        )
        return `Wrote ${result.cert_file} and ${result.key_file}.`
      }),
    [act, trigger, certDomain, certDir],
  )

  const [accounts, setAccounts] = useState<Account[] | null>(null)
  const [lock, setLock] = useState<LockStatus | null>(null)
  const [version, setVersion] = useState<Version | null>(null)

  const loadAccount = useCallback(async () => {
    try {
      const [nextAccounts, nextLock, nextVersion] = await Promise.all([
        trigger<{ accounts: Account[] }>('tailscale::accounts::list'),
        trigger<LockStatus>('tailscale::lock::status'),
        trigger<Version>('tailscale::version', { check_upstream: true }),
      ])
      setAccounts(nextAccounts.accounts)
      setLock(nextLock)
      setVersion(nextVersion)
    } catch (cause) {
      setError(describe(cause))
    }
  }, [trigger])

  useEffect(() => {
    if (tab === 'account' && accounts === null) void loadAccount()
  }, [tab, accounts, loadAccount])

  const switchAccount = useCallback(
    (id: string) =>
      act(async () => {
        const result = await trigger<{ accounts: Account[] }>('tailscale::accounts::switch', { account: id })
        setAccounts(result.accounts)
        return 'Switched account.'
      }),
    [act, trigger],
  )

  const login = useCallback(
    () =>
      act(async () => {
        const result = await trigger<{ authorization_url?: string | null; connected: boolean }>('tailscale::login', {}, 60_000)
        setLoginUrl(result.authorization_url ?? null)
        return result.authorization_url ? 'Open the sign-in page to finish.' : 'Signed in.'
      }),
    [act, trigger],
  )

  const logout = useCallback(
    () =>
      act(async () => {
        await trigger('tailscale::logout')
        return 'Logged out; the next connect needs a sign-in.'
      }),
    [act, trigger],
  )

  const updateClient = useCallback(
    (dryRun: boolean) =>
      act(async () => {
        const result = await trigger<{ output: string }>('tailscale::update', { dry_run: dryRun }, 300_000)
        return result.output
      }),
    [act, trigger],
  )

  const bugreport = useCallback(
    () =>
      act(async () => {
        const result = await trigger<{ report_id: string; output: string }>('tailscale::bugreport', {}, 60_000)
        return result.report_id ? `Bug report ${result.report_id}` : result.output
      }),
    [act, trigger],
  )

  useEffect(
    () =>
      commands?.register([
        { id: 'refresh', title: 'Refresh status', shortcut: 'R', run: () => void refresh() },
        { id: 'create', title: 'Create link', shortcut: 'N', enabled: () => canCreate, run: requestShare },
        { id: 'copy', title: 'Copy link', shortcut: 'C', enabled: () => share !== null, run: () => void copyLink() },
        { id: 'open', title: 'Open link in a browser tab', shortcut: 'O', enabled: () => share !== null, run: openLink },
        { id: 'stop', title: 'Stop route', shortcut: 'X', enabled: () => routeLive && !busy, run: stopShare },
        { id: 'devices', title: 'Devices', shortcut: 'D', run: () => setTab('devices') },
        { id: 'network', title: 'Network', shortcut: 'W', run: () => setTab('network') },
        { id: 'settings', title: 'Settings', shortcut: 'S', run: () => setTab('settings') },
        { id: 'files', title: 'Files', shortcut: 'F', run: () => setTab('files') },
      ]),
    [commands, refresh, requestShare, copyLink, openLink, stopShare, canCreate, share, routeLive, busy],
  )

  const visiblePeers = useMemo(() => (peers ?? []).filter((peer) => !onlineOnly || peer.online), [peers, onlineOnly])

  const exitOptions = useMemo(
    () => [
      { value: '', label: 'No exit node' },
      { value: 'auto:any', label: 'Best available (auto)' },
      ...(exitNodes?.exit_nodes ?? []).map((peer) => ({ value: peer.dns_name, label: `${peer.hostname} · ${peer.dns_name}` })),
    ],
    [exitNodes],
  )

  const sendOptions = useMemo(() => (fileTargets ?? []).map((t) => ({ value: t.name, label: `${t.name} · ${t.ip}` })), [fileTargets])

  const certOptions = useMemo(() => (dns?.cert_domains ?? []).map((d) => ({ value: d, label: d })), [dns])

  const prefRow = (label: string, value: boolean, key: string, onLabel = 'Turn on', offLabel = 'Turn off') => (
    <div className="ts-pref">
      <span className="ts-pref-label">{label}</span>
      <Chip tone={value ? 'success' : 'neutral'}>{value ? 'On' : 'Off'}</Chip>
      <Button variant="ghost" size="sm" disabled={busy} onClick={() => void setPref({ [key]: !value }, `${label}: ${value ? 'off' : 'on'}.`)}>
        {value ? offLabel : onLabel}
      </Button>
    </div>
  )

  return (
    <PageShell className="ts-shell">
      <PageHeader
        icon={<Globe />}
        title="Tailscale"
        description={
          status?.dns_name ? (
            <>
              <span className="ts-mono">{status.dns_name}</span>
              {status.tailnet ? ` · ${status.tailnet}` : ''}
              {status.peer_count > 0 ? ` · ${status.online_peer_count}/${status.peer_count} devices online` : ''}
            </>
          ) : (
            'Your tailnet from the Console'
          )
        }
        actions={
          <>
            {updatedLabel && <span className="ts-updated">{refreshing ? 'refreshing…' : updatedLabel}</span>}
            <IconButton label="Refresh status" variant="ghost" disabled={refreshing} aria-busy={refreshing} onClick={() => void refresh()}>
              <RefreshCw className={refreshing ? 'ts-spin' : undefined} />
            </IconButton>
          </>
        }
        onClose={onRequestClose}
      />
      <PageBody>
        <PageMain className="ts-main">
          {error && <StatusPanel variant="alert" headline="Tailscale request failed" detail={error} />}
          {notice && !error && <StatusPanel variant="success" headline="Done" detail={notice} />}
          {status?.error && !error && <StatusPanel variant="warn" headline="Tailscale is not available" detail={status.error} />}
          {status && status.installed && !status.error && !online && (
            <StatusPanel
              variant="warn"
              headline="Tailscale is not connected"
              detail={
                status.health.length
                  ? status.health.join(' · ')
                  : `The client reports ${status.backend_state ?? 'no state'}. Connect on this machine, then refresh.`
              }
            />
          )}

          <Card className="ts-card">
            <CardHeader className="ts-card-header">
              <span className="ts-card-title">
                <StatusDot tone={online ? 'accent' : 'warn'} pulse={online} />
                Connection
              </span>
              <Badge variant={online ? 'ok' : 'warn'}>{connectionLabel}</Badge>
            </CardHeader>
            <CardBody>
              <dl className="ts-facts ts-facts-grid">
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
                  <dt>Exit node</dt>
                  <dd>{status?.exit_node ?? 'none'}</dd>
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
                  <div className="ts-facts-wide">
                    <dt>Health</dt>
                    <dd>{status.health.join(' · ')}</dd>
                  </div>
                ) : null}
              </dl>
              {loginUrl && (
                <StatusPanel variant="info" headline="This node needs a Tailscale sign-in" detail="Open the sign-in page, finish the login, then connect again." />
              )}
              <div className="ts-actions">
                {status?.installed && !online && (
                  <Button variant="primary" disabled={busy} onClick={() => void connect()}>
                    {busy ? 'Working…' : 'Connect to tailnet'}
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

          <Tabs value={tab} onValueChange={(next: string) => setTab(next as Tab)}>
            <TabsList variant="line" aria-label="Tailscale sections">
              <TabsTrigger value="share">Share</TabsTrigger>
              <TabsTrigger value="devices">Devices</TabsTrigger>
              <TabsTrigger value="network">Network</TabsTrigger>
              <TabsTrigger value="settings">Settings</TabsTrigger>
              <TabsTrigger value="files">Files</TabsTrigger>
              <TabsTrigger value="account">Account</TabsTrigger>
            </TabsList>

            <TabsContent value="share" className="ts-tab">
              <div className="ts-columns">
                <Card className="ts-card">
                  <CardHeader className="ts-card-header">
                    <span className="ts-card-title">Publish</span>
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
                    <label className="ts-field">
                      <span>What to publish</span>
                      <Input
                        value={target}
                        onChange={setTarget}
                        placeholder={configuration ? `${configuration.console_url} (the Console)` : 'the Console'}
                        aria-label="Target: a local port, loopback URL, or absolute directory; empty publishes the Console"
                      />
                    </label>
                    <p className="ts-note">
                      {mode === 'serve'
                        ? 'Opens on any of your devices that are signed into this tailnet (phone or laptop with the Tailscale app), with no extra login. Each request carries Tailscale identity headers.'
                        : 'For devices that are not on your tailnet. Anyone with the link can open it; Funnel uses port 443, 8443, or 10000 and carries no identity headers.'}
                      {' '}Leave the target empty to publish the Console, or give a local port, loopback URL, or directory.
                    </p>
                    {funnelLocked && (
                      <StatusPanel variant="warn" headline="Funnel is locked" detail="Set allow_funnel to true in the tailscale worker configuration to publish anything publicly." />
                    )}
                    <div className="ts-actions">
                      <Button variant="primary" disabled={!canCreate} data-autofocus="" onClick={requestShare}>
                        {busy ? 'Working…' : mode === 'funnel' ? 'Create public link…' : 'Create link'}
                      </Button>
                    </div>
                  </CardBody>
                </Card>

                <Card className="ts-card ts-link-card">
                  <CardHeader className="ts-card-header">
                    <span className="ts-card-title">
                      <QrCode />
                      {!share ? 'Link' : authorizationRequired ? 'Public link not available yet' : 'Open on another device'}
                    </span>
                    {share && (
                      <Chip tone={share.public ? 'danger' : authorizationRequired ? 'warning' : 'neutral'}>
                        {authorizationRequired ? 'Admin step' : share.public ? 'Public' : 'Tailnet'}
                      </Chip>
                    )}
                  </CardHeader>
                  <CardBody className="ts-link-body">
                    {!share ? (
                      <p className="ts-note">Create a link to see it here with a QR code for a phone.</p>
                    ) : authorizationRequired ? (
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
                          {share.public ? 'Opens for anyone with the link.' : 'Opens on any device signed into your tailnet; other devices are refused by Tailscale.'}
                          {' '}Proxies <span className="ts-mono">{share.target}</span>.
                        </p>
                        <div className="ts-actions">
                          <IconButton label={copied ? 'Copied' : 'Copy link'} variant="ghost" onClick={() => void copyLink()}>
                            {copied ? <Check /> : <Copy />}
                          </IconButton>
                          <IconButton label="Open link in a browser tab" variant="ghost" onClick={openLink}>
                            <ExternalLink />
                          </IconButton>
                          <IconButton label="Stop route" variant="ghost" disabled={busy} onClick={stopShare}>
                            <Square />
                          </IconButton>
                        </div>
                      </>
                    )}
                  </CardBody>
                </Card>
              </div>

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
                                      label={route.mode === 'funnel' ? `Remove public access from ${route.url}` : `Stop ${route.url}`}
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
            </TabsContent>

            <TabsContent value="devices" className="ts-tab">
              <Card className="ts-card">
                <CardHeader className="ts-card-header">
                  <span className="ts-card-title">Devices on the tailnet</span>
                  <div className="ts-actions">
                    <Button variant="ghost" size="sm" onClick={() => setOnlineOnly((v) => !v)}>
                      {onlineOnly ? 'Show all' : 'Online only'}
                    </Button>
                    {(hiddenIngress > 0 || includeIngress) && (
                      <Button variant="ghost" size="sm" onClick={toggleIngress}>
                        {includeIngress ? 'Hide Funnel relays' : `Show ${hiddenIngress} Funnel relays`}
                      </Button>
                    )}
                    <IconButton label="Reload devices" variant="ghost" onClick={() => void loadPeers()}>
                      <RefreshCw />
                    </IconButton>
                  </div>
                </CardHeader>
                <CardBody>
                  {peers === null ? (
                    <p className="ts-note">Loading devices…</p>
                  ) : visiblePeers.length === 0 ? (
                    <p className="ts-note">
                      No devices to show.
                      {hiddenIngress > 0 ? ` ${hiddenIngress} Tailscale Funnel relay nodes are hidden; they are infrastructure, not your devices.` : ''}
                    </p>
                  ) : (
                    <TableViewport>
                      <TableFrame>
                        <Table density="compact">
                          <TableHeader>
                            <TableRow>
                              <TableHead>Device</TableHead>
                              <TableHead>Tailscale IP</TableHead>
                              <TableHead>OS</TableHead>
                              <TableHead>State</TableHead>
                              <TableHead>Path</TableHead>
                              <TableHead>Traffic</TableHead>
                              <TableHead className="ts-row-actions-head">
                                <span className="ts-visually-hidden">Actions</span>
                              </TableHead>
                            </TableRow>
                          </TableHeader>
                          <TableBody>
                            {visiblePeers.map((peer) => {
                              const ping = pings[peer.id]
                              return (
                                <TableRow key={peer.id}>
                                  <TableCell>
                                    <div className="ts-peer-name">
                                      <StatusDot tone={peer.online ? 'accent' : 'ink'} />
                                      <span>{peer.hostname}</span>
                                      {peer.exit_node_option && <Chip tone="neutral">exit node</Chip>}
                                      {peer.exit_node && <Chip tone="accent">in use</Chip>}
                                      {peer.tags.map((tag) => (
                                        <Chip key={tag} tone="neutral">
                                          {tag}
                                        </Chip>
                                      ))}
                                    </div>
                                    <div className="ts-mono ts-sub">{peer.dns_name}</div>
                                  </TableCell>
                                  <TableCell className="ts-mono">{peer.tailscale_ips[0] ?? '–'}</TableCell>
                                  <TableCell>{peer.os ?? '–'}</TableCell>
                                  <TableCell>{peer.online ? (peer.active ? 'Active' : 'Online') : 'Offline'}</TableCell>
                                  <TableCell>
                                    {ping === 'pending'
                                      ? 'Pinging…'
                                      : typeof ping === 'string'
                                        ? ping
                                        : ping
                                          ? `${ping.direct ? 'Direct' : 'Relay'} · ${ping.replies.map((r) => (r.latency_ms == null ? '?' : `${r.latency_ms.toFixed(0)}ms`)).join(' ')}`
                                          : peer.relay
                                            ? `Relay ${peer.relay}`
                                            : peer.active
                                              ? 'Direct'
                                              : '–'}
                                  </TableCell>
                                  <TableCell className="ts-mono">
                                    ↓ {bytes(peer.rx_bytes)} ↑ {bytes(peer.tx_bytes)}
                                  </TableCell>
                                  <TableCell className="ts-row-actions">
                                    <IconButton label={`Ping ${peer.hostname}`} variant="ghost" disabled={!peer.online || ping === 'pending'} onClick={() => void pingPeer(peer)}>
                                      <Activity />
                                    </IconButton>
                                  </TableCell>
                                </TableRow>
                              )
                            })}
                          </TableBody>
                        </Table>
                      </TableFrame>
                    </TableViewport>
                  )}
                </CardBody>
              </Card>
            </TabsContent>

            <TabsContent value="network" className="ts-tab">
              <div className="ts-columns">
                <Card className="ts-card">
                  <CardHeader className="ts-card-header">
                    <span className="ts-card-title">Exit node</span>
                  </CardHeader>
                  <CardBody className="ts-form">
                    <p className="ts-note">Route this device's internet traffic through another device on the tailnet.</p>
                    <Select aria-label="Exit node" value={exitChoice} options={exitOptions} onChange={setExitChoice} />
                    {suggestion && <p className="ts-note">{suggestion}</p>}
                    <div className="ts-actions">
                      <Button variant="primary" disabled={busy || !online || exitChoice === (exitNodes?.current ?? '')} onClick={() => void applyExitNode()}>
                        Apply
                      </Button>
                      <Button variant="ghost" disabled={busy || !online} onClick={() => void suggestExitNode()}>
                        Suggest
                      </Button>
                    </div>
                  </CardBody>
                </Card>

                <Card className="ts-card">
                  <CardHeader className="ts-card-header">
                    <span className="ts-card-title">DNS</span>
                    <IconButton label="Reload DNS status" variant="ghost" onClick={() => void loadNetwork()}>
                      <RefreshCw />
                    </IconButton>
                  </CardHeader>
                  <CardBody>
                    {dns ? (
                      <dl className="ts-facts">
                        <div>
                          <dt>MagicDNS</dt>
                          <dd>
                            {dns.magic_dns ? 'On' : 'Off'}
                            {dns.magic_dns_suffix ? <span className="ts-mono"> · {dns.magic_dns_suffix}</span> : null}
                          </dd>
                        </div>
                        <div>
                          <dt>Resolvers</dt>
                          <dd className="ts-mono">{dns.resolvers.join(' · ') || 'system'}</dd>
                        </div>
                        <div>
                          <dt>Search domains</dt>
                          <dd className="ts-mono">{dns.search_domains.join(' · ') || '–'}</dd>
                        </div>
                        <div>
                          <dt>Split DNS</dt>
                          <dd className="ts-mono">{dns.split_dns_routes.map((r) => `${r.domain} → ${r.resolvers.join(', ')}`).join(' · ') || '–'}</dd>
                        </div>
                        <div>
                          <dt>Certificate domains</dt>
                          <dd className="ts-mono">{dns.cert_domains.join(' · ') || '–'}</dd>
                        </div>
                      </dl>
                    ) : (
                      <p className="ts-note">Loading DNS status…</p>
                    )}
                  </CardBody>
                </Card>
              </div>

              <Card className="ts-card">
                <CardHeader className="ts-card-header">
                  <span className="ts-card-title">Network check</span>
                  <Button variant="ghost" size="sm" disabled={busy} onClick={() => void runNetcheck()}>
                    {busy ? 'Running…' : netcheck ? 'Run again' : 'Run'}
                  </Button>
                </CardHeader>
                <CardBody>
                  {netcheck ? (
                    <dl className="ts-facts ts-facts-grid">
                      <div>
                        <dt>UDP</dt>
                        <dd>{yesNo(netcheck.udp)}</dd>
                      </div>
                      <div>
                        <dt>IPv4 / IPv6</dt>
                        <dd>
                          {yesNo(netcheck.ipv4)} / {yesNo(netcheck.ipv6)}
                        </dd>
                      </div>
                      <div>
                        <dt>Hard NAT</dt>
                        <dd>{yesNo(netcheck.mapping_varies_by_dest_ip)}</dd>
                      </div>
                      <div>
                        <dt>Port mapping</dt>
                        <dd>
                          UPnP {yesNo(netcheck.upnp)} · PMP {yesNo(netcheck.pmp)} · PCP {yesNo(netcheck.pcp)}
                        </dd>
                      </div>
                      <div>
                        <dt>Public IP</dt>
                        <dd className="ts-mono">{netcheck.global_v4 ?? netcheck.global_v6 ?? '–'}</dd>
                      </div>
                      <div>
                        <dt>Preferred relay</dt>
                        <dd>{netcheck.preferred_derp != null ? `DERP ${netcheck.preferred_derp}` : '–'}</dd>
                      </div>
                      <div className="ts-facts-wide">
                        <dt>Relay latency</dt>
                        <dd className="ts-mono">
                          {netcheck.region_latency_ms
                            .slice(0, 6)
                            .map((r) => `${r.region}: ${r.latency_ms.toFixed(0)}ms`)
                            .join(' · ') || '–'}
                        </dd>
                      </div>
                      {netcheck.captive_portal ? (
                        <div className="ts-facts-wide">
                          <dt>Captive portal</dt>
                          <dd>detected</dd>
                        </div>
                      ) : null}
                    </dl>
                  ) : (
                    <p className="ts-note">Checks UDP reachability, NAT type, port mapping, and relay latency. Takes a few seconds.</p>
                  )}
                </CardBody>
              </Card>
            </TabsContent>

            <TabsContent value="settings" className="ts-tab">
              <Card className="ts-card">
                <CardHeader className="ts-card-header">
                  <span className="ts-card-title">Preferences</span>
                  <IconButton label="Reload preferences" variant="ghost" onClick={() => void loadPrefs()}>
                    <RefreshCw />
                  </IconButton>
                </CardHeader>
                <CardBody className="ts-form">
                  {prefs ? (
                    <>
                      {prefRow('Accept subnet routes', prefs.accept_routes, 'accept_routes')}
                      {prefRow('Accept tailnet DNS', prefs.accept_dns, 'accept_dns')}
                      {prefRow('Shields up (block incoming connections)', prefs.shields_up, 'shields_up')}
                      {prefRow('Tailscale SSH server', prefs.ssh, 'ssh')}
                      {prefRow('Offer this device as an exit node', prefs.advertise_exit_node, 'advertise_exit_node')}
                      {prefRow('LAN access while using an exit node', prefs.exit_node_allow_lan_access, 'exit_node_allow_lan_access')}
                      {prefRow('Automatic updates', prefs.auto_update_apply, 'auto_update')}
                      {prefRow('Web client on port 5252', prefs.webclient, 'webclient')}
                      <div className="ts-fields">
                        <label className="ts-field">
                          <span>Hostname override</span>
                          <Input value={hostnameDraft} onChange={setHostnameDraft} placeholder="empty = OS name" aria-label="Hostname" />
                        </label>
                        <label className="ts-field">
                          <span>Advertised subnet routes</span>
                          <Input value={routesDraft} onChange={setRoutesDraft} placeholder="10.0.0.0/8, 192.168.1.0/24" aria-label="Advertised routes" />
                        </label>
                      </div>
                      <div className="ts-actions">
                        <Button
                          variant="primary"
                          disabled={busy}
                          onClick={() =>
                            void setPref(
                              {
                                hostname: hostnameDraft.trim(),
                                advertise_routes: routesDraft
                                  .split(',')
                                  .map((r) => r.trim())
                                  .filter(Boolean),
                              },
                              'Hostname and routes saved.',
                            )
                          }
                        >
                          Save
                        </Button>
                      </div>
                    </>
                  ) : (
                    <p className="ts-note">Loading preferences…</p>
                  )}
                </CardBody>
              </Card>
            </TabsContent>

            <TabsContent value="files" className="ts-tab">
              <div className="ts-columns">
                <Card className="ts-card">
                  <CardHeader className="ts-card-header">
                    <span className="ts-card-title">
                      <Send />
                      Send a file (Taildrop)
                    </span>
                    <IconButton label="Reload targets" variant="ghost" onClick={() => void loadFiles()}>
                      <RefreshCw />
                    </IconButton>
                  </CardHeader>
                  <CardBody className="ts-form">
                    <label className="ts-field">
                      <span>File on this machine</span>
                      <Input value={sendPath} onChange={setSendPath} placeholder="/Users/you/report.pdf" aria-label="File path" />
                    </label>
                    <label className="ts-field">
                      <span>Send to</span>
                      <Select aria-label="Target device" value={sendTarget} options={sendOptions} onChange={setSendTarget} placeholder={fileTargets === null ? 'Loading…' : fileTargets.length ? 'Choose a device' : 'No device accepts files'} />
                    </label>
                    <div className="ts-actions">
                      <Button variant="primary" disabled={busy || !sendPath.trim().startsWith('/') || !sendTarget} onClick={() => void sendFile()}>
                        Send
                      </Button>
                    </div>
                  </CardBody>
                </Card>

                <Card className="ts-card">
                  <CardHeader className="ts-card-header">
                    <span className="ts-card-title">Receive files</span>
                  </CardHeader>
                  <CardBody className="ts-form">
                    <p className="ts-note">Moves everything in this device's Taildrop inbox into a directory.</p>
                    <label className="ts-field">
                      <span>Directory</span>
                      <Input value={receiveDir} onChange={setReceiveDir} placeholder="/Users/you/Downloads" aria-label="Receive directory" />
                    </label>
                    <label className="ts-field">
                      <span>If a file already exists</span>
                      <Select aria-label="Conflict" value={conflict} options={conflictOptions} onChange={setConflict} />
                    </label>
                    <div className="ts-actions">
                      <Button variant="primary" disabled={busy || !receiveDir.trim().startsWith('/')} onClick={() => void receiveFiles()}>
                        Receive
                      </Button>
                    </div>
                  </CardBody>
                </Card>
              </div>

              <Card className="ts-card">
                <CardHeader className="ts-card-header">
                  <span className="ts-card-title">HTTPS certificate</span>
                </CardHeader>
                <CardBody className="ts-form">
                  <p className="ts-note">Fetches a Let's Encrypt certificate and key for one of this device's MagicDNS names.</p>
                  <div className="ts-fields">
                    <label className="ts-field">
                      <span>Domain</span>
                      <Select aria-label="Domain" value={certDomain} options={certOptions} onChange={setCertDomain} placeholder={certOptions.length ? 'Choose a domain' : 'HTTPS is not enabled for this tailnet'} />
                    </label>
                    <label className="ts-field">
                      <span>Write into directory</span>
                      <Input value={certDir} onChange={setCertDir} placeholder="/Users/you/certs" aria-label="Certificate directory" />
                    </label>
                  </div>
                  <div className="ts-actions">
                    <Button variant="primary" disabled={busy || !certDomain || !certDir.trim().startsWith('/')} onClick={() => void fetchCert()}>
                      Fetch certificate
                    </Button>
                  </div>
                </CardBody>
              </Card>
            </TabsContent>

            <TabsContent value="account" className="ts-tab">
              <div className="ts-columns">
                <Card className="ts-card">
                  <CardHeader className="ts-card-header">
                    <span className="ts-card-title">Accounts</span>
                    <IconButton label="Reload accounts" variant="ghost" onClick={() => void loadAccount()}>
                      <RefreshCw />
                    </IconButton>
                  </CardHeader>
                  <CardBody className="ts-form">
                    {accounts === null ? (
                      <p className="ts-note">Loading accounts…</p>
                    ) : accounts.length === 0 ? (
                      <p className="ts-note">No account is signed in on this device.</p>
                    ) : (
                      accounts.map((account) => (
                        <div key={account.id} className="ts-pref">
                          <span className="ts-pref-label">
                            {account.account}
                            <span className="ts-mono ts-sub"> {account.tailnet}</span>
                          </span>
                          {account.selected ? (
                            <Chip tone="success">Active</Chip>
                          ) : (
                            <Button variant="ghost" size="sm" disabled={busy} onClick={() => void switchAccount(account.id)}>
                              Switch
                            </Button>
                          )}
                        </div>
                      ))
                    )}
                    <div className="ts-actions">
                      <Button variant="ghost" disabled={busy} onClick={() => void login()}>
                        Sign in to another account
                      </Button>
                      <Button variant="ghost" disabled={busy || !online} onClick={() => void logout()}>
                        Log out
                      </Button>
                    </div>
                  </CardBody>
                </Card>

                <Card className="ts-card">
                  <CardHeader className="ts-card-header">
                    <span className="ts-card-title">Client</span>
                  </CardHeader>
                  <CardBody className="ts-form">
                    <dl className="ts-facts">
                      <div>
                        <dt>Version</dt>
                        <dd className="ts-mono">{version?.long ?? status?.version ?? '–'}</dd>
                      </div>
                      <div>
                        <dt>Latest release</dt>
                        <dd className="ts-mono">{version?.upstream ?? (version ? 'up to date' : '–')}</dd>
                      </div>
                      <div>
                        <dt>Tailnet lock</dt>
                        <dd>
                          {lock ? (lock.enabled ? `Enabled${lock.node_signed === false ? ' · this node is not signed' : ''}` : 'Not enabled') : '–'}
                        </dd>
                      </div>
                      {lock?.node_key && (
                        <div>
                          <dt>Lock key</dt>
                          <dd className="ts-mono">{lock.node_key}</dd>
                        </div>
                      )}
                    </dl>
                    <div className="ts-actions">
                      <Button variant="ghost" disabled={busy} onClick={() => void updateClient(true)}>
                        Check for updates
                      </Button>
                      <Button variant="ghost" disabled={busy} onClick={() => void updateClient(false)}>
                        Update now
                      </Button>
                      <Button variant="ghost" disabled={busy} onClick={() => void bugreport()}>
                        Bug report
                      </Button>
                    </div>
                  </CardBody>
                </Card>
              </div>
            </TabsContent>
          </Tabs>
        </PageMain>
      </PageBody>

      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title="Publish to the internet?"
        description="Anyone with the link can open this until the route is stopped. Funnel carries no Tailscale identity headers, so the service cannot tell who is connecting."
        confirmLabel="Publish publicly"
        onConfirm={() => {
          setConfirming(false)
          void createShare(true)
        }}
      />
    </PageShell>
  )
}
