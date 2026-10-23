/**
 * What listens on this machine: every `iii compose` stack with its engine
 * port and its workers' ports, then every other listening socket of this
 * user's processes. A location jumps to the tmux window the process runs in.
 */

import {
  Button,
  Card,
  CardBody,
  CardHeader,
  Chip,
  EmptyState,
  Eyebrow,
  MetaRow,
  type MetaRowItem,
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
import { Boxes, ExternalLink, Network } from 'lucide-react'
import { groupPorts, locationLabel, portUrl, shortPath, stackListeners } from '../lib/model'
import type { ComposeStack, Listener, PaneRef, PortsSnapshot } from '../lib/types'

export interface PortsViewProps {
  ports: PortsSnapshot | null
  loading: boolean
  home?: string
  onSelectLocation(location: PaneRef): void
}

function PortLink({ listener }: { listener: Pick<Listener, 'port' | 'role'> }) {
  const url = portUrl(listener)
  if (!url) return <span className="tmux-mono">:{listener.port}</span>
  return (
    <a className="tmux-port-link tmux-mono" href={url} target="_blank" rel="noreferrer" title={`Open ${url}`}>
      :{listener.port}
      <ExternalLink className="tmux-meta-icon" aria-hidden />
    </a>
  )
}

function LocationButton({ location, onSelect }: { location: PaneRef | null; onSelect(location: PaneRef): void }) {
  if (!location) return <span className="tmux-faint">—</span>
  return (
    <Button
      size="sm"
      variant="ghost"
      className="tmux-mono"
      title={`Show ${location.session} window ${location.window_index} (${location.pane_id})`}
      onClick={() => onSelect(location)}
    >
      {locationLabel(location)}
    </Button>
  )
}

function ComposeCard({
  stack,
  listeners,
  home,
  onSelectLocation,
}: {
  stack: ComposeStack
  listeners: Listener[]
  home?: string
  onSelectLocation(location: PaneRef): void
}) {
  const items: MetaRowItem[] = [
    { label: 'project', value: shortPath(stack.project_dir, home) || '—' },
    { label: 'pid', value: String(stack.pid) },
  ]
  if (stack.compose_file) items.push({ label: 'file', value: shortPath(stack.compose_file, home) })
  if (stack.engine) {
    items.push({
      label: 'engine',
      value: stack.engine.ws_port !== null ? `ws://127.0.0.1:${stack.engine.ws_port}` : `pid ${stack.engine.pid}`,
      tone: 'accent',
    })
  }
  const portsByPid = new Map<number, Listener[]>()
  for (const listener of listeners) {
    const list = portsByPid.get(listener.pid) ?? []
    list.push(listener)
    portsByPid.set(listener.pid, list)
  }
  const engineOthers = stack.engine ? stack.engine.ports.filter((port) => port !== stack.engine?.ws_port) : []

  return (
    <Card className="tmux-compose">
      <CardHeader className="tmux-compose-header">
        <span className="tmux-compose-title">
          <Boxes className="tmux-meta-icon" aria-hidden />
          <span>iii compose</span>
          <Chip tone="neutral" className="tmux-chip tmux-mono">
            {stack.namespace}
          </Chip>
        </span>
        <LocationButton location={stack.location} onSelect={onSelectLocation} />
      </CardHeader>
      <CardBody className="tmux-compose-body">
        <MetaRow items={items} />
        {engineOthers.length > 0 ? (
          <p className="tmux-faint tmux-compose-note">
            The engine also listens on{' '}
            {engineOthers.map((port) => (
              <span key={port} className="tmux-mono">
                :{port}{' '}
              </span>
            ))}
          </p>
        ) : null}
        {stack.workers.length > 0 ? (
          <TableViewport>
            <TableFrame>
              <Table density="compact" inset>
                <TableHeader>
                  <TableRow>
                    <TableHead>Worker</TableHead>
                    <TableHead>PID</TableHead>
                    <TableHead>Ports</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {stack.workers.map((worker) => {
                    const owned = portsByPid.get(worker.pid) ?? []
                    return (
                      <TableRow key={`${worker.name}:${worker.pid}`}>
                        <TableCell className="tmux-mono">{worker.name}</TableCell>
                        <TableCell className="tmux-mono tmux-num">{worker.pid}</TableCell>
                        <TableCell>
                          {worker.ports.length === 0 ? (
                            <span className="tmux-faint">—</span>
                          ) : (
                            <span className="tmux-port-list">
                              {worker.ports.map((port) => (
                                <PortLink
                                  key={port}
                                  listener={{
                                    port,
                                    role: owned.find((listener) => listener.port === port)?.role ?? 'worker',
                                  }}
                                />
                              ))}
                            </span>
                          )}
                        </TableCell>
                      </TableRow>
                    )
                  })}
                </TableBody>
              </Table>
            </TableFrame>
          </TableViewport>
        ) : (
          <p className="tmux-faint">No worker processes found under this daemon yet.</p>
        )}
      </CardBody>
    </Card>
  )
}

export function PortsView({ ports, loading, home, onSelectLocation }: PortsViewProps) {
  if (!ports) {
    return loading ? (
      <div className="tmux-ports-empty">
        <p className="tmux-faint">Scanning listening ports…</p>
      </div>
    ) : (
      <div className="tmux-ports-empty">
        <EmptyState icon={Network} title="No port scan yet" description="The tmux worker has not reported its port scan." />
      </div>
    )
  }
  const { composes, others } = groupPorts(ports)

  return (
    <div className="tmux-ports">
      {!ports.supported ? (
        <StatusPanel
          variant="info"
          headline="Port scanning is not available here"
          detail={ports.error ?? 'This platform has no port scanner the tmux worker can use.'}
        />
      ) : ports.error ? (
        <StatusPanel variant="warn" headline="The last port scan failed" detail={ports.error} />
      ) : null}

      <section className="tmux-ports-section" aria-label="iii compose stacks">
        <Eyebrow as="h3">iii compose</Eyebrow>
        {composes.length === 0 ? (
          <p className="tmux-faint">No `iii compose` daemon is running.</p>
        ) : (
          composes.map((stack) => (
            <ComposeCard
              key={`${stack.namespace}:${stack.pid}`}
              stack={stack}
              listeners={stackListeners(ports, stack)}
              home={home}
              onSelectLocation={onSelectLocation}
            />
          ))
        )}
      </section>

      <section className="tmux-ports-section" aria-label="Other listening ports">
        <Eyebrow as="h3">Other listening ports</Eyebrow>
        {ports.supported && others.length === 0 ? (
          <p className="tmux-faint">Nothing else is listening.</p>
        ) : others.length > 0 ? (
          <TableViewport>
            <TableFrame>
              <Table density="compact">
                <TableHeader>
                  <TableRow>
                    <TableHead>Port</TableHead>
                    <TableHead>Address</TableHead>
                    <TableHead>Process</TableHead>
                    <TableHead>Worker</TableHead>
                    <TableHead>tmux</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {others.map((listener) => (
                    <TableRow key={`${listener.family}:${listener.address}:${listener.port}:${listener.pid}`}>
                      <TableCell>
                        <PortLink listener={listener} />
                      </TableCell>
                      <TableCell className="tmux-mono">{listener.address}</TableCell>
                      <TableCell className="tmux-mono" title={`pid ${listener.pid}`}>
                        {listener.process}
                      </TableCell>
                      <TableCell className="tmux-mono">{listener.worker ?? '—'}</TableCell>
                      <TableCell>
                        <LocationButton location={listener.location} onSelect={onSelectLocation} />
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </TableFrame>
          </TableViewport>
        ) : null}
      </section>
    </div>
  )
}
