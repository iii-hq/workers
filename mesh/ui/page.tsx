import { useCallback, useEffect, useRef, useState, type ComponentType, type ReactNode } from 'react'
import {
  Badge, Button, ConfirmDialog, EmptyState, Input, LiveRegion, PageHeader, PageMain, PageShell,
  RawValueInput, SegmentedControl, SettingsField, SettingsList, SettingsSection, StatusPanel, Switch,
  type ConfigFormProps, type Host, type JsonValue, type PageRenderProps, type SecretKeyFieldProps, type StatusPanelProps,
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
type Shared = { worker: string; state: string; functions: string[] }
// The hub's mesh::status has no `mode`; only a client-mode worker reports one.
// retrying: the worker tries again by itself. requested_name: the client.name the hub last answered for.
type ClientStatus = {
  mode: 'client'; hub_url: string; node: string; namespace: string; state: string; error: string
  retrying?: boolean; requested_name?: string; rename_error?: string
  // join_unused: client.hub_url or client.join is not the enrollment in use; it is used only after a leave.
  join_unused?: boolean; hub_wants_version?: string; shared: Shared[]
  // What this machine can call, by owner, hub first: ids are its local hub::… and peer::<machine>::… functions.
  remote?: { machine: string; functions: string[] }[]
}
// The hub's mesh::status; `tunnel` is present only while the tunnel is enabled.
type Tunnel = { enabled: boolean; tunnel_id: string; state: string; url?: string; error?: string }
// shared: the hub's own shares, ids as joined machines see them (hub::…). catalog_skipped: machines over the catalog caps.
type HubStatus = { public_url?: string; tunnel?: Tunnel; shared?: Shared[]; catalog_skipped?: string[] }
// The client's mesh::leave; `warning` is set when the hub may still list the machine.
type Left = { hub_notified: boolean; warning: string }
type SecretInput = ComponentType<SecretKeyFieldProps>
const NO_SOURCES: readonly string[] = []

function CopyValue({ label, value, action }: { label: string; value: string; action?: ReactNode }) {
  const { state, copy } = useCopyFlash(value)
  return <div className="mesh-ui-copy">
    <div className="mesh-ui-copy-head"><span>{label}</span>
      {action}
      <Button variant="ghost" size="sm" onClick={copy} aria-label={`Copy ${label}`}>
        {state === 'copied' ? 'Copied' : state === 'failed' ? 'Copy failed — retry' : 'Copy'}
      </Button>
    </div>
    <code>{value}</code>
    <LiveRegion announcement={state === 'idle' ? null : { seq: 1, urgency: 'polite', text: state === 'copied' ? `${label} copied` : `Could not copy ${label}. Select and copy the text.` }} />
  </div>
}

// A hub id is <machine>::<local id>, and a local id usually starts with its worker's name: say that prefix once.
function idPrefix(ids: string[], machine: string, worker: string) {
  const base = machine && ids.every((id) => id.startsWith(`${machine}::`)) ? `${machine}::` : ''
  return ids.every((id) => id.startsWith(`${base}${worker}::`)) ? `${base}${worker}::` : base
}

// Collapsed to a count: one worker can register dozens of functions.
function FunctionList({ worker, machine, functions, lead, state }: { worker: string; machine: string; functions: EngineFunction[]; lead?: ReactNode; state?: string }) {
  const prefix = idPrefix(functions.map((fn) => fn.function_id), machine, worker)
  const count = `${functions.length} ${functions.length === 1 ? 'function' : 'functions'}`
  return <details className="mesh-ui-fns">
    <summary>{lead}<span className="mesh-ui-muted">{state ? `${state} · ${count}` : count}</span></summary>
    {prefix ? <p className="mesh-ui-muted">Ids start with <code>{prefix}</code></p> : null}
    {/* role="list": Safari drops list semantics from a list without markers. */}
    <ul role="list" className={functions.some((fn) => fn.description) ? 'mesh-ui-functions' : 'mesh-ui-functions mesh-ui-names'} aria-label={`${worker} functions`}>
      {functions.map((fn) => <li key={fn.function_id}>
        <code>{fn.function_id.slice(prefix.length)}</code>{fn.description ? <> <span className="mesh-ui-muted" title={fn.description}>{fn.description}</span></> : null}
      </li>)}
    </ul>
  </details>
}

function SharedWorkers({ node, functions, skipped }: { node: Node; functions: EngineFunction[]; skipped: boolean }) {
  const label = `mesh-node-${node.name}-workers`
  const workers = node.workers ?? []
  const described = new Map(functions.map((fn) => [fn.function_id, fn.description]))
  return <div className="mesh-ui-workers">
    <p className="mesh-ui-label" id={label}>Shared workers</p>
    {skipped ? <StatusPanel variant="warn" headline={<><code>{node.name}</code> shares more than the mesh carries; other machines cannot call it.</>} /> : null}
    {workers.length ? <ul role="list" aria-labelledby={label}>
      {workers.map((worker) => {
        // An offline machine's functions left with its bridge; the hub still holds the last ids it reported.
        const ids = node.online ? worker.functions ?? [] : []
        const lead = <code>{worker.name}</code>
        return <li key={worker.name}>
          {ids.length ? <FunctionList worker={worker.name} machine={node.name} lead={lead} state={worker.state}
            functions={ids.map((id) => ({ function_id: id, description: described.get(id) }))} />
            : <div className="mesh-ui-worker">{lead}<span className="mesh-ui-muted">
              {node.online ? `${worker.state} · No functions registered yet` : 'Unavailable while the machine is offline'}</span></div>}
        </li>
      })}
    </ul> : <p className="mesh-ui-muted">Not sharing any workers</p>}
  </div>
}

type Mode = 'hub' | 'client'
// The basics a connection needs. Everything else lives in the configuration form.
type Basics = { mode: Mode; public_url: string; tunnel: boolean; hub_url: string; join: string; name: string; share: string[]; hub_share: string[] }
const BASIC_PATHS: Record<keyof Basics, string[]> = {
  mode: ['mode'], public_url: ['public_url'], tunnel: ['tunnel', 'enabled'], hub_url: ['client', 'hub_url'], join: ['client', 'join'],
  name: ['client', 'name'], share: ['client', 'share'], hub_share: ['share'],
}
function basicsOf(config: JsonValue | null): Basics {
  const text = (path: string[]) => { const v = config === null ? undefined : at(config, path); return typeof v === 'string' ? v : '' }
  const list = (path: string[]) => { const v = config === null ? undefined : at(config, path); return Array.isArray(v) ? v.filter((s): s is string => typeof s === 'string') : [] }
  return {
    mode: text(BASIC_PATHS.mode) === 'client' ? 'client' : 'hub', public_url: text(BASIC_PATHS.public_url),
    tunnel: config !== null && at(config, BASIC_PATHS.tunnel) === true,
    hub_url: text(BASIC_PATHS.hub_url), join: text(BASIC_PATHS.join), name: text(BASIC_PATHS.name),
    share: list(BASIC_PATHS.share), hub_share: list(BASIC_PATHS.hub_share),
  }
}
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b)
const MODE_OPTIONS: { value: Mode; label: string; icon: false }[] = [
  { value: 'hub', label: 'Hub', icon: false }, { value: 'client', label: 'Client', icon: false },
]
const MODE_HELP: Record<Mode, string> = {
  hub: 'This project is the hub. Other projects join it as clients; the hub and every machine can call the workers the others share.',
  client: 'This project joins another project\'s hub. It shares the workers you pick below and can call what the hub and other machines share.',
}
// The mesh worker refuses these or never publishes their functions; listing them would only offer a switch that does nothing.
const NEVER_SHARED = new Set(['mesh', 'secrets', 'configuration', 'rbac-proxy', 'approval-gate'])
// Sharing one of these lets every caller in the mesh run commands where it runs.
const RUNS_COMMANDS = new Set(['ide', 'harness', 'shell', 'coder', 'tmux', 'code-runner'])
// What a share state the hub reports asks of the person, beyond its name.
const SHARE_HELP: Record<string, string> = {
  'name conflict': 'A machine has the name these functions start with. Rename or remove it to share this worker.',
  'too large': 'Too many functions to share.',
}
// A name the hub keeps as it is: no hyphen at the end or next to another.
const NAME = /^(?=.{1,31}$)[a-z](?:-?[a-z0-9])*$/
const RAW_CONFIG = 'The mesh settings are one raw value. Open Settings and replace it with individual settings first.'
const STATE_BADGE: Record<string, 'ok' | 'warn' | 'alert' | 'default'> = {
  connected: 'ok', 'restart required': 'warn', 'join failed': 'alert', error: 'alert', 'removed by hub': 'alert',
}
// What each client state means to the person watching: the headline, and what happens next or what to do.
const CLIENT_STATE: Record<string, [StatusPanelProps['variant'], string, string]> = {
  starting: ['info', 'Starting…', 'The mesh worker is reading its settings.'],
  'waiting for join': ['info', 'Not joined yet', 'Enter the hub\'s URL and join code, or a token, and save. The worker joins as soon as they are stored.'],
  joining: ['info', 'Joining the hub…', 'The worker is sending the join code to the hub.'],
  'waiting for hub': ['info', 'Waiting for the hub to accept this machine…', 'This takes up to a minute. Nothing to do.'],
  connecting: ['info', 'Connecting to the hub…', 'The hub knows this machine. The worker is waiting for its name and gateway.'],
  renaming: ['info', 'Renaming this machine…', 'Shared functions reconnect under the new name by themselves.'],
  error: ['alert', 'Not sharing', ''],
  'join failed': ['alert', 'The hub refused the join', 'Store another join code or token. This one is not tried again.'],
  'removed by hub': ['alert', 'The hub removed this machine', ''],
  'left hub': ['info', 'This engine left the hub', 'To join again, store a different join code or token. The one used before is not tried again.'],
}
const RETRYING = 'The worker keeps trying by itself.'
// The error is its own element, as the worker wrote it; the sentence after it starts on its own.
const said = (error: string | undefined, next: string) => <>{error ? <span>{error}</span> : null}{error && next ? (/[.!?]$/.test(error) ? ' ' : '. ') : null}{next}</>
// Client states with no enrollment to forget.
const NOTHING_TO_LEAVE = new Set(['', 'starting', 'waiting for join', 'joining', 'join failed', 'left hub'])

function JoinField({ value, onChange, secretInput: SecretKeyField }: { value: string; onChange: (next: string) => void; secretInput?: SecretInput }) {
  // A non-reference stays in this draft: the join code itself never reaches the versioned entry.
  const [draft, setDraft] = useState<string | null>(null)
  if (SecretKeyField) return <div data-field="client.join">
    {/* No `environment`: the worker reads env:// from its own process, not from the secrets worker's env file. */}
    <SecretKeyField name="MESH_JOIN" label="Join code or token" value={value || undefined} consumers={['mesh']} onChange={(next: string | undefined) => onChange(next ?? '')} />
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
  // An item of the settings list, and announced when the hub reports a change.
  return line ? <div role="listitem" aria-live="polite" className="mesh-ui-tunnel">{line}</div> : null
}

// The share switches of either mode. live: what the running worker reports; null while no worker of this mode runs.
function ShareList({ share, saved, live, workers, machine, place, calls, risk, footer, onChange }: {
  share: string[]; saved: string[]; live: Shared[] | null; workers: string[]; machine: string
  place: string; calls: ReactNode; risk: string; footer: string; onChange: (next: string[]) => void
}) {
  const shared = new Map((live ?? []).map((s) => [s.worker, s]))
  // A shared name that is not running still needs its switch, to turn it off.
  const names = [...new Set([...workers, ...share])].filter((n) => !NEVER_SHARED.has(n)).sort()
  const risky = share.filter((n) => RUNS_COMMANDS.has(n))
  // What the worker has not shown yet about a saved share switch.
  const pending = (name: string, on: boolean) => on !== saved.includes(name) ? (on ? 'not saved yet' : 'shared until you save')
    : on ? (shared.get(name)?.state ?? (live ? 'applying…' : 'starts after the restart'))
    : shared.has(name) ? 'stopping…' : null
  return <section className="mesh-ui-section" aria-labelledby="mesh-share-title">
    <div className="mesh-ui-section-head"><h2 id="mesh-share-title">Shared workers</h2>
      <span className="mesh-ui-muted">{share.length ? `${share.length} shared` : 'Nothing shared'} · {calls}</span>
    </div>
    {risky.length ? <StatusPanel variant="warn" headline={`${risky.join(', ')} can run commands on ${place}`} detail={risk} /> : null}
    {names.length === 0 ? <p className="mesh-ui-muted">No local workers found. Start a worker in this project to share it.</p> : null}
    <ul className="mesh-ui-share">{names.map((name) => {
      const on = share.includes(name)
      const state = shared.get(name)
      const label = pending(name, on)
      return <li key={name}>
        <label><Switch checked={on} aria-label={`Share ${name}`}
          onChange={(e) => onChange(e.currentTarget.checked ? [...share, name].sort() : share.filter((n) => n !== name))} />
          <code>{name}</code>
          {RUNS_COMMANDS.has(name) ? <Badge variant="warn">runs commands</Badge> : null}
          {label ? <span className="mesh-ui-muted">{label}</span> : null}
        </label>
        {on && state && SHARE_HELP[state.state] ? <p className="mesh-ui-muted">{SHARE_HELP[state.state]}</p> : null}
        {on && state?.functions?.length ? <FunctionList worker={name} machine={machine} functions={state.functions.map((id) => ({ function_id: id }))} /> : null}
      </li>
    })}</ul>
    {risky.length === 0 && names.length ? <p className="mesh-ui-muted">{footer}</p> : null}
  </section>
}

function ClientSetup({ basics, saved, edit, status, workers, functions, secretInput, onLeave }: {
  basics: Basics; saved: Basics; edit: <K extends keyof Basics>(key: K, value: Basics[K]) => void
  status: ClientStatus | null; workers: string[]; functions: EngineFunction[]; secretInput?: SecretInput
  onLeave: () => void
}) {
  const remote = status?.remote ?? []
  // The mirrors are local functions: the local listing has their descriptions.
  const described = new Map(functions.map((fn) => [fn.function_id, fn.description]))
  // The saved client.name; '' for a ${VAR} template, which only the worker expands, so it compares with nothing the worker reports.
  const wanted = saved.name.includes('${') ? '' : saved.name
  // The worker has not asked the hub for the saved name yet: it does on the event of the save.
  const renamePending = status?.state === 'connected' && wanted !== '' && status.requested_name !== wanted && !status.rename_error
  const state = renamePending ? 'renaming' : status?.state ?? ''
  const [variant, headline, next] = CLIENT_STATE[state] ?? (status?.error ? CLIENT_STATE.error : [])
  const renamed = state === 'connected' && wanted !== '' && status?.requested_name === wanted && status.node !== wanted
  const node = status?.node || basics.name || 'machine'
  return <>
    <section className="mesh-ui-section" aria-labelledby="mesh-connect-title">
      <div className="mesh-ui-section-head"><h2 id="mesh-connect-title">Hub connection</h2>
        {state ? <Badge variant={STATE_BADGE[state] ?? 'default'}>{state}</Badge> : null}
        {status?.node ? <span className="mesh-ui-muted">joined as <code>{status.node}</code>{status.namespace ? <> in namespace <code>{status.namespace}</code></> : null}</span> : null}
        {NOTHING_TO_LEAVE.has(state) ? null : <Button variant="ghost" size="sm" className="mesh-ui-head-action" onClick={onLeave}>Leave hub</Button>}
      </div>
      {/* Announced: the worker pushes each step of a join, and nobody is clicking Refresh. */}
      <div aria-live="polite" className="mesh-ui-progress">
        {headline ? <StatusPanel variant={status?.error && variant === 'info' ? 'warn' : variant} headline={headline}
          detail={said(status?.error, status?.retrying ? RETRYING : next ?? '')} /> : null}
        {/* Until the worker reads a save, the status keeps the failure of the name asked before. */}
        {status?.rename_error && saved.name !== '' && wanted !== status.requested_name ? <StatusPanel variant="warn" headline={`Could not rename this machine${wanted ? ` to ${wanted}` : ''}`}
          detail={said(status.rename_error, `It keeps the name ${status.node}.${status.retrying ? ` ${RETRYING}` : ''}`)} /> : null}
        {renamed ? <p className="mesh-ui-muted">The hub named this machine <code>{status?.node}</code>: <code>{wanted}</code> {NAME.test(wanted) ? 'is taken or reserved there.' : 'is not a valid machine name.'}</p> : null}
        {status?.join_unused ? <p className="mesh-ui-muted">Joined to <code>{status.hub_url}</code>. Another Hub URL or join code is used only after Leave hub.</p> : null}
      </div>
      {status?.hub_wants_version ? <StatusPanel variant="warn" headline={`The hub wants version ${status.hub_wants_version}`}
        detail="Nothing is shared or available until this project's mesh worker runs the hub's version." /> : null}
      <SettingsList>
        <SettingsField label="Hub URL" field="client.hub_url" description="The address the hub gave you. Not needed when you join with a token."
          renderControl={(props) => <Input {...props} placeholder="https://mesh.example.com" value={basics.hub_url} onChange={(next) => edit('hub_url', next)} />} />
        <JoinField value={basics.join} onChange={(next) => edit('join', next)} secretInput={secretInput} />
        <SettingsField label="Machine name" field="client.name" description="How this project appears on the hub. The hub may add a suffix such as -2."
          error={basics.name && !NAME.test(basics.name) ? 'Use lowercase letters, digits and single hyphens, starting with a letter and not ending with a hyphen.' : undefined}
          renderControl={(props) => <Input {...props} placeholder="my-laptop" value={basics.name} onChange={(next) => edit('name', next)} />} />
      </SettingsList>
    </section>
    <ShareList share={basics.share} saved={saved.share} live={status ? status.shared ?? [] : null} workers={workers} machine={status?.node ?? ''}
      place="this machine" calls={<>the hub calls them as <code>{node}::function</code>, other machines as <code>peer::{node}::function</code></>}
      risk="The hub project and every machine joined to the hub can call shared workers, and nothing asks here first, so sharing these gives all of them remote code execution here."
      footer="The hub project and every machine joined to the hub can call what you share." onChange={(next) => edit('share', next)} />
    <section className="mesh-ui-section" aria-labelledby="mesh-remote-title">
      <div className="mesh-ui-section-head"><h2 id="mesh-remote-title">Available from the mesh</h2></div>
      <div className="mesh-ui-workers">{remote.length ? <ul role="list" aria-labelledby="mesh-remote-title">
        {remote.map(({ machine, functions: ids }) => <li key={machine}>
          <FunctionList worker={machine} machine={machine === 'hub' ? 'hub' : `peer::${machine}`} lead={<code>{machine}</code>}
            functions={ids.map((id) => ({ function_id: id, description: described.get(id) }))} />
        </li>)}
      </ul> : <p className="mesh-ui-muted">{status?.state === 'connected' ? 'Nothing is shared with this machine yet' : 'Shown once this machine is connected'}</p>}</div>
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
  // A join replaces the code. Observed beside the hook, not as a source, so tab focus still re-reads the rest.
  const isHub = Boolean(data?.hub)
  useEffect(() => {
    if (!isHub) return
    try {
      return host.iii.observe({ function_id: 'mesh::on-node-changed', args: {} }, `iii::mesh-ui::events::${host.iii.browserId}`)
    } catch { return undefined }
  }, [host, isHub])
  // The worker calls this with every change of its mesh::status, in either mode: nothing here re-reads on a timer.
  useEffect(() => {
    try {
      return host.iii.observe('mesh::on-status-changed', `iii::mesh-ui::events::${host.iii.browserId}`)
    } catch { return undefined }
  }, [host])
  // What the last Save wrote, shown until the next read replaces it, so the old values never flash back.
  const [written, setWritten] = useState<JsonValue | null>(null)
  // Likewise the code mesh::join::cycle returned.
  const [cycled, setCycled] = useState<string | null>(null)
  useEffect(() => { setWritten(null); setCycled(null) }, [data])
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
  const [cycling, setCycling] = useState(false)
  const [cycleError, setCycleError] = useState<string | null>(null)
  // seq counts the announcements, so the same text is read again.
  const [notice, setNotice] = useState<{ seq: number; text: string } | null>(null)
  const announce = (text: string) => setNotice((prev) => ({ seq: (prev?.seq ?? 0) + 1, text }))
  const cycle = async () => {
    setCycling(true); setCycleError(null)
    try {
      setCycled((await host.iii.call<Join>('mesh::join::cycle', {})).code)
      announce('New join code ready. The old code no longer works.')
    } catch (err) { setCycleError(errorMessage(err)) } finally { setCycling(false); refresh() }
  }
  // ConfirmDialog shows why a confirmed action failed only for an Error; a failed call rejects with a plain object.
  const failed = (err: unknown) => new Error(errorMessage(err))
  // The machine whose removal is being confirmed.
  const [removing, setRemoving] = useState<string | null>(null)
  const remove = async (name: string) => {
    try {
      await host.iii.call('mesh::nodes::remove', { name })
      announce(`Removed ${name}. The join code was replaced.`)
    } catch (err) { throw failed(err) } finally { refresh() }
  }
  const [leaving, setLeaving] = useState(false)
  const leave = async () => {
    try {
      // A leave waits for a join in flight (up to a minute) and then up to 10 s for the hub.
      const left = await host.iii.call<Left>('mesh::leave', {}, { timeoutMs: 90_000 })
      announce(left?.warning ? `Left the hub. ${left.warning}` : 'Left the hub. This engine no longer shares workers.')
    } catch (err) { throw failed(err) } finally { refresh() }
  }
  const running: Mode = data?.client ? 'client' : 'hub'
  const hubView = basics.mode !== 'client' && running !== 'client'
  // Only the mode needs a restart: the worker picks hub or client when it starts.
  const needsRestart = !dirty && data !== undefined && config !== null && saved.mode !== running
  // A tunnel settles seconds to minutes after a save; the hub pushes each step.
  const tunnel = data?.hub?.tunnel
  const settling = Boolean(data?.hub) && (saved.tunnel !== Boolean(tunnel) || (tunnel !== undefined && !TUNNEL_RESTING.has(tunnel.state)))
  const [stalled, setStalled] = useState(false)
  useEffect(() => {
    setStalled(false)
    if (!settling) return
    // A deadline, not a poll: it only says the hub went quiet.
    const timer = setTimeout(() => setStalled(true), SETTLE_MS)
    return () => clearTimeout(timer)
    // `written` changes on every save, so each save gets a full window.
  }, [settling, written])
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
        headline={`Restart the mesh worker to switch to ${saved.mode} mode`}
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
            <Button variant="ghost" size="sm" className="mesh-ui-head-action" aria-label={`Remove ${node.name}`} onClick={() => setRemoving(node.name)}>Remove</Button>
          </header>
          <SharedWorkers node={node} functions={data.functions} skipped={data.hub?.catalog_skipped?.includes(node.name) ?? false} />
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
        ? <ClientSetup basics={basics} saved={saved} edit={edit} status={data?.client ?? null} workers={data?.workers ?? []} functions={data?.functions ?? []}
          secretInput={secretInput} onLeave={() => setLeaving(true)} />
        : <><ShareList share={basics.hub_share} saved={saved.hub_share} live={data?.hub ? data.hub.shared ?? [] : null} workers={data?.workers ?? []} machine="hub"
          place="this hub" calls={<>joined machines call them as <code>hub::function</code></>}
          risk="Every machine joined to this hub can call shared workers, and nothing asks here first, so sharing these gives each of them remote code execution with this project's access."
          footer="Every machine joined to this hub can call what you share." onChange={(next) => edit('hub_share', next)} />
        <section className="mesh-ui-section" aria-labelledby="mesh-address-title">
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
        </section></>}
      {dirty || failure ? <section className="mesh-ui-section mesh-ui-save" aria-live="polite">
        {failure ? <StatusPanel variant="alert" headline="Could not save" detail={failure} /> : null}
        {dirty ? <div className="mesh-ui-save-bar">
          <span className="mesh-ui-muted">Unsaved changes{changed.includes('mode') || saved.mode !== running ? ' · applied after a restart' : ' · applied within a few seconds'}</span>
          <Button variant="ghost" size="sm" disabled={busy !== null} onClick={() => setEdits({})}>Discard</Button>
          <Button size="sm" disabled={busy !== null || (basics.mode === 'client' && basics.name !== '' && !NAME.test(basics.name))} onClick={save}>{busy === 'save' ? 'Saving…' : 'Save'}</Button>
        </div> : null}
      </section> : null}
      {hubView ? <section className="mesh-ui-section" aria-labelledby="mesh-join-title">
        <div className="mesh-ui-section-head"><h2 id="mesh-join-title">Connect another engine</h2></div>
        <p className="mesh-ui-muted">On the other machine, open Mesh in that project's Console, choose Client, and paste these into Hub URL and Join code or token. If that field takes only a secret://NAME reference, store the code in the secrets worker and enter the reference. The code works once: after a machine joins with it, the hub replaces it. A machine that joins can call every worker shared by this hub and by the other machines. Share the join code only with people you trust.</p>
        {data?.joinError ? <StatusPanel variant="warn" headline="Join instructions unavailable" detail={data.joinError}
          action={<Button onClick={refresh}>Retry</Button>} /> : null}
        {cycleError ? <StatusPanel variant="alert" headline="Could not generate a new code" detail={cycleError} /> : null}
        {data?.join ? <div className="mesh-ui-join">
          {data.hub?.public_url ? <CopyValue label="Hub URL" value={data.hub.public_url} /> : null}
          <CopyValue label="Join code" value={cycled ?? data.join.code} action={<Button variant="ghost" size="sm" disabled={cycling} onClick={cycle}>
            {cycling ? 'Generating…' : 'Generate new code'}
          </Button>} />
        </div> : null}
      </section> : null}
      <LiveRegion announcement={notice ? { ...notice, urgency: 'polite' } : null} />
      <ConfirmDialog open={removing !== null} onOpenChange={(open) => { if (!open) setRemoving(null) }} tone="danger"
        title={`Remove ${removing ?? 'machine'}?`} confirmLabel="Remove machine" busyLabel="Removing…"
        description="It stops sharing its workers at once, loses access to everything shared and cannot reconnect with its current keys. The join code is replaced. To bring it back, choose Leave hub on that machine, then join with a new code."
        onConfirm={() => remove(removing ?? '')} />
      <ConfirmDialog open={leaving} onOpenChange={setLeaving} tone="danger" title="Leave the hub?" confirmLabel="Leave hub" busyLabel="Leaving…"
        description={data?.client?.state === 'removed by hub'
          ? 'The hub already removed this machine. Leaving deletes the saved join here, so this engine can join again with a new join code.'
          : 'This engine stops sharing its workers and loses access to everything shared. The hub drops this machine and the saved join is deleted. It joins again only with a different join code or token, as a new machine.'}
        onConfirm={leave} />
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
