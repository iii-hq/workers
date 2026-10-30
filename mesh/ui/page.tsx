import { useCallback, useEffect, useRef, useState } from 'react'
import {
  Badge, Button, EmptyState, Input, LiveRegion, PageHeader, PageMain, PageShell,
  RawValueInput, SettingsField, SettingsList, SettingsSection, StatusPanel,
  type ConfigFormProps, type Host, type JsonValue, type PageRenderProps,
} from '@iii-dev/console-ui'
import { resolveConfigurationId } from '@iii-dev/console-ui/configuration'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { useCopyFlash, useWorkerLive } from '@iii-dev/console-ui/hooks'
import { Network, RefreshCw } from 'lucide-react'

type ComponentState = { name: string; state: string }
type Node = {
  name: string; address: string; os: string; arch: string; version: string
  online: boolean; last_seen: string; ephemeral: boolean; path: string
  workers: ComponentState[]; drives: ComponentState[]
}
type Join = { code: string; install: { unix: string; windows: string } }
const NO_TRIGGERS: readonly string[] = []

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

function Components({ label, values }: { label: string; values: ComponentState[] }) {
  return <div className="mesh-ui-components"><dt>{label}</dt><dd>
    {values.length ? values.map((item) => <span className="mesh-ui-component" key={item.name}>
      <code>{item.name}</code><span>{item.state}</span>
    </span>) : <span className="mesh-ui-muted">None reported</span>}
  </dd></div>
}

function MeshPage({ host, onRequestClose }: PageRenderProps & { host: Host }) {
  const fetch = useCallback(async () => {
    const [nodes, join] = await Promise.allSettled([
      host.iii.trigger<Node[]>('mesh::nodes::list', {}),
      host.iii.trigger<Join>('mesh::join::code', {}),
    ])
    return {
      nodes: nodes.status === 'fulfilled' ? nodes.value : null,
      nodesError: nodes.status === 'rejected' ? errorMessage(nodes.reason) : null,
      join: join.status === 'fulfilled' ? join.value : null,
      joinError: join.status === 'rejected' ? errorMessage(join.reason) : null,
    }
  }, [host])
  const { data, loading, error, refresh } = useWorkerLive({
    iii: host.iii, fetch, triggers: NO_TRIGGERS, handlerId: 'iii::mesh-ui::events',
  })
  return <PageShell className="mesh-ui-shell">
    <PageHeader icon={<Network />} title="Mesh" description="Your machines" onClose={onRequestClose}
      actions={<Button variant="ghost" size="sm" disabled={loading} onClick={refresh}>
        <RefreshCw size={16} aria-hidden />{loading ? 'Refreshing…' : 'Refresh'}
      </Button>} />
    <PageMain className="mesh-ui-main">
      <section className="mesh-ui-section" aria-labelledby="mesh-nodes-title" aria-busy={loading}>
        <div className="mesh-ui-section-head"><h2 id="mesh-nodes-title">Machines</h2>
          {data?.nodes ? <span className="mesh-ui-muted">{data.nodes.filter((n) => n.online).length} online · {data.nodes.length} total</span> : null}
        </div>
        {loading && !data ? <p role="status">Loading machines and join instructions…</p> : null}
        {error || data?.nodesError ? <StatusPanel variant="alert" headline="Could not load machines"
          detail={error || data?.nodesError} action={<Button onClick={refresh}>Retry</Button>} /> : null}
        {data?.nodes?.length === 0 ? <EmptyState icon={Network} title="No machines joined"
          description="Run a join command below on a machine to connect its workers to this project." /> : null}
        <div className="mesh-ui-nodes">{data?.nodes?.map((node) => <article key={node.name} className="mesh-ui-node" aria-label={node.name}>
          <header className="mesh-ui-node-head"><h3><code>{node.name}</code></h3>
            <Badge variant={node.online ? 'ok' : 'default'}>{node.online ? 'Online' : 'Offline'}</Badge>
            {node.ephemeral ? <Badge>Ephemeral</Badge> : null}
          </header>
          <dl className="mesh-ui-details">
            <div><dt>Address</dt><dd><code>{node.address}</code></dd></div>
            <div><dt>Platform</dt><dd>{node.os || 'Unknown'}{node.arch ? ` / ${node.arch}` : ''}</dd></div>
            <div><dt>Version</dt><dd><code>{node.version || 'Not reported'}</code></dd></div>
            <div><dt>Path</dt><dd><code>{node.path || 'Not reported'}</code></dd></div>
            {!node.online ? <div><dt>Last seen</dt><dd>{node.last_seen && !node.last_seen.startsWith('0001-') ? formatRelative(node.last_seen) : 'Not reported'}</dd></div> : null}
            <Components label="Workers" values={node.workers ?? []} />
            <Components label="Drives" values={node.drives ?? []} />
          </dl>
        </article>)}</div>
      </section>
      <section className="mesh-ui-section" aria-labelledby="mesh-join-title">
        <div className="mesh-ui-section-head"><h2 id="mesh-join-title">Join a machine</h2></div>
        <p className="mesh-ui-muted">Run the command as the person who will use the machine. Share the join code only with people you trust.</p>
        {data?.joinError ? <StatusPanel variant="warn" headline="Join instructions unavailable" detail={data.joinError}
          action={<Button onClick={refresh}>Retry</Button>} /> : null}
        {data?.join ? <div className="mesh-ui-join">
          <CopyValue label="Join code" value={data.join.code} />
          <CopyValue label="Linux / macOS command" value={data.join.install.unix} />
          <CopyValue label="Windows PowerShell command" value={data.join.install.windows} />
        </div> : null}
      </section>
    </PageMain>
  </PageShell>
}

function object(value: JsonValue | undefined): Record<string, JsonValue> {
  return value && typeof value === 'object' && !Array.isArray(value) ? value : {}
}
function at(value: JsonValue, path: string[]): JsonValue {
  let current: JsonValue | undefined = value
  for (const key of path) current = Array.isArray(current) ? current[Number(key)] : object(current)[key]
  return current ?? null
}
function setAt(value: JsonValue, path: string[], next: JsonValue): JsonValue {
  if (!path.length) return next
  const [key, ...rest] = path
  if (Array.isArray(value)) return value.map((item, i) => i === Number(key) ? setAt(item, rest, next) : item)
  const record = object(value)
  return { ...record, [key]: setAt(record[key] ?? null, rest, next) }
}
function pointer(path: string[]) { return '/' + path.map((s) => s.replaceAll('~', '~0').replaceAll('/', '~1')).join('/') }

type FormFieldProps = { form: ConfigFormProps; path: string[]; label: string; description?: string; numeric?: boolean }
function Field({ form, path, label, description, numeric }: FormFieldProps) {
  const value = at(form.value, path)
  return <SettingsField label={label} description={description} field={path.join('.')} error={form.errors?.get(pointer(path))}
    renderControl={(props) => <Input {...props} value={typeof value === 'string' || typeof value === 'number' ? String(value) : ''}
      onChange={(next) => form.onChange(setAt(form.value, path, numeric && /^\d+$/.test(next) ? Number(next) : next))} />} />
}
function StringList({ form, path, label, description }: FormFieldProps) {
  const value = at(form.value, path)
  if (value !== null && !Array.isArray(value)) return <Field form={form} path={path} label={label} description={description} />
  const values = Array.isArray(value) ? value : []
  return <SettingsSection title={label} description={description} action={<Button variant="ghost" size="sm" onClick={() => form.onChange(setAt(form.value, path, [...values, '']))}>Add item</Button>}>
    <SettingsList>{values.map((_, index) => <div className="mesh-ui-form-item" key={index}>
      <Field form={form} path={[...path, String(index)]} label={`${label} ${index + 1}`} />
      <Button variant="ghost" size="sm" aria-label={`Remove ${label} ${index + 1}`} onClick={() => form.onChange(setAt(form.value, path, values.filter((_, i) => i !== index)))}>Remove</Button>
    </div>)}{values.length === 0 ? <p className="mesh-ui-form-empty">No items configured.</p> : null}</SettingsList>
  </SettingsSection>
}
function Collection({ form, kind }: { form: ConfigFormProps; kind: 'nodes' | 'drives' }) {
  const [name, setName] = useState('')
  const collectionValue = at(form.value, [kind])
  const entries = object(collectionValue)
  const validName = (kind === 'drives' ? /^(?=.{1,31}$)[a-z0-9]+(-[a-z0-9]+)*$/.test(name) : /^[a-z][a-z0-9-]{0,30}$/.test(name)) && !Object.hasOwn(entries, name)
  const label = kind === 'nodes' ? 'Machine' : 'Drive'
  if (typeof collectionValue === 'string') return <SettingsSection title={kind === 'nodes' ? 'Machine workers' : 'Shared drives'}>
    <RawValueInput label={kind} value={collectionValue} kind="environment" replacementLabel="Use literal settings"
      onUseLiteral={() => form.onChange(setAt(form.value, [kind], {}))}
      onChange={(next) => form.onChange(setAt(form.value, [kind], next))} />
  </SettingsSection>
  return <SettingsSection title={kind === 'nodes' ? 'Machine workers' : 'Shared drives'}
    description={kind === 'nodes' ? 'Choose which workers each machine runs and any additional function grants.' : 'A drive server must be a joined Linux machine. Drive names identify shared mounts.'}>
    {Object.keys(entries).map((key) => <details className="mesh-ui-config-entry" key={key} open>
      <summary><code>{key}</code></summary>
      {typeof entries[key] === 'string' ? <RawValueInput label={key} value={entries[key]} kind="environment" replacementLabel="Use literal settings"
        onUseLiteral={() => form.onChange(setAt(form.value, [kind, key], kind === 'nodes' ? { workers: [], allow: [] } : { server: '', nodes: [] }))}
        onChange={(next) => form.onChange(setAt(form.value, [kind, key], next))} /> : kind === 'nodes' ? <>
        <StringList form={form} path={[kind, key, 'workers']} label="Workers" description="Worker names, optionally worker@version." />
        <StringList form={form} path={[kind, key, 'allow']} label="Additional grants" description="Extra hub function patterns allowed for this machine." />
      </> : <>
        <Field form={form} path={[kind, key, 'server']} label="Server machine" />
        <StringList form={form} path={[kind, key, 'nodes']} label="Client machines" description="Use * for every persistent machine, or list machine names." />
      </>}
      <Button variant="ghost" size="sm" onClick={() => {
        const next = { ...entries }; delete next[key]
        form.onChange(setAt(form.value, [kind], next))
      }}>Remove {label.toLowerCase()} {key}</Button>
    </details>)}
    <div className="mesh-ui-form-add">
      <Input aria-label={`New ${label.toLowerCase()} name`} placeholder={`${label} name`} value={name} onChange={setName} />
      <Button size="sm" disabled={!validName} onClick={() => {
        form.onChange(setAt(form.value, [kind], { ...entries, [name]: kind === 'nodes' ? { workers: [], allow: [] } : { server: '', nodes: [] } }))
        setName('')
      }}>Add {label.toLowerCase()}</Button>
    </div>
    {name && !validName ? <p role="status" className="mesh-ui-muted">{kind === 'nodes' ? 'Use a unique lowercase name, starting with a letter, up to 31 letters, digits or hyphens.' : 'Use a unique name up to 31 lowercase letters or digits, separated by single hyphens.'}</p> : null}
  </SettingsSection>
}

function MeshConfiguration(form: ConfigFormProps) {
  const root = useRef<HTMLDivElement>(null)
  const focus = form.focusField?.join('.')
  useEffect(() => {
    if (!focus) return
    const target = root.current?.querySelector<HTMLElement>(`[data-field="${CSS.escape(focus)}"]`)
    target?.focus(); target?.scrollIntoView({ block: 'center' })
  }, [focus])
  const links = at(form.value, ['links'])
  const pairs = Array.isArray(links) ? links : []
  return <div className="mesh-ui-configuration" ref={root}>
    <SettingsSection title="Connection" description="Set the public URL before joining machines. Network changes reconfigure the hub.">
      <SettingsList>
        <Field form={form} path={['public_url']} label="Public URL" description="HTTPS front door reachable by your machines." />
        <Field form={form} path={['listen']} label="Listen address" />
        <Field form={form} path={['gateway_port']} label="Gateway port" numeric />
        <Field form={form} path={['proxy_url']} label="Proxy URL" description="Direct numeric loopback WebSocket endpoint for rbac-proxy." />
        <Field form={form} path={['stun_port']} label="STUN port" description="Use 0 to disable STUN." numeric />
      </SettingsList>
    </SettingsSection>
    <Collection form={form} kind="nodes" />
    <Collection form={form} kind="drives" />
    <details className="mesh-ui-advanced"><summary>Advanced networking</summary>
      <StringList form={form} path={['trusted_proxies']} label="Trusted proxies" description="CIDRs trusted to supply forwarded client addresses." />
      <SettingsSection title="Machine links" description="Explicit pairs of machines allowed to connect."
        action={typeof links === 'string' ? undefined : <Button variant="ghost" size="sm" onClick={() => form.onChange(setAt(form.value, ['links'], [...pairs, ['', '']]))}>Add link</Button>}>
        {typeof links === 'string' ? <Field form={form} path={['links']} label="Machine links template" /> : null}
        {pairs.map((_, i) => <SettingsList key={i}>
          <Field form={form} path={['links', String(i), '0']} label={`Link ${i + 1} first machine`} />
          <Field form={form} path={['links', String(i), '1']} label={`Link ${i + 1} second machine`} />
          <Button variant="ghost" size="sm" onClick={() => form.onChange(setAt(form.value, ['links'], pairs.filter((_, index) => index !== i)))}>Remove link {i + 1}</Button>
        </SettingsList>)}
      </SettingsSection>
    </details>
    {form.errors?.size ? <StatusPanel variant="alert" headline="Check the configuration fields" detail={[...form.errors.values()].join(' · ')} /> : null}
  </div>
}

export default async function setup(host: Host) {
  const configurationId = await resolveConfigurationId(host.iii, 'mesh')
  host.configForms.register('mesh', MeshConfiguration)
  host.pages.register({ id: 'mesh-manager', title: 'Mesh', configurationId,
    render: (props) => <MeshPage host={host} {...props} />,
  })
}
