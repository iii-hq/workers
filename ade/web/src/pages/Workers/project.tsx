import { errorMessage } from '@iii-dev/console-ui/format'
import {
  ArrowUpCircle,
  Play,
  RotateCw,
  ShieldCheck,
  Square,
} from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { Badge } from '@/components/shadcn/badge'
import { Button } from '@/components/shadcn/button'
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/shadcn/card'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/shadcn/table'
import { cn } from '@/lib/utils'
import { Notice } from './components/kit'
import type { ComposeApi, Snapshot } from './compose-api'
import type { Actions, Latest } from './index'
import {
  inPlaceVersion,
  shortPath,
  startWaves,
  toneFor,
  waveLabel,
  withAlsoStarts,
} from './model'

type Validation =
  | { ok: true; order: string[]; deferred: string[] }
  | { ok: false; error: string }

/** A worker's status dot, in the pills and the table. */
const DOT: Record<string, string> = {
  accent: 'bg-ok',
  alert: 'bg-alert',
  warn: 'bg-warn',
  ink: 'bg-neutral-400 dark:bg-neutral-600',
  ok: 'bg-ok',
  ghost: 'bg-neutral-300 dark:bg-neutral-700',
}

export function WorkerPill({
  name,
  state,
  onClick,
}: {
  name: string
  state: string
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="wk-press inline-flex h-7 items-center gap-2 rounded-lg border border-neutral-200 bg-neutral-50 px-3 font-inter text-[13px] font-medium text-neutral-800 transition-[background-color,color,border-color] duration-150 ease-out outline-none hover:border-neutral-300 hover:bg-neutral-100 hover:text-neutral-950 focus-visible:ring-2 focus-visible:ring-neutral-500 dark:border-neutral-800 dark:bg-neutral-900/60 dark:text-neutral-200 dark:hover:border-neutral-700 dark:hover:bg-neutral-800 dark:hover:text-neutral-50 dark:focus-visible:ring-neutral-400"
    >
      <span
        aria-hidden
        className={cn('size-2 rounded-full', DOT[toneFor(state).dot])}
      />
      {name}
      <span className="sr-only">{state}</span>
    </button>
  )
}

/** A key/value row in the facts cards: label, then a machine value. */
function Fact({
  label,
  children,
  title,
  mono = true,
}: {
  label: string
  children: ReactNode
  title?: string
  mono?: boolean
}) {
  return (
    <>
      <dt className="font-inter text-[12px] font-medium leading-5 text-neutral-500 dark:text-neutral-400">
        {label}
      </dt>
      <dd
        className={cn(
          'min-w-0 truncate text-[13px] leading-5 text-neutral-900 dark:text-neutral-100',
          mono ? 'wk-machine' : 'font-inter',
        )}
        title={title}
      >
        {children}
      </dd>
    </>
  )
}

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
  const known = packages.filter((c) => latest.of(c.name)).length
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
    const chosen = packages.filter((c) => names.includes(c.name))
    // In place when every one carries its package's name: only they restart.
    const inPlace =
      chosen.length === names.length && chosen.every(inPlaceVersion)
    const one = names.length === 1
    const ok = await confirm({
      title: one
        ? `Update ${names[0]} to ${latest.of(names[0])}?`
        : `Update ${names.length} packages?`,
      description: withAlsoStarts(
        `${one ? '' : `${targets.join(', ')}. `}${
          inPlace
            ? `Compose installs ${one ? 'it' : 'them'} and restarts only ${one ? names[0] : `these ${names.length} containers`}; the rest keep running.`
            : `Compose resolves ${one ? 'it and its' : 'them and their'} dependencies again, and restarts the whole project when what runs changes.`
        }`,
        actions.idle,
        names,
      ),
      confirmLabel: inPlace ? 'Update' : 'Update and restart all',
    })
    if (!ok) return
    await actions.track(
      one
        ? `Updating ${names[0]} to ${latest.of(names[0])}`
        : `Updating ${names.length} packages`,
      (operationId) =>
        inPlace
          ? api.setVersions(
              chosen.map((c) => ({
                ref: c.ref,
                version: latest.of(c.name) ?? 'latest',
              })),
              operationId,
            )
          : api.update(targets, operationId),
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
            <h2 className="wk-title">{status.namespace ?? 'Project'}</h2>
            {failed ? (
              <Badge variant="destructive" className="font-inter">
                {failed} failed
              </Badge>
            ) : null}
            <Badge variant="secondary" className="wk-badge font-inter">
              {ready} of {containers.length} ready
            </Badge>
          </div>
          <div className="wk-meta">
            {status.file ? (
              <span className="wk-machine" title={status.file}>
                {shortPath(status.file)}
              </span>
            ) : null}
            {status.daemon_pid ? (
              <span className="wk-machine">daemon pid {status.daemon_pid}</span>
            ) : null}
          </div>
        </div>
        <div className="wk-actions">
          <Button
            variant="ghost"
            className="wk-press font-inter"
            disabled={busy}
            onClick={() => void lifecycle('up')}
          >
            <Play data-icon="inline-start" />
            Start all
          </Button>
          <Button
            variant="ghost"
            className="wk-press font-inter"
            disabled={busy}
            onClick={() => void lifecycle('restart')}
          >
            <RotateCw data-icon="inline-start" />
            Restart all
          </Button>
          <Button
            variant="ghost"
            className="wk-press font-inter"
            disabled={busy}
            onClick={() => void lifecycle('down')}
          >
            <Square data-icon="inline-start" />
            Stop all
          </Button>
          <Button
            variant="outline"
            className="wk-press font-inter"
            onClick={() => void validate()}
          >
            <ShieldCheck data-icon="inline-start" />
            Validate file
          </Button>
        </div>
      </div>

      {validation ? (
        validation.ok ? (
          <Notice
            variant="success"
            headline="The compose file is valid"
            detail={
              validation.deferred.length
                ? `${validation.order.length} containers in start order. Resolved on the next start: ${validation.deferred.join(', ')}.`
                : `${validation.order.length} containers in start order.`
            }
          />
        ) : (
          <Notice
            variant="alert"
            headline="The compose file does not validate"
            detail={validation.error}
          />
        )
      ) : null}

      <Card size="sm" className="wk-card">
        <CardHeader>
          <CardTitle className="wk-card-title">Start order</CardTitle>
          <CardDescription className="wk-card-note">
            From <span className="wk-machine">start_after</span>; each step
            waits for the one above.
          </CardDescription>
        </CardHeader>
        <CardContent className="wk-waves">
          {waves.map((wave, index) => (
            <div key={wave.join()} className="wk-wave">
              <span className="wk-wave-label">
                <span className="wk-step" aria-hidden>
                  {index + 1}
                </span>
                <span className="sr-only">Step {index + 1}:</span>
                {index === 0
                  ? 'With the engine'
                  : waveLabel(wave, project?.containers ?? [])}
              </span>
              <div className="wk-pills">
                {wave.map((name) => (
                  <WorkerPill
                    key={name}
                    name={name}
                    state={state.get(name) ?? 'stopped'}
                    onClick={() => onSelect(name)}
                  />
                ))}
              </div>
            </div>
          ))}
        </CardContent>
      </Card>

      <div className="wk-columns">
        <Card size="sm" className="wk-card">
          <CardHeader>
            <CardTitle className="wk-card-title">Registry packages</CardTitle>
            <CardDescription className="wk-card-note">
              {packages.length === 0
                ? 'Every container runs from a local path.'
                : outdated.length
                  ? `${outdated.length} of ${packages.length} ${outdated.length === 1 ? 'has' : 'have'} a newer release.`
                  : known === 0
                    ? 'Looking up the registry…'
                    : `All ${packages.length} up to date.`}
            </CardDescription>
            {outdated.length ? (
              <CardAction>
                <Button
                  variant="outline"
                  className="wk-press font-inter"
                  disabled={busy}
                  onClick={() => void update(outdated.map((c) => c.name))}
                >
                  <ArrowUpCircle data-icon="inline-start" />
                  Update {outdated.length} package
                  {outdated.length === 1 ? '' : 's'}
                </Button>
              </CardAction>
            ) : null}
          </CardHeader>
          {packages.length === 0 ? null : (
            <CardContent className="px-0!">
              <Table className="wk-table">
                <TableHeader>
                  <TableRow className="hover:bg-transparent">
                    <TableHead className="pl-4">Package</TableHead>
                    <TableHead>Declared</TableHead>
                    <TableHead>Latest</TableHead>
                    <TableHead className="pr-4 text-right">
                      <span className="sr-only">Action</span>
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {packages.map((c) => {
                    const newer = latest.outdated(c.name)
                    return (
                      <TableRow
                        key={c.name}
                        className="cursor-pointer"
                        onClick={() => onSelect(c.name)}
                        onKeyDown={(event) => {
                          if (event.key === 'Enter' || event.key === ' ') {
                            event.preventDefault()
                            onSelect(c.name)
                          }
                        }}
                        tabIndex={0}
                      >
                        <TableCell className="pl-4">
                          <span className="inline-flex items-center gap-2.5">
                            <span
                              aria-hidden
                              className={cn(
                                'size-2 shrink-0 rounded-full',
                                DOT[
                                  toneFor(state.get(c.name) ?? 'stopped').dot
                                ],
                              )}
                            />
                            <span className="font-inter text-[13px] font-medium text-neutral-900 dark:text-neutral-100">
                              {c.name}
                            </span>
                          </span>
                        </TableCell>
                        <TableCell className="wk-machine text-neutral-500 dark:text-neutral-400">
                          {c.version ?? 'unpinned'}
                        </TableCell>
                        <TableCell>
                          {newer ? (
                            <Badge className="wk-machine">{newer}</Badge>
                          ) : (
                            <span className="font-inter text-[12px] text-neutral-500 dark:text-neutral-400">
                              {latest.of(c.name) ? 'up to date' : '–'}
                            </span>
                          )}
                        </TableCell>
                        <TableCell className="pr-3 text-right">
                          {newer ? (
                            <Button
                              variant="ghost"
                              className="wk-press font-inter"
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
              {locals ? (
                <p className="wk-note px-4 pt-3">
                  {locals === 1
                    ? 'The local-path worker follows its checkout.'
                    : `The ${locals} local-path workers follow their checkout.`}{' '}
                  Point one elsewhere from its Source tab.
                </p>
              ) : null}
            </CardContent>
          )}
        </Card>

        <div className="wk-stack">
          <Card size="sm" className="wk-card">
            <CardHeader>
              <CardTitle className="wk-card-title">Compose file</CardTitle>
            </CardHeader>
            <CardContent>
              <dl className="wk-facts">
                <Fact label="File" title={status.file}>
                  {status.file ? shortPath(status.file) : '–'}
                </Fact>
                <Fact label="Namespace">{status.namespace ?? '–'}</Fact>
                <Fact label="Engine">
                  {project?.engine_url ?? 'compose default'}
                </Fact>
                <Fact label="Timeouts">
                  start {project?.startup_timeout ?? '–'} · stop{' '}
                  {project?.stop_timeout ?? '–'}
                </Fact>
                <Fact label="State" title={status.state_dir}>
                  {status.state_dir ? shortPath(status.state_dir) : '–'}
                </Fact>
              </dl>
            </CardContent>
          </Card>
          <Card size="sm" className="wk-card">
            <CardHeader>
              <CardTitle className="wk-card-title">Daemon</CardTitle>
              {(list?.daemon_pid ?? status.daemon_pid) ? (
                <CardDescription className="wk-card-note wk-machine">
                  pid {list?.daemon_pid ?? status.daemon_pid}
                </CardDescription>
              ) : null}
            </CardHeader>
            <CardContent className="flex flex-col gap-3">
              <dl className="wk-facts">
                {(list?.projects ?? []).map((p) => (
                  <Fact
                    key={`${p.namespace}:${p.file}`}
                    label="Supervises"
                    mono={false}
                  >
                    <span className="font-medium">{p.namespace ?? '–'}</span>{' '}
                    <span className="text-neutral-500 dark:text-neutral-400">
                      {
                        (p.containers ?? []).filter((c) => c.state === 'ready')
                          .length
                      }{' '}
                      of {p.containers?.length ?? 0} ready
                      {p.file === status.file ? ' · this page' : ''}
                    </span>
                  </Fact>
                ))}
              </dl>
              <div className="wk-danger-row">
                <p className="wk-note">
                  Stopping the daemon takes down every project it supervises.
                </p>
                <Button
                  variant="destructive"
                  className="wk-press shrink-0 font-inter"
                  disabled={busy}
                  onClick={() => void stopDaemon()}
                >
                  Stop daemon…
                </Button>
              </div>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  )
}
