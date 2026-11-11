import { useCallback, useEffect, useRef, useState, type ComponentType } from 'react'
import {
  Badge, Button, EmptyState, Input, LiveRegion, PageHeader, PageMain, PageShell,
  RawValueInput, SegmentedControl, SettingsField, SettingsList, SettingsSection, StatusPanel, Switch,
  type ConfigFormProps, type Host, type JsonValue, type PageRenderProps, type SecretKeyFieldProps,
} from '@iii-dev/console-ui'
import { resolveConfigurationId } from '@iii-dev/console-ui/configuration'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { useCopyFlash, useWorkerLive } from '@iii-dev/console-ui/hooks'
import { Network, RefreshCw } from 'lucide-react'

// functions: the worker's hub function ids (<machine>::<local id>), sorted.
type ComponentState = { name: string; state: string; functions?: string[] }
type EngineFunction = { function_id: string; worker_name?: string; description?: string }
type Node = {
  name: string; address: string; os: string; arch: string; version: string
  online: boolean; last_seen: string; ephemeral: boolean; path: string
  workers: ComponentState[]
}
type Join = { code: string }
// The hub's mesh::status has no `mode`; only a client-mode worker reports one.
type ClientStatus = {
  mode: 'client'; hub_url: string; node: string; namespace: string; state: string; error: string
  hub_wants_version?: string; shared: { worker: string; state: string; functions: string[] }[]
}
// The hub's mesh::status; `tunnel` is present only while the tunnel is enabled.
type Tunnel = { enabled: boolean; tunnel_id: string; state: string; url?: string; error?: string }
type HubStatus = { public_url?: string; tunnel?: Tunnel }
type SecretInput = ComponentType<SecretKeyFieldProps>
const NO_SOURCES: readonly string[] = []

function CopyValue({ label, value }: { label: string; value: string }) {
  const { state, copy } = useCopyFlash(value)
  return <div className="mesh-ui-copy">
    <div className="mesh-ui-copy-head"><span>{label}</span>
      <Button variant="ghost" size="sm" onClick={copy} aria-label={`Copy ${label}`}>
        {state === 'copied' ? 'Copied' : state === 'failed' ? 'Copy failed — retry' : 'Copy'}
      </Button>
    </div>
    <code>{value}</code>
    <LiveRegion announcement={state === 'idle' ? null : { seq: 1, urgency: 'polite', text: state === 'copied' ? `${label} copied` : `Could not copy ${label}. Select and copy the text.` }} />
  </div>
}

function FunctionList({ label, functions }: { label: string; functions: EngineFunction[] }) {
  // role="list": Safari drops list semantics from a list without markers.
  return <ul role="list" className="mesh-ui-functions" aria-label={label}>{functions.map((fn) => <li key={fn.function_id}>
    <code>{fn.function_id}</code>{fn.description ? <> <span className="mesh-ui-muted">{fn.description}</span></> : null}
  </li>)}</ul>
}

function SharedWorkers({ node, functions }: { node: Node; functions: EngineFunction[] }) {
  const label = `mesh-node-${node.name}-workers`
  const workers = node.workers ?? []
  const described = new Map(functions.map((fn) => [fn.function_id, fn.description]))
  return <div className="mesh-ui-workers">
    <p className="mesh-ui-label" id={label}>Shared workers</p>
    {workers.length ? <ul role="list" aria-labelledby={label}>
      {workers.map((worker) => {
        // An offline machine's functions left with its bridge; the hub still holds the last ids it reported.
        const ids = node.online ? worker.functions ?? [] : []
        return <li key={worker.name}>
          <div className="mesh-ui-worker"><code>{worker.name}</code>{node.online ? <span className="mesh-ui-muted">{worker.state}</span> : null}</div>
          {ids.length ? <FunctionList label={`${worker.name} functions`} functions={ids.map((id) => ({ function_id: id, description: described.get(id) }))} />
            : <p className="mesh-ui-muted">{node.online ? 'No functions registered yet' : 'Unavailable while the machine is offline'}</p>}
        </li>
      })}
    </ul> : <p className="mesh-ui-muted">Not sharing any workers</p>}
  </div>
}

type Mode = 'hub' | 'client'
// The basics a connection needs. Everything else lives in the configuration form.
type Basics = { mode: Mode; public_url: string; tunnel: boolean; hub_url: string; join: string; name: string; share: string[] }
const BASIC_PATHS: Record<keyof Basics, string[]> = {
  mode: ['mode'], public_url: ['public_url'], tunnel: ['tunnel', 'enabled'], hub_url: ['client', 'hub_url'], join: ['client', 'join'],
  name: ['client', 'name'], share: ['client', 'share'],
}
// The running worker applies these without a restart.
const LIVE = new Set<keyof Basics>(['share', 'public_url', 'tunnel'])
function basicsOf(config: JsonValue | null): Basics {
  const text = (path: string[]) => { const v = config === null ? undefined : at(config, path); return typeof v === 'string' ? v : '' }
  const share = config === null ? undefined : at(config, BASIC_PATHS.share)
  return {
    mode: text(BASIC_PATHS.mode) === 'client' ? 'client' : 'hub', public_url: text(BASIC_PATHS.public_url),
    tunnel: config !== null && at(config, BASIC_PATHS.tunnel) === true,
    hub_url: text(BASIC_PATHS.hub_url), join: text(BASIC_PATHS.join), name: text(BASIC_PATHS.name),
    share: Array.isArray(share) ? share.filter((s): s is string => typeof s === 'string') : [],
  }
}
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b)
const MODE_OPTIONS: { value: Mode; label: string; icon: false }[] = [
  { value: 'hub', label: 'Hub', icon: false }, { value: 'client', label: 'Client', icon: false },
]
const MODE_HELP: Record<Mode, string> = {
  hub: 'This project is the hub. Other projects join it as clients and share workers here.',
  client: 'This project joins another project\'s hub and shares the workers you pick below.',
}
// The mesh worker refuses these; listing them would only offer a switch that fails.
const NEVER_SHARED = new Set(['mesh', 'secrets', 'configuration', 'rbac-proxy'])
// Sharing one of these lets the hub project run commands on this machine.
const RUNS_COMMANDS = new Set(['ide', 'harness', 'shell', 'coder', 'tmux', 'code-runner'])
const NAME = /^[a-z][a-z0-9-]{0,30}$/
const RAW_CONFIG = 'The mesh settings are one raw value. Open Settings and replace it with individual settings first.'
const STATE_BADGE: Record<string, 'ok' | 'warn' | 'alert' | 'default'> = {
  connected: 'ok', 'restart required': 'warn', 'join failed': 'alert', error: 'alert',
}

function JoinField({ value, onChange, secretInput: SecretKeyField }: { value: string; onChange: (next: string) => void; secretInput?: SecretInput }) {
  // A non-reference stays in this draft: the join code itself never reaches the versioned entry.
  const [draft, setDraft] = useState<string | null>(null)
  if (SecretKeyField) return <div data-field="client.join">
    <SecretKeyField name="MESH_JOIN" label="Join code or token" value={value || undefined} consumers={['mesh']} environment onChange={(next: string | undefined) => onChange(next ?? '')} />
  </div>
  return <SettingsField label="Join code or token" field="client.join"
    description="A secret://NAME or env://NAME reference to the hub's join code or token; mesh must be a consumer."
    error={draft !== null ? 'Use secret://NAME or env://NAME. Store the code in the secrets worker; never paste it here.' : undefined}
    renderControl={(props) => <Input {...props} value={draft ?? value} onChange={(next) => {
      const ok = next === '' || JOIN_REF.test(next)
      setDraft(ok ? null : next)
      if (ok) onChange(next)
    }} />} />
}

// Tunnel states that stay until something changes; any other state is still moving.
const TUNNEL_RESTING = new Set(['ready', 'overridden', 'unavailable', 'failed'])
// Longer than quick-tunnel's 10-minute bound on creating an OpenTunnel identity.
const SETTLE_MS = 12 * 60_000

function TunnelLine({ tunnel, stalled }: { tunnel?: Tunnel; stalled: boolean }) {
  const line = stalled ? <p className="mesh-ui-muted">The hub has not reported a tunnel result. Check the mesh and quick-tunnel workers, then refresh.</p>
    : tunnel?.state === 'ready' && tunnel.url ? <CopyValue label="Tunnel URL" value={tunnel.url} />
    : tunnel?.state === 'starting' || tunnel?.state === 'reconnecting' ? <p className="mesh-ui-muted">Starting the tunnel…</p>
    : tunnel?.state === 'overridden' ? <p className="mesh-ui-muted">Not used while a Public URL is set</p>
    : tunnel?.state === 'unavailable' || tunnel?.state === 'failed' ? <StatusPanel variant="warn"
      headline={tunnel.state === 'failed' ? 'The tunnel failed' : 'The tunnel is unavailable'} detail={tunnel.error} />
    : null
  // An item of the settings list, and announced when polling changes it.
  return line ? <div role="listitem" aria-live="polite" className="mesh-ui-tunnel">{line}</div> : null
}

function ClientSetup({ basics, edit, status, workers, secretInput }: {
  basics: Basics; edit: <K extends keyof Basics>(key: K, value: Basics[K]) => void
  status: ClientStatus | null; workers: string[]; secretInput?: SecretInput
}) {
  const shared = new Map((status?.shared ?? []).map((s) => [s.worker, s]))
  // A shared name that is not running locally still needs its switch, to turn it off.
  const names = [...new Set([...workers, ...basics.share])].filter((n) => !NEVER_SHARED.has(n)).sort()
  const risky = basics.share.filter((n) => RUNS_COMMANDS.has(n))
  const state = status?.state ?? ''
  return <>
    <section className="mesh-ui-section" aria-labelledby="mesh-connect-title">
      <div className="mesh-ui-section-head"><h2 id="mesh-connect-title">Hub connection</h2>
        {state ? <Badge variant={STATE_BADGE[state] ?? 'default'}>{state}</Badge> : null}
        {status?.node ? <span className="mesh-ui-muted">joined as <code>{status.node}</code>{status.namespace ? <> in namespace <code>{status.namespace}</code></> : null}</span> : null}
      </div>
      {status?.error ? <StatusPanel variant="alert" headline="Not sharing" detail={status.error} /> : null}
      {status?.hub_wants_version ? <StatusPanel variant="warn" headline={`The hub wants version ${status.hub_wants_version}`}
        detail="Nothing is shared until this project's mesh worker runs the hub's version." /> : null}
      <SettingsList>
        <SettingsField label="Hub URL" field="client.hub_url" description="The address the hub gave you. Not needed when you join with a token."
          renderControl={(props) => <Input {...props} placeholder="https://mesh.example.com" value={basics.hub_url} onChange={(next) => edit('hub_url', next)} />} />
        <JoinField value={basics.join} onChange={(next) => edit('join', next)} secretInput={secretInput} />
        <SettingsField label="Machine name" field="client.name" description="How this project appears on the hub. The hub may add a suffix such as -2."
          error={basics.name && !NAME.test(basics.name) ? 'Use lowercase letters, digits or hyphens, starting with a letter.' : undefined}
          renderControl={(props) => <Input {...props} placeholder="my-laptop" value={basics.name} onChange={(next) => edit('name', next)} />} />
      </SettingsList>
    </section>
    <section className="mesh-ui-section" aria-labelledby="mesh-share-title">
      <div className="mesh-ui-section-head"><h2 id="mesh-share-title">Shared workers</h2>
        <span className="mesh-ui-muted">{basics.share.length ? `${basics.share.length} shared` : 'Nothing shared'} · the hub calls them as <code>{status?.node || basics.name || 'machine'}::function</code></span>
      </div>
      {risky.length ? <StatusPanel variant="warn" headline={`${risky.join(', ')} can run commands on this machine`}
        detail="Anyone in the hub project can call shared workers, so sharing these gives that project remote code execution here." /> : null}
      {names.length === 0 ? <p className="mesh-ui-muted">No local workers found. Start a worker in this project to share it.</p> : null}
      <ul className="mesh-ui-share">{names.map((name) => {
        const on = basics.share.includes(name)
        const live = shared.get(name)
        return <li key={name}>
          <label><Switch checked={on} aria-label={`Share ${name}`}
            onChange={(e) => edit('share', e.currentTarget.checked ? [...basics.share, name].sort() : basics.share.filter((n) => n !== name))} />
            <code>{name}</code>
            {RUNS_COMMANDS.has(name) ? <Badge variant="warn">runs commands</Badge> : null}
            {on ? <span className="mesh-ui-muted">{live?.state ?? 'not saved yet'}</span> : null}
          </label>
          {on && live?.functions?.length ? <span className="mesh-ui-functions">{live.functions.map((fn) => <code key={fn}>{fn}</code>)}</span> : null}
        </li>
      })}</ul>
      {risky.length === 0 && names.length ? <p className="mesh-ui-muted">Anyone in the hub project can call what you share.</p> : null}
    </section>
  </>
}

function MeshPage({ host, configurationId, secretInput, onRequestClose, setDirty }: PageRenderProps & { host: Host; configurationId: string; secretInput?: SecretInput }) {
  const fetch = useCallback(async () => {
    const [nodes, join, status, config, functions] = await Promise.allSettled([
      host.iii.call<Node[]>('mesh::nodes::list', {}),
      host.iii.call<Join>('mesh::join::code', {}),
      host.iii.call<Partial<ClientStatus> & HubStatus>('mesh::status', {}),
      // Raw keeps ${VAR} templates intact when the entry is written back.
      host.iii.call<{ value: JsonValue }>('configuration::get', { id: configurationId, raw: true }),
      host.iii.call<{ functions: EngineFunction[] }>('engine::functions::list', {}),
    ])
    const engineFunctions = functions.status === 'fulfilled' ? functions.value.functions ?? [] : []
    return {
      client: status.status === 'fulfilled' && status.value?.mode === 'client' ? status.value as ClientStatus : null,
      hub: status.status === 'fulfilled' && status.value?.mode !== 'client' ? status.value as HubStatus : null,
      // Online machines first, then by name.
      nodes: nodes.status === 'fulfilled' ? nodes.value.sort((a, b) => Number(b.online) - Number(a.online) || a.name.localeCompare(b.name)) : null,
      nodesError: nodes.status === 'rejected' ? errorMessage(nodes.reason) : null,
      join: join.status === 'fulfilled' ? join.value : null,
      joinError: join.status === 'rejected' ? errorMessage(join.reason) : null,
      config: config.status === 'fulfilled' && isRecord(config.value?.value) ? config.value.value : null,
      // A raw root (for example one ${VAR} template) is edited only in the configuration form.
      rawConfig: config.status === 'fulfilled' && !isRecord(config.value?.value),
      configError: config.status === 'rejected' ? errorMessage(config.reason) : null,
      functions: engineFunctions,
      workers: [...new Set(engineFunctions.map((f) => f.worker_name ?? '').filter((n) => /^[a-z0-9-]+$/.test(n)))],
    }
  }, [host, configurationId])
  const { data, loading, error, refresh } = useWorkerLive({
    iii: host.iii, fetch, sources: NO_SOURCES, handlerId: 'iii::mesh-ui::events',
  })
  // What the last Save wrote, shown until the next read replaces it, so the old values never flash back.
  const [written, setWritten] = useState<JsonValue | null>(null)
  useEffect(() => setWritten(null), [data])
  const config = written ?? data?.config ?? null
  const raw = data?.rawConfig === true
  const saved = basicsOf(config)
  // Only what differs from the stored entry is kept, so a refresh never drops an edit in progress.
  const [edits, setEdits] = useState<Partial<Basics>>({})
  const basics = { ...saved, ...edits }
  const changed = (Object.keys(edits) as (keyof Basics)[]).filter((key) => !same(edits[key], saved[key]))
  const dirty = changed.length > 0 && !raw
  const edit = <K extends keyof Basics>(key: K, value: Basics[K]) => setEdits((prev) => ({ ...prev, [key]: value }))
  useEffect(() => { setDirty?.(dirty ? 'Mesh connection' : false) }, [dirty, setDirty])
  const [busy, setBusy] = useState<'save' | 'restart' | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  const run = async (kind: 'save' | 'restart', work: () => Promise<unknown>) => {
    setBusy(kind); setFailure(null)
    try { await work() } catch (err) { setFailure(errorMessage(err)) } finally { setBusy(null); refresh() }
  }
  const save = () => run('save', async () => {
    // Re-read first: the page stays mounted while the configuration form saves the same entry.
    const current = await host.iii.call<{ value: JsonValue }>('configuration::get', { id: configurationId, raw: true })
    // Writing the basics over a raw root would replace everything it holds.
    if (!isRecord(current?.value)) throw new Error(RAW_CONFIG)
    let next: JsonValue = current.value
    // Unedited keys are left untouched, so raw or templated values survive a save.
    for (const key of changed) next = setAt(next, BASIC_PATHS[key], edits[key] as JsonValue)
    await host.iii.call('configuration::set', { id: configurationId, value: next })
    setWritten(next)
    // Edits typed while the save was in flight stay.
    setEdits((prev) => {
      const rest = { ...prev }
      for (const key of Object.keys(edits) as (keyof Basics)[]) if (same(rest[key], edits[key])) delete rest[key]
      return rest
    })
  })
  // shortcut: assumes the compose key is "mesh"; pass the key through mesh::status if projects rename it.
  const restart = () => run('restart', () => host.iii.call('compose::restart', { container: 'mesh' }))
  const running: Mode = data?.client ? 'client' : 'hub'
  const hubView = basics.mode !== 'client' && running !== 'client'
  const needsRestart = !dirty && data !== undefined && config !== null && (saved.mode !== running || data?.client?.state === 'restart required')
  // mesh::status is not pushed, and a tunnel settles seconds to minutes after a save: re-read until the hub
  // shows the saved switch and a resting state. Only the hub knows what a templated Public URL expands to.
  const tunnel = data?.hub?.tunnel
  const settling = Boolean(data?.hub) && (saved.tunnel !== Boolean(tunnel) || (tunnel !== undefined && !TUNNEL_RESTING.has(tunnel.state)))
  const [stalled, setStalled] = useState(false)
  useEffect(() => {
    setStalled(false)
    if (!settling) return
    const until = Date.now() + SETTLE_MS
    const timer = setInterval(() => {
      if (Date.now() > until) { clearInterval(timer); setStalled(true) } else if (!document.hidden) refresh()
    }, 5000)
    return () => clearInterval(timer)
    // `written` changes on every save, so each save gets a full window.
  }, [settling, refresh, written])
  // A template can expand to nothing; then the hub's own address decides.
  const hasAddress = saved.public_url.includes('${') && data?.hub ? Boolean(data.hub.public_url) : Boolean(saved.public_url)
  return <PageShell className="mesh-ui-shell">
    <PageHeader icon={<Network />} title="Mesh" description={running === 'client' ? 'Sharing workers with a hub' : 'Your machines'} onClose={onRequestClose}
      actions={<Button variant="ghost" size="sm" disabled={loading} onClick={refresh}>
        <RefreshCw size={16} aria-hidden />{loading ? 'Refreshing…' : 'Refresh'}
      </Button>} />
    <PageMain className="mesh-ui-main">
      {raw ? <section className="mesh-ui-section"><StatusPanel variant="warn" headline="The mesh settings are one raw value"
        detail="Open Settings and replace it with individual settings to edit them here." /></section>
        : <section className="mesh-ui-section mesh-ui-mode" aria-labelledby="mesh-mode-title">
          <h2 id="mesh-mode-title">This project is a</h2>
          <SegmentedControl aria-label="Mesh mode" variant="radio" value={basics.mode} options={MODE_OPTIONS} onChange={(next) => edit('mode', next)} />
          <p className="mesh-ui-muted">{MODE_HELP[basics.mode]}</p>
        </section>}
      {data?.configError ? <section className="mesh-ui-section"><StatusPanel variant="alert" headline="Could not load the mesh settings"
        detail={data.configError} action={<Button onClick={refresh}>Retry</Button>} /></section> : null}
      {needsRestart ? <section className="mesh-ui-section"><StatusPanel variant="warn"
        headline={saved.mode !== running ? `Restart the mesh worker to switch to ${saved.mode} mode` : 'Restart the mesh worker to apply the new connection'}
        detail="Saved settings take effect when the worker starts again."
        action={<Button disabled={busy !== null} onClick={restart}>{busy === 'restart' ? 'Restarting…' : 'Restart mesh'}</Button>} /></section> : null}
      {hubView ? <section className="mesh-ui-section" aria-labelledby="mesh-nodes-title" aria-busy={loading}>
        <div className="mesh-ui-section-head"><h2 id="mesh-nodes-title">Machines</h2>
          {data?.nodes ? <span className="mesh-ui-muted">{data.nodes.filter((n) => n.online).length} online · {data.nodes.length} total</span> : null}
        </div>
        {loading && !data ? <p role="status">Loading machines and join instructions…</p> : null}
        {error || data?.nodesError ? <StatusPanel variant="alert" headline="Could not load machines"
          detail={error || data?.nodesError} action={<Button onClick={refresh}>Retry</Button>} /> : null}
        {data?.nodes?.length === 0 ? <EmptyState icon={Network} title="No machines joined"
          description="Use the hub URL and join code below in another engine's Mesh page to connect it." /> : null}
        <div className="mesh-ui-nodes">{data?.nodes?.map((node) => <article key={node.name} className="mesh-ui-node" aria-label={node.name}>
          <header className="mesh-ui-node-head"><h3><code>{node.name}</code></h3>
            <Badge variant={node.online ? 'ok' : 'default'}>{node.online ? 'Online' : 'Offline'}</Badge>
            {node.ephemeral ? <Badge>Ephemeral</Badge> : null}
          </header>
          <SharedWorkers node={node} functions={data.functions} />
          <dl className="mesh-ui-details">
            <div><dt>Address</dt><dd><code>{node.address}</code></dd></div>
            <div><dt>Platform</dt><dd>{node.os || 'Unknown'}{node.arch ? ` / ${node.arch}` : ''}</dd></div>
            <div><dt>Version</dt><dd><code>{node.version || 'Not reported'}</code></dd></div>
            <div><dt>Path</dt><dd><code>{node.path || 'Not reported'}</code></dd></div>
            {!node.online ? <div><dt>Last seen</dt><dd>{node.last_seen && !node.last_seen.startsWith('0001-') ? formatRelative(node.last_seen) : 'Not reported'}</dd></div> : null}
          </dl>
        </article>)}</div>
      </section> : null}
      {raw ? null : basics.mode === 'client'
        ? <ClientSetup basics={basics} edit={edit} status={data?.client ?? null} workers={data?.workers ?? []} secretInput={secretInput} />
        : <section className="mesh-ui-section" aria-labelledby="mesh-address-title">
          <div className="mesh-ui-section-head"><h2 id="mesh-address-title">Hub address</h2>
            {hasAddress || saved.tunnel ? null : <Badge variant="warn">needed before machines can join</Badge>}
          </div>
          <SettingsList>
            <SettingsField label="Expose through a tunnel" field="tunnel.enabled" description="Gives this hub a public address through the quick-tunnel worker."
              renderControl={(props) => <Switch {...props} checked={basics.tunnel} onChange={(e) => edit('tunnel', e.currentTarget.checked)} />} />
            <TunnelLine tunnel={tunnel} stalled={stalled && settling} />
            <SettingsField label="Public URL" field="public_url" description="A manual address. Overrides the tunnel."
              renderControl={(props) => <Input {...props} placeholder="https://mesh.example.com" value={basics.public_url} onChange={(next) => edit('public_url', next)} />} />
          </SettingsList>
        </section>}
      {dirty || failure ? <section className="mesh-ui-section mesh-ui-save" aria-live="polite">
        {failure ? <StatusPanel variant="alert" headline="Could not save" detail={failure} /> : null}
        {dirty ? <div className="mesh-ui-save-bar">
          <span className="mesh-ui-muted">Unsaved changes{changed.some((k) => !LIVE.has(k)) ? ' · applied after a restart' : ' · applied within a few seconds'}</span>
          <Button variant="ghost" size="sm" disabled={busy !== null} onClick={() => setEdits({})}>Discard</Button>
          <Button size="sm" disabled={busy !== null || (basics.mode === 'client' && basics.name !== '' && !NAME.test(basics.name))} onClick={save}>{busy === 'save' ? 'Saving…' : 'Save'}</Button>
        </div> : null}
      </section> : null}
      {hubView ? <section className="mesh-ui-section" aria-labelledby="mesh-join-title">
        <div className="mesh-ui-section-head"><h2 id="mesh-join-title">Connect another engine</h2></div>
        <p className="mesh-ui-muted">On the other machine, open Mesh in that project's Console, choose Client, and paste these into Hub URL and Join code or token. If that field takes only a secret://NAME reference, store the code in the secrets worker and enter the reference. Share the join code only with people you trust.</p>
        {data?.joinError ? <StatusPanel variant="warn" headline="Join instructions unavailable" detail={data.joinError}
          action={<Button onClick={refresh}>Retry</Button>} /> : null}
        {data?.join ? <div className="mesh-ui-join">
          {data.hub?.public_url ? <CopyValue label="Hub URL" value={data.hub.public_url} /> : null}
          <CopyValue label="Join code" value={data.join.code} />
        </div> : null}
      </section> : null}
    </PageMain>
  </PageShell>
}

function isRecord(value: JsonValue | undefined): value is Record<string, JsonValue> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}
function at(value: JsonValue, path: string[]): JsonValue | undefined {
  let current: JsonValue | undefined = value
  for (const key of path) {
    if (Array.isArray(current)) current = current[Number(key)]
    else if (isRecord(current) && Object.hasOwn(current, key)) current = current[key]
    else return undefined
  }
  return current
}
function setAt(value: JsonValue | undefined, path: string[], next: JsonValue): JsonValue {
  if (!path.length) return next
  const [key, ...rest] = path
  if (Array.isArray(value)) return value.map((item, i) => i === Number(key) ? setAt(item, rest, next) : item)
  // Only missing parents may be created by a typed edit; existing raw blocks need conversion.
  if (value !== undefined && !isRecord(value)) return value
  const record = value ?? {}
  const child = Object.hasOwn(record, key) ? record[key] : undefined
  return { ...record, [key]: setAt(child, rest, next) }
}
function pointer(path: string[]) { return path.length ? '/' + path.map((s) => s.replaceAll('~', '~0').replaceAll('/', '~1')).join('/') : '' }
function defaultConfiguration(): JsonValue {
  return { public_url: '', listen: '0.0.0.0:49180', gateway_port: 49135, proxy_url: 'ws://127.0.0.1:49200', stun_port: 0, trusted_proxies: [] }
}

type FormFieldProps = { form: ConfigFormProps; path: string[]; label: string; description?: string; numeric?: boolean; literal?: JsonValue }
function OpaqueValue({ form, path, label, value, literal, replacementLabel = 'literal settings' }: {
  form: ConfigFormProps; path: string[]; label: string; value: JsonValue; literal: JsonValue; replacementLabel?: string
}) {
  const text = typeof value === 'string' ? value : JSON.stringify(value)
  return <SettingsField label={label} field={path.join('.') || undefined} controlSize="full"
    description="This raw value is preserved. Replace it explicitly to edit individual settings."
    error={form.errors?.get(pointer(path))}
    renderControl={(props) => <RawValueInput {...props} label={label} value={text}
      readOnly={typeof value !== 'string'} kind={typeof value === 'string' && value.includes('${') ? 'environment' : 'custom'}
      replacementLabel={replacementLabel}
      onUseLiteral={() => form.onChange(setAt(form.value, path, literal))}
      onChange={(next) => { if (typeof value === 'string') form.onChange(setAt(form.value, path, next)) }} />} />
}
function Field({ form, path, label, description, numeric, literal = numeric ? 0 : '' }: FormFieldProps) {
  const value = at(form.value, path)
  if (value !== undefined && (numeric ? typeof value !== 'number' && value !== '' : typeof value !== 'string')) {
    return <OpaqueValue form={form} path={path} label={label} value={value} literal={literal} replacementLabel="default value" />
  }
  return <SettingsField label={label} description={description} field={path.join('.')} error={form.errors?.get(pointer(path))}
    renderControl={(props) => <Input {...props} value={value === undefined ? '' : String(value)}
      onChange={(next) => form.onChange(setAt(form.value, path, numeric && /^\d+$/.test(next) ? Number(next) : next))} />} />
}
function StringList({ form, path, label, description }: FormFieldProps) {
  const value = at(form.value, path)
  if (value !== undefined && !Array.isArray(value)) return <OpaqueValue form={form} path={path} label={label} value={value} literal={[]} replacementLabel="an item list" />
  const values = value ?? []
  return <SettingsSection title={label} description={description} action={<Button variant="ghost" size="sm" onClick={() => form.onChange(setAt(form.value, path, [...values, '']))}>Add item</Button>}>
    <SettingsList>{values.map((_, index) => <div className="mesh-ui-form-item" key={index}>
      <Field form={form} path={[...path, String(index)]} label={`${label} ${index + 1}`} />
      <Button variant="ghost" size="sm" aria-label={`Remove ${label} ${index + 1}`} onClick={() => form.onChange(setAt(form.value, path, values.filter((_, i) => i !== index)))}>Remove</Button>
    </div>)}{values.length === 0 ? <p className="mesh-ui-form-empty">No items configured.</p> : null}</SettingsList>
  </SettingsSection>
}
const JOIN_REF = /^(secret|env):\/\/\S+$/
function MeshConfiguration(form: ConfigFormProps) {
  const root = useRef<HTMLDivElement>(null)
  const focus = form.focusField?.join('.')
  useEffect(() => {
    if (!focus) return
    const target = root.current?.querySelector<HTMLElement>(`[data-field="${CSS.escape(focus)}"]`)
    for (let ancestor = target?.parentElement; ancestor; ancestor = ancestor.parentElement) {
      if (ancestor instanceof HTMLDetailsElement) ancestor.open = true
    }
    target?.focus(); target?.scrollIntoView({ block: 'center' })
  }, [focus])
  if (!isRecord(form.value)) return <div className="mesh-ui-configuration" ref={root}>
    <OpaqueValue form={form} path={[]} label="Mesh configuration" value={form.value} literal={defaultConfiguration()} replacementLabel="default settings" />
  </div>
  const client = form.value.mode === 'client'
  return <div className="mesh-ui-configuration" ref={root}>
    <p className="mesh-ui-muted">Mode, hub address, join code and shared workers are set on the Mesh page. These are the advanced settings.</p>
    {client ? <p className="mesh-ui-form-empty">Client mode has no advanced settings.</p> : <>
      <SettingsSection title="Network" description="Network changes reconfigure the hub.">
        <SettingsList>
          <Field form={form} path={['listen']} label="Listen address" />
          <Field form={form} path={['gateway_port']} label="Gateway port" numeric literal={49135} />
          <Field form={form} path={['proxy_url']} label="Proxy URL" description="Direct numeric loopback WebSocket endpoint for rbac-proxy." />
          <Field form={form} path={['stun_port']} label="STUN port" description="Use 0 to disable STUN." numeric />
        </SettingsList>
      </SettingsSection>
      <details className="mesh-ui-advanced"><summary>Advanced networking</summary>
        <StringList form={form} path={['trusted_proxies']} label="Trusted proxies" description="CIDRs trusted to supply forwarded client addresses." />
      </details>
    </>}
    {form.errors?.size ? <StatusPanel variant="alert" headline="Check the configuration fields" detail={[...form.errors.values()].join(' · ')} /> : null}
  </div>
}

export default async function setup(host: Host) {
  const configurationId = await resolveConfigurationId(host.iii, 'mesh')
  // Consoles without the shared secrets field fall back to a reference-only input.
  const secretInput = host.components?.SecretKeyField as SecretInput | undefined
  host.configForms.register('mesh', (props) => <MeshConfiguration {...props} />)
  host.pages.register({ id: 'mesh-manager', title: 'Mesh', configurationId,
    render: (props) => <MeshPage host={host} configurationId={configurationId} secretInput={secretInput} {...props} />,
  })
}
