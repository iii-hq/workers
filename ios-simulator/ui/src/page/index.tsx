/**
 * The Simulators page (terminal/instrument archetype): a device rail beside
 * one live iPhone. The rail lists the selected tenant's simulators, re-read
 * on every `ios-simulator::device-changed` event (so simulators booted from
 * Xcode or a terminal appear too); the stage shows the chosen simulator live
 * with its controls and media. Under the narrow threshold it becomes a
 * drill-in: list, then the phone with a labelled back action.
 */

import {
  EmptyState,
  Eyebrow,
  type Host,
  IconButton,
  List,
  ListGroup,
  ListGroupLabel,
  ListItem,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  Selector,
  Skeleton,
  StatusDot,
  StatusPanel,
} from '@iii-dev/console-ui'
import { useContainerNarrow, usePaneState, useWorkerLive } from '@iii-dev/console-ui/hooks'
import { Plus, Smartphone } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { DEFAULT_TENANT, DEVICE_CHANGED, type Device, isBooted, runtimeLabel, SimApi } from '../lib/api'
import { CreateDeviceDialog } from './CreateDevice'
import { Stage } from './Stage'

const NO_DEVICES: Device[] = []

function deviceDot(d: Device) {
  if (d.recording) return <StatusDot tone="alert" pulse aria-hidden />
  if (isBooted(d)) return <StatusDot tone="ok" pulse={d.streaming} aria-hidden />
  if (d.state === 'Booting' || d.state === 'Shutting Down') return <StatusDot tone="warn" pulse aria-hidden />
  return <StatusDot tone="ink" aria-hidden />
}

export function SimulatorsPage({
  host,
  panelSide = 'left',
  onRequestClose,
  paneId,
  tabId,
  panelContext,
  commands,
}: { host: Host } & Partial<PageRenderProps>) {
  const pane = paneId || tabId || 'default'
  const [tenant, setTenant] = usePaneState(`ios-simulator:${pane}:tenant`, DEFAULT_TENANT)
  const [selected, setSelected] = usePaneState<string | null>(`ios-simulator:${pane}:udid`, null)
  const [drilled, setDrilled] = useState(false)
  const [creating, setCreating] = useState(false)
  const [tenants, setTenants] = useState<string[]>([DEFAULT_TENANT])
  const [tenantQuery, setTenantQuery] = useState('')
  const api = useMemo(() => new SimApi(host.iii, tenant), [host, tenant])
  const { ref, narrow } = useContainerNarrow()

  const { data, loading, error, live, refresh } = useWorkerLive({
    iii: host.iii,
    triggers: [{ type: DEVICE_CHANGED, config: { tenant } }],
    fetch: () => api.devices(),
    handlerId: 'iii::ios-simulator-ui::devices',
  })
  const devices = data ?? NO_DEVICES

  // The binding follows the tenant; the list must too.
  // biome-ignore lint/correctness/useExhaustiveDependencies: refresh on tenant change only
  useEffect(() => refresh(), [tenant])

  useEffect(() => {
    void api
      .tenants()
      .then((list) => setTenants(list.includes(tenant) ? list : [...list, tenant].sort()))
      .catch(() => {})
  }, [api, tenant])

  // host.panels.open({ context: { tenant?, udid } }) — e.g. from a chat card.
  const appliedContext = useRef(0)
  useEffect(() => {
    if (!panelContext || panelContext.id === appliedContext.current) return
    appliedContext.current = panelContext.id
    const ctx = panelContext.context as { tenant?: unknown; udid?: unknown } | null
    if (typeof ctx?.tenant === 'string') setTenant(ctx.tenant)
    if (typeof ctx?.udid === 'string') {
      setSelected(ctx.udid)
      setDrilled(true)
    }
  }, [panelContext, setTenant, setSelected])

  const device = devices.find((d) => d.udid === selected) ?? null
  // A deleted or foreign selection falls back to the first booted device.
  useEffect(() => {
    if (loading || devices.length === 0 || device) return
    setSelected((devices.find(isBooted) ?? devices[0]).udid)
  }, [loading, devices, device, setSelected])

  useEffect(
    () =>
      commands?.register([
        {
          id: 'new-simulator',
          title: 'New simulator',
          keywords: ['create', 'iphone', 'device'],
          run: () => setCreating(true),
        },
      ]),
    [commands],
  )

  const booted = devices.filter(isBooted)
  const others = devices.filter((d) => !isBooted(d))
  const showRail = !narrow || !drilled || !device
  const showStage = !narrow || (drilled && device !== null)

  const open = (udid: string) => {
    setSelected(udid)
    setDrilled(true)
  }

  const row = (d: Device) => (
    <ListItem
      key={d.udid}
      selected={d.udid === selected}
      aria-current={d.udid === selected ? 'true' : undefined}
      leading={<Smartphone size={16} aria-hidden />}
      label={d.name}
      description={`${runtimeLabel(d.runtime)} · ${d.state}`}
      trailing={deviceDot(d)}
      onClick={() => open(d.udid)}
    />
  )

  return (
    <PageShell className="ios-ui-shell" ref={ref}>
      <PageHeader
        icon={<Smartphone size={16} aria-hidden />}
        title="Simulators"
        description={tenant === DEFAULT_TENANT ? 'iOS Simulators on this Mac' : `Tenant ${tenant}`}
        actions={
          <span className="ios-ui-live" title={live ? 'Live: updates as simulators change' : 'Polling every 15s'}>
            <StatusDot tone={live ? 'ok' : 'ink'} pulse={live} aria-hidden />
            {live ? 'live' : 'polling'}
          </span>
        }
        onClose={onRequestClose}
      />
      <PageBody side={panelSide} className="ios-ui-body">
        {showRail ? (
          <PageSidebar
            label="Simulators"
            side={panelSide}
            narrow={narrow}
            collapsible
            resizable
            defaultWidth={280}
            storageKey="ios-simulator:rail"
            header={
              <div className="ios-ui-rail-head">
                <Eyebrow as="span">simulators</Eyebrow>
                <IconButton label="New simulator" onClick={() => setCreating(true)}>
                  <Plus size={16} aria-hidden />
                </IconButton>
              </div>
            }
          >
            <div className="ios-ui-tenant">
              <Selector
                aria-label="Tenant"
                value={tenant}
                options={tenants.map((t) => ({
                  value: t,
                  label: t,
                  description: t === DEFAULT_TENANT ? 'This Mac' : undefined,
                }))}
                onChange={(next) => {
                  setTenant(next)
                  setSelected(null)
                }}
                query={tenantQuery}
                onQueryChange={setTenantQuery}
                onCreate={(name) => {
                  const slug = name.trim().toLowerCase()
                  if (!/^[a-z0-9][a-z0-9_-]{0,62}$/.test(slug)) return
                  setTenants((list) => (list.includes(slug) ? list : [...list, slug].sort()))
                  setTenant(slug)
                  setSelected(null)
                }}
                createOptionLabel={(q) => `Use tenant "${q.trim().toLowerCase()}"`}
                searchPlaceholder="Find or add a tenant"
              />
            </div>
            <div className="ios-ui-rail-scroll">
              {error ? (
                <StatusPanel variant="alert" role="alert" headline="Could not list simulators" detail={error} />
              ) : loading && devices.length === 0 ? (
                <div className="ios-ui-skeletons" aria-busy="true">
                  <Skeleton className="ios-ui-skeleton-row" />
                  <Skeleton className="ios-ui-skeleton-row" />
                  <Skeleton className="ios-ui-skeleton-row" />
                </div>
              ) : devices.length === 0 ? (
                <EmptyState
                  compact
                  icon={Smartphone}
                  title="No simulators"
                  description={
                    tenant === DEFAULT_TENANT
                      ? 'Create one, or install a runtime in Xcode.'
                      : `Tenant ${tenant} has no simulators yet.`
                  }
                  action={{ label: 'New simulator', onClick: () => setCreating(true) }}
                />
              ) : (
                <List>
                  {booted.length > 0 ? (
                    <ListGroup>
                      <ListGroupLabel>Running</ListGroupLabel>
                      {booted.map(row)}
                    </ListGroup>
                  ) : null}
                  {others.length > 0 ? (
                    <ListGroup>
                      <ListGroupLabel>Shut down</ListGroupLabel>
                      {others.map(row)}
                    </ListGroup>
                  ) : null}
                </List>
              )}
            </div>
          </PageSidebar>
        ) : null}
        {showStage ? (
          <PageMain className="ios-ui-main">
            {device ? (
              <Stage
                key={`${tenant}/${device.udid}`}
                host={host}
                api={api}
                device={device}
                commands={commands}
                onBack={narrow ? () => setDrilled(false) : undefined}
                onDeleted={() => setSelected(null)}
              />
            ) : (
              <EmptyState
                icon={Smartphone}
                title="Pick a simulator"
                description="Choose one on the left to watch and drive it live."
              />
            )}
          </PageMain>
        ) : null}
      </PageBody>
      <CreateDeviceDialog
        api={api}
        open={creating}
        onOpenChange={setCreating}
        onCreated={(udid) => {
          open(udid)
          refresh()
        }}
      />
    </PageShell>
  )
}
