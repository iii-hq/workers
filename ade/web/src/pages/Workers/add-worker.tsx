import { errorMessage } from '@iii-dev/console-ui/format'
import { useDebounce } from '@iii-dev/console-ui/hooks'
import { Folder, Package } from 'lucide-react'
import { Fragment, useEffect, useMemo, useState } from 'react'
import { Button } from '@/components/shadcn/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/shadcn/dialog'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/shadcn/select'
import { List, ListItem } from '@/components/ui/List'
import { SegmentedControl } from '@/components/ui/ModeToggle'
import { Skeleton } from '@/components/ui/Skeleton'
import { FieldLabel, Notice, SearchBox, TextInput } from './components/kit'
import type {
  ComposeApi,
  Inspection,
  Project,
  RegistryWorker,
} from './compose-api'
import type { Actions } from './index'
import {
  alsoStarts,
  basename,
  defaultRun,
  shortPath,
  usualParent,
} from './model'

type Mode = 'registry' | 'path'

export function AddWorkerDialog({
  open,
  onOpenChange,
  api,
  actions,
  project,
  engine,
  onAdded,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  api: ComposeApi
  actions: Actions
  project: Project | null
  /** Workers the engine itself provides, such as configuration. */
  engine: ReadonlySet<string>
  onAdded: (name: string) => void
}) {
  const [mode, setMode] = useState<Mode>('registry')
  const declared = useMemo(
    () => new Set((project?.containers ?? []).map((c) => c.name)),
    [project],
  )
  const localRoot = useMemo(
    () =>
      usualParent(
        (project?.containers ?? [])
          .filter((c) => c.source === 'path')
          .map((c) => c.ref),
      ) ?? '',
    [project],
  )
  const [plan, setPlan] = useState<{
    name: string
    input: string | Record<string, unknown>
    summary: string
  } | null>(null)

  const add = () => {
    if (!plan) return
    onOpenChange(false)
    void actions
      .track(`Adding ${plan.name}`, (operationId) =>
        api.add([plan.input], operationId),
      )
      .then((ok) => ok && onAdded(plan.name))
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="wk-surface wk-dialog wk-dialog-box gap-0 p-0 [--dialog-max-width:38rem]">
        <div className="flex flex-col gap-1.5 px-6 pt-6 pb-4">
          <DialogTitle className="pr-8 font-inter text-[16px] font-semibold leading-6 text-neutral-900 dark:text-neutral-50">
            Add worker
          </DialogTitle>
          <DialogDescription className="font-inter text-[13px] leading-5 text-neutral-600 dark:text-neutral-400">
            Declare a worker in the compose file, from the registry or from a
            directory on the daemon host.
          </DialogDescription>
        </div>
        <div className="wk-dialog-body">
          <SegmentedControl
            variant="radio"
            className="wk-seg"
            aria-label="Where the worker comes from"
            value={mode}
            onChange={(next) => {
              setMode(next)
              setPlan(null)
            }}
            options={[
              { value: 'registry', label: 'Registry', icon: <Package /> },
              { value: 'path', label: 'Local path', icon: <Folder /> },
            ]}
          />
          {open && mode === 'registry' ? (
            <RegistryPick
              api={api}
              declared={declared}
              engine={engine}
              onPlan={setPlan}
            />
          ) : null}
          {open && mode === 'path' ? (
            <PathPick
              api={api}
              declared={declared}
              engine={engine}
              start={localRoot}
              onPlan={setPlan}
            />
          ) : null}
          {plan && actions.idle.length ? (
            <Notice
              variant="info"
              headline="Stopped workers start too"
              detail={`${alsoStarts(actions.idle)[0]}: Compose brings every declared container up after an add.`}
            />
          ) : null}
        </div>
        <div className="wk-dialog-footer">
          <span className="wk-note">{plan?.summary}</span>
          <Button
            variant="outline"
            size="lg"
            className="wk-press font-inter"
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button
            variant="default"
            size="lg"
            className="wk-press font-inter"
            disabled={!plan || actions.busy}
            onClick={add}
          >
            {plan ? `Add ${plan.name}` : 'Add worker'}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

type Plan = {
  name: string
  input: string | Record<string, unknown>
  summary: string
} | null

/** A worker's dependencies, split by where each one comes from. */
function Dependencies({
  deps,
  declared,
  engine,
  keptLabel = 'Already here',
}: {
  deps: readonly string[]
  declared: Set<string>
  engine: ReadonlySet<string>
  keptLabel?: string
}) {
  const rows: [string, string[]][] = [
    [keptLabel, deps.filter((d) => declared.has(d))],
    ['From the engine', deps.filter((d) => !declared.has(d) && engine.has(d))],
    ['Not declared', deps.filter((d) => !declared.has(d) && !engine.has(d))],
  ]
  if (!deps.length) return null
  return (
    <div className="wk-deps">
      {rows.map(([label, names]) =>
        names.length ? (
          <Fragment key={label}>
            <FieldLabel>{label}</FieldLabel>
            <span className="wk-machine">{names.join(', ')}</span>
          </Fragment>
        ) : null,
      )}
    </div>
  )
}

function RegistryPick({
  api,
  declared,
  engine,
  onPlan,
}: {
  api: ComposeApi
  declared: Set<string>
  engine: ReadonlySet<string>
  onPlan: (plan: Plan) => void
}) {
  const [query, setQuery] = useState('')
  const [results, setResults] = useState<RegistryWorker[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [pick, setPick] = useState<RegistryWorker | null>(null)
  const [versions, setVersions] = useState<string[]>([])
  const [version, setVersion] = useState<string | undefined>(undefined)
  const settled = useDebounce(query.trim(), 250)

  useEffect(() => {
    if (!settled) {
      setResults(null)
      return
    }
    let cancelled = false
    setError(null)
    api
      .search(settled)
      .then((found) => !cancelled && setResults(found.workers))
      .catch((cause) => !cancelled && setError(errorMessage(cause)))
    return () => {
      cancelled = true
    }
  }, [api, settled])

  useEffect(() => {
    setVersions([])
    setVersion(pick?.version)
    if (!pick) return
    let cancelled = false
    api
      .packageVersions(pick.name)
      .then(
        (found) =>
          !cancelled && setVersions(found.versions.map((v) => v.version)),
      )
      .catch(() => undefined)
    return () => {
      cancelled = true
    }
  }, [api, pick])

  useEffect(() => {
    if (!pick || declared.has(pick.name)) return onPlan(null)
    const missing = pick.dependencies.filter(
      (d) => !declared.has(d) && !engine.has(d),
    )
    onPlan({
      name: pick.name,
      input: version ? `${pick.name}@${version}` : pick.name,
      summary: missing.length
        ? `Compose resolves ${missing.join(', ')} from the registry or the engine.`
        : '',
    })
  }, [pick, version, declared, engine, onPlan])

  return (
    <>
      <SearchBox
        aria-label="Search the registry"
        placeholder="Search the registry"
        value={query}
        onChange={setQuery}
        autoFocus
      />
      {error ? (
        <Notice
          variant="alert"
          headline="The registry did not answer"
          detail={error}
        />
      ) : null}
      {settled && !results && !error ? (
        <Skeleton className="wk-skeleton-block" />
      ) : null}
      {results && !results.length ? (
        <p className="wk-note">
          Nothing in the registry is called “{settled}”.
        </p>
      ) : null}
      {results?.length ? (
        <List aria-label="Registry workers" className="wk-results">
          {results.map((worker) => (
            <ListItem
              key={worker.name}
              selected={pick?.name === worker.name}
              disabled={declared.has(worker.name)}
              label={worker.name}
              description={
                declared.has(worker.name)
                  ? 'Already in this project'
                  : worker.description
              }
              trailing={
                <span className="wk-machine wk-faint">{worker.version}</span>
              }
              onClick={() => setPick(worker)}
            />
          ))}
        </List>
      ) : null}
      {pick ? (
        <>
          <div className="wk-inline-field">
            <FieldLabel>Version</FieldLabel>
            <Select
              value={version ?? null}
              onValueChange={(next) => setVersion(next ?? undefined)}
            >
              <SelectTrigger
                aria-label="Version"
                className="wk-select wk-machine w-full max-w-[20rem]"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent className="wk-surface wk-popover">
                {(versions.length ? versions : [pick.version]).map((v, i) => (
                  <SelectItem key={v} value={v} className="wk-machine">
                    {i === 0 ? `${v} (latest)` : v}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <Dependencies
            deps={pick.dependencies}
            declared={declared}
            engine={engine}
          />
        </>
      ) : null}
    </>
  )
}

function PathPick({
  api,
  declared,
  engine,
  start,
  onPlan,
}: {
  api: ComposeApi
  declared: Set<string>
  engine: ReadonlySet<string>
  start: string
  onPlan: (plan: Plan) => void
}) {
  const [path, setPath] = useState(start)
  const [found, setFound] = useState<Inspection | null>(null)
  const settled = useDebounce(path.trim(), 300)
  // A manifest with no `scripts.start` gives compose nothing to start: the
  // entry carries a `run`, a default offered where there is one.
  const needsRun = Boolean(found?.manifest && !found.manifest.start)
  const [run, setRun] = useState('')
  useEffect(() => {
    setRun(found?.manifest ? defaultRun(found.manifest) : '')
  }, [found])

  useEffect(() => {
    setFound(null)
    if (!settled) return
    let cancelled = false
    api
      .inspect(settled)
      .then((result) => !cancelled && setFound(result))
      .catch(() => undefined)
    return () => {
      cancelled = true
    }
  }, [api, settled])

  const name = found?.manifest ? basename(found.path) : null
  const taken = name ? declared.has(name) : false
  const missing =
    found?.manifest?.dependencies.filter(
      (d) => !declared.has(d) && !engine.has(d),
    ) ?? []

  useEffect(() => {
    const deps =
      found?.manifest?.dependencies.filter((d) => declared.has(d)) ?? []
    const key = found?.manifest ? basename(found.path) : null
    if (!found || !key || declared.has(key)) return onPlan(null)
    const command = run.trim()
    if (needsRun && !command) return onPlan(null)
    onPlan({
      name: key,
      input: {
        worker: `path://${found.path}`,
        ...(deps.length ? { start_after: deps } : {}),
        ...(needsRun ? { scripts: { run: command } } : {}),
      },
      summary: `Runs from ${shortPath(found.path)}`,
    })
  }, [found, declared, onPlan, needsRun, run])

  const available = (found?.workers ?? []).filter(
    (w) => !declared.has(basename(w.path)),
  )

  return (
    <>
      <div className="wk-inline-field">
        <FieldLabel as="label" htmlFor="wk-add-path">
          Directory
        </FieldLabel>
        <TextInput
          id="wk-add-path"
          className="wk-machine"
          value={path}
          onChange={setPath}
          placeholder="/path/to/worker"
          spellCheck={false}
          autoFocus
        />
      </div>
      {!found && settled ? <Skeleton className="wk-skeleton-field" /> : null}
      {found && !found.exists ? (
        <Notice
          variant="alert"
          headline="No such directory"
          detail={shortPath(found.path)}
        />
      ) : null}
      {found?.exists && !found.manifest && !available.length ? (
        <Notice
          variant="warn"
          headline="No worker here"
          detail={`There is no iii.worker.yaml in ${shortPath(found.path)}, nor in the folders inside it.`}
        />
      ) : null}
      {available.length ? (
        <>
          <FieldLabel>Not in this project yet · {available.length}</FieldLabel>
          <List aria-label="Workers in this folder" className="wk-results">
            {available.map((worker) => (
              <ListItem
                key={worker.path}
                label={basename(worker.path)}
                description={worker.description ?? worker.language ?? undefined}
                onClick={() => setPath(worker.path)}
              />
            ))}
          </List>
        </>
      ) : null}
      {found?.manifest && name ? (
        taken ? (
          <Notice
            variant="alert"
            headline={`${name} is already declared`}
            detail="Point the existing container at this directory from its Source tab."
          />
        ) : (
          <>
            <Notice
              variant="success"
              headline={`iii.worker.yaml found: ${found.manifest.name}${found.manifest.language ? `, ${found.manifest.language}` : ''}`}
              detail={`The container is named ${name}, after the directory.`}
            />
            <Dependencies
              deps={found.manifest.dependencies}
              declared={declared}
              engine={engine}
              keptLabel="Starts after"
            />
            {missing.length ? (
              <Notice
                variant="warn"
                headline="Compose does not resolve a local worker's dependencies"
                detail={`Not in this project: ${missing.join(', ')}. Add them first.`}
              />
            ) : null}
            {needsRun ? (
              <>
                <div className="wk-inline-field">
                  <FieldLabel as="label" htmlFor="wk-add-run">
                    Start command
                  </FieldLabel>
                  <TextInput
                    id="wk-add-run"
                    className="wk-machine"
                    value={run}
                    onChange={setRun}
                    placeholder="./target/release/worker"
                    spellCheck={false}
                  />
                </div>
                {run.trim() ? null : (
                  <Notice
                    variant="warn"
                    headline="No start command"
                    detail="Its iii.worker.yaml has no scripts.start, so compose needs one here to start it."
                  />
                )}
              </>
            ) : null}
          </>
        )
      ) : null}
    </>
  )
}
