import { errorMessage } from '@iii-dev/console-ui/format'
import { Play, RotateCw, ShieldCheck, Square } from 'lucide-react'
import { useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { Eyebrow } from '@/components/ui/Eyebrow'
import { StatusDot } from '@/components/ui/StatusDot'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { Card, CardBody, CardHeader } from '@/components/ui/Surface'
import {
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@/components/ui/Table'
import type { ComposeApi, Snapshot } from './compose-api'
import type { Actions, Latest } from './index'
import { shortPath, startWaves, toneFor, waveLabel } from './model'

type Validation =
  | { ok: true; order: string[]; deferred: string[] }
  | { ok: false; error: string }

export function ProjectView({
  api,
  actions,
  snapshot,
  latest,
  onSelect,
}: {
  api: ComposeApi
  actions: Actions
  snapshot: Snapshot
  latest: Latest
  onSelect: (name: string) => void
}) {
  const [validation, setValidation] = useState<Validation | null>(null)
  const { status, project, list } = snapshot
  const containers = status.containers ?? []
  const state = new Map(containers.map((c) => [c.container, c.state]))
  const ready = containers.filter((c) => c.state === 'ready').length
  const failed = containers.filter((c) => c.state === 'failed').length
  const waves = startWaves(project?.containers ?? [])
  const locals = (project?.containers ?? []).filter(
    (c) => c.source === 'path',
  ).length
  const packages = (project?.containers ?? []).filter(
    (c) => c.source === 'package',
  )
  const outdated = packages.filter((c) => latest.outdated(c.name))
  const { busy, confirm } = actions

  const lifecycle = async (verb: 'up' | 'down' | 'restart') => {
    if (verb !== 'up') {
      const ok = await confirm({
        title:
          verb === 'down'
            ? 'Stop every container?'
            : 'Restart every container?',
        description:
          verb === 'down'
            ? 'Compose stops the whole project in reverse dependency order. The daemon keeps running.'
            : 'Compose restarts the whole project in dependency order.',
        confirmLabel: verb === 'down' ? 'Stop all' : 'Restart all',
        tone: verb === 'down' ? 'danger' : 'default',
      })
      if (!ok) return
    }
    const label = {
      up: 'Starting the project',
      down: 'Stopping the project',
      restart: 'Restarting the project',
    }[verb]
    await actions.run(label, () => api.lifecycle(verb))
  }

  const validate = async () => {
    try {
      const result = await api.validate()
      setValidation({
        ok: true,
        order: result.start_order,
        deferred: result.deferred_packages,
      })
    } catch (cause) {
      setValidation({ ok: false, error: errorMessage(cause) })
    }
  }

  const update = async (names: string[]) => {
    const targets = names.map((name) => `${name}@${latest.of(name)}`)
    const ok = await confirm({
      title:
        names.length === 1
          ? `Update ${names[0]} to ${latest.of(names[0])}?`
          : `Update ${names.length} packages?`,
      description: `Compose resolves ${names.length === 1 ? 'it and its' : 'them and their'} dependencies again, and restarts the whole project when what runs changes.`,
      details: names.length > 1 ? targets : undefined,
      confirmLabel: 'Update and restart',
    })
    if (!ok) return
    await actions.track(
      names.length === 1
        ? `Updating ${names[0]} to ${latest.of(names[0])}`
        : `Updating ${names.length} packages`,
      () => api.update(targets),
    )
  }

  const stopDaemon = async () => {
    const ok = await confirm({
      title: 'Stop the compose daemon?',
      description:
        'Every project it supervises goes down and the daemon exits. This page has nothing to show until a daemon returns.',
      confirmLabel: 'Stop daemon',
      tone: 'danger',
    })
    if (!ok) return
    await actions.run('Stopping the daemon', async () => {
      await api.stopDaemon()
      return { status: 'ok' }
    })
  }

  return (
    <div className="wk-view">
      <div className="wk-masthead">
        <div className="wk-identity">
          <div className="wk-title-row">
            <h2 className="wk-title wk-mono">
              {status.namespace ?? 'Project'}
            </h2>
            {failed ? <Badge variant="alert">{failed} failed</Badge> : null}
            <Badge>
              {ready} of {containers.length} ready
            </Badge>
          </div>
          <div className="wk-meta wk-mono">
            {status.file ? (
              <span title={status.file}>{shortPath(status.file)}</span>
            ) : null}
            {status.daemon_pid ? (
              <span>daemon pid {status.daemon_pid}</span>
            ) : null}
          </div>
        </div>
        <div className="wk-actions">
          <Button
            variant="ghost"
            size="sm"
            disabled={busy}
            onClick={() => void lifecycle('up')}
          >
            <Play />
            Start all
          </Button>
          <Button
            variant="ghost"
            size="sm"
            disabled={busy}
            onClick={() => void lifecycle('restart')}
          >
            <RotateCw />
            Restart all
          </Button>
          <Button
            variant="ghost"
            size="sm"
            disabled={busy}
            onClick={() => void lifecycle('down')}
          >
            <Square />
            Stop all
          </Button>
          <Button variant="pill" size="sm" onClick={() => void validate()}>
            <ShieldCheck />
            Validate file
          </Button>
        </div>
      </div>

      {validation ? (
        validation.ok ? (
          <StatusPanel
            variant="success"
            headline="The compose file is valid"
            detail={
              validation.deferred.length
                ? `${validation.order.length} containers in start order. Resolved on the next start: ${validation.deferred.join(', ')}.`
                : `${validation.order.length} containers in start order.`
            }
          />
        ) : (
          <StatusPanel
            variant="alert"
            headline="The compose file does not validate"
            detail={validation.error}
          />
        )
      ) : null}

      <Card>
        <CardHeader className="wk-card-header">
          Start order
          <span className="wk-card-note">
            from <span className="wk-mono">start_after</span>; each step waits
            for the one above
          </span>
        </CardHeader>
        <CardBody className="wk-waves">
          {waves.map((wave, index) => (
            <div key={wave.join()} className="wk-wave">
              <span className="wk-wave-label">
                <span className="wk-mono wk-faint">{index + 1}</span>
                {index === 0
                  ? 'With the engine'
                  : waveLabel(wave, project?.containers ?? [])}
              </span>
              <div className="wk-pills">
                {wave.map((name) => (
                  <Button
                    key={name}
                    variant="pill"
                    size="sm"
                    className="h-7 px-[11px] text-[12px]"
                    onClick={() => onSelect(name)}
                  >
                    <StatusDot
                      tone={toneFor(state.get(name) ?? 'stopped').dot}
                      aria-label={state.get(name) ?? 'stopped'}
                    />
                    <span className="wk-mono">{name}</span>
                  </Button>
                ))}
              </div>
            </div>
          ))}
        </CardBody>
      </Card>

      <div className="wk-columns">
        <Card>
          <CardHeader className="wk-card-header">
            Registry packages
            {outdated.length ? (
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
                onClick={() => void update(outdated.map((c) => c.name))}
              >
                Update {outdated.length} package
                {outdated.length === 1 ? '' : 's'}
              </Button>
            ) : null}
          </CardHeader>
          <CardBody>
            {packages.length === 0 ? (
              <p className="wk-note">Every container runs from a local path.</p>
            ) : (
              <TableViewport>
                <TableFrame>
                  <Table density="compact" inset>
                    <TableHeader>
                      <TableRow>
                        <TableHead>Package</TableHead>
                        <TableHead>Declared</TableHead>
                        <TableHead>Latest</TableHead>
                        <TableHead aria-label="Action" />
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {packages.map((c) => {
                        const newer = latest.outdated(c.name)
                        return (
                          <TableRow
                            key={c.name}
                            interactive
                            onClick={() => onSelect(c.name)}
                          >
                            <TableCell className="wk-mono">{c.name}</TableCell>
                            <TableCell className="wk-mono wk-faint">
                              {c.version ?? 'unpinned'}
                            </TableCell>
                            <TableCell>
                              {newer ? (
                                <Badge variant="accent">{newer}</Badge>
                              ) : (
                                <span className="wk-faint">
                                  {latest.of(c.name) ? 'up to date' : '–'}
                                </span>
                              )}
                            </TableCell>
                            <TableCell className="wk-cell-action">
                              {newer ? (
                                <Button
                                  variant="ghost"
                                  size="sm"
                                  disabled={busy}
                                  onClick={(event) => {
                                    event.stopPropagation()
                                    void update([c.name])
                                  }}
                                >
                                  Update
                                </Button>
                              ) : null}
                            </TableCell>
                          </TableRow>
                        )
                      })}
                    </TableBody>
                  </Table>
                </TableFrame>
              </TableViewport>
            )}
            {locals ? (
              <p className="wk-note wk-card-foot">
                {locals === 1
                  ? 'The local-path worker follows its checkout.'
                  : `The ${locals} local-path workers follow their checkout.`}{' '}
                Point one elsewhere from its Source tab.
              </p>
            ) : null}
          </CardBody>
        </Card>

        <div className="wk-stack">
          <Card>
            <CardHeader className="wk-card-header">Compose file</CardHeader>
            <CardBody>
              <dl className="wk-facts">
                <Eyebrow as="dt">File</Eyebrow>
                <dd className="wk-mono" title={status.file}>
                  {status.file ? shortPath(status.file) : '–'}
                </dd>
                <Eyebrow as="dt">Namespace</Eyebrow>
                <dd className="wk-mono">{status.namespace ?? '–'}</dd>
                <Eyebrow as="dt">Engine</Eyebrow>
                <dd className="wk-mono">
                  {project?.engine_url ?? 'compose default'}
                </dd>
                <Eyebrow as="dt">Timeouts</Eyebrow>
                <dd className="wk-mono">
                  start {project?.startup_timeout ?? '–'} · stop{' '}
                  {project?.stop_timeout ?? '–'}
                </dd>
                <Eyebrow as="dt">State</Eyebrow>
                <dd className="wk-mono" title={status.state_dir}>
                  {status.state_dir ? shortPath(status.state_dir) : '–'}
                </dd>
              </dl>
            </CardBody>
          </Card>
          <Card>
            <CardHeader className="wk-card-header">
              Daemon
              <span className="wk-card-note wk-mono">
                {(list?.daemon_pid ?? status.daemon_pid)
                  ? `pid ${list?.daemon_pid ?? status.daemon_pid}`
                  : ''}
              </span>
            </CardHeader>
            <CardBody>
              <dl className="wk-facts">
                {(list?.projects ?? []).map((p) => (
                  <div key={`${p.namespace}:${p.file}`} className="wk-fact-row">
                    <Eyebrow as="dt">Supervises</Eyebrow>
                    <dd>
                      <span className="wk-mono">{p.namespace ?? '–'}</span>{' '}
                      <span className="wk-faint">
                        {
                          (p.containers ?? []).filter(
                            (c) => c.state === 'ready',
                          ).length
                        }{' '}
                        of {p.containers?.length ?? 0} ready
                        {p.file === status.file ? ' (this page)' : ''}
                      </span>
                    </dd>
                  </div>
                ))}
              </dl>
              <div className="wk-danger-row">
                <p className="wk-note">
                  Stopping the daemon takes down every project it supervises.
                </p>
                <Button
                  variant="ghost"
                  size="sm"
                  className="wk-danger"
                  disabled={busy}
                  onClick={() => void stopDaemon()}
                >
                  Stop daemon…
                </Button>
              </div>
            </CardBody>
          </Card>
        </div>
      </div>
    </div>
  )
}
