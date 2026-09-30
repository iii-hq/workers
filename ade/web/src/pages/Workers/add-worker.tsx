import { errorMessage } from '@iii-dev/console-ui/format'
import { useDebounce } from '@iii-dev/console-ui/hooks'
import { Folder, Package } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { Button } from '@/components/ui/Button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { Eyebrow, eyebrowClassName } from '@/components/ui/Eyebrow'
import { Input } from '@/components/ui/Input'
import { List, ListItem } from '@/components/ui/List'
import { SegmentedControl } from '@/components/ui/ModeToggle'
import { SearchField } from '@/components/ui/SearchField'
import { Select } from '@/components/ui/Select'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { CardHighlight } from '@/components/ui/Surface'
import type {
  ComposeApi,
  Inspection,
  Project,
  RegistryWorker,
} from './compose-api'
import type { Actions } from './index'
import { basename, shortPath, usualParent } from './model'

type Mode = 'registry' | 'path'

export function AddWorkerDialog({
  open,
  onOpenChange,
  api,
  actions,
  project,
  onAdded,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  api: ComposeApi
  actions: Actions
  project: Project | null
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
      .track(`Adding ${plan.name}`, () => api.add([plan.input]))
      .then((ok) => ok && onAdded(plan.name))
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="wk-dialog">
        <DialogTitle>Add worker</DialogTitle>
        <DialogDescription>
          Declare a worker in the compose file, from the registry or from a
          directory on the daemon host.
        </DialogDescription>
        <div className="wk-dialog-body">
          <SegmentedControl
            variant="radio"
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
            <RegistryPick api={api} declared={declared} onPlan={setPlan} />
          ) : null}
          {open && mode === 'path' ? (
            <PathPick
              api={api}
              declared={declared}
              start={localRoot}
              onPlan={setPlan}
            />
          ) : null}
        </div>
        <div className="wk-dialog-footer">
          <span className="wk-note">{plan?.summary}</span>
          <Button variant="ghost" size="sm" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            size="sm"
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

function Dependencies({
  deps,
  declared,
}: {
  deps: string[]
  declared: Set<string>
}) {
  const kept = deps.filter((d) => declared.has(d))
  const other = deps.filter((d) => !declared.has(d))
  if (!deps.length) return null
  return (
    <CardHighlight className="wk-deps">
      {kept.length ? (
        <>
          <Eyebrow as="span">Already here</Eyebrow>
          <span className="wk-mono">{kept.join(', ')}</span>
        </>
      ) : null}
      {other.length ? (
        <>
          <Eyebrow as="span">Not declared</Eyebrow>
          <span className="wk-mono">{other.join(', ')}</span>
        </>
      ) : null}
    </CardHighlight>
  )
}

function RegistryPick({
  api,
  declared,
  onPlan,
}: {
  api: ComposeApi
  declared: Set<string>
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
    const missing = pick.dependencies.filter((d) => !declared.has(d))
    onPlan({
      name: pick.name,
      input: version ? `${pick.name}@${version}` : pick.name,
      summary: missing.length
        ? `Compose resolves ${missing.join(', ')} from the registry or the engine.`
        : '',
    })
  }, [pick, version, declared, onPlan])

  return (
    <>
      <SearchField
        aria-label="Search the registry"
        placeholder="Search the registry"
        value={query}
        onChange={setQuery}
        autoFocus
      />
      {error ? (
        <StatusPanel
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
              label={<span className="wk-mono">{worker.name}</span>}
              description={
                declared.has(worker.name)
                  ? 'Already in this project'
                  : worker.description
              }
              trailing={
                <span className="wk-mono wk-faint">{worker.version}</span>
              }
              onClick={() => setPick(worker)}
            />
          ))}
        </List>
      ) : null}
      {pick ? (
        <>
          <div className="wk-inline-field">
            <Eyebrow as="span">Version</Eyebrow>
            <Select
              aria-label="Version"
              value={version}
              onChange={setVersion}
              options={(versions.length ? versions : [pick.version]).map(
                (v, i) => ({
                  value: v,
                  label: i === 0 ? `${v} (latest)` : v,
                }),
              )}
            />
          </div>
          <Dependencies deps={pick.dependencies} declared={declared} />
        </>
      ) : null}
    </>
  )
}

function PathPick({
  api,
  declared,
  start,
  onPlan,
}: {
  api: ComposeApi
  declared: Set<string>
  start: string
  onPlan: (plan: Plan) => void
}) {
  const [path, setPath] = useState(start)
  const [found, setFound] = useState<Inspection | null>(null)
  const settled = useDebounce(path.trim(), 300)

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
  const kept =
    found?.manifest?.dependencies.filter((d) => declared.has(d)) ?? []

  useEffect(() => {
    const deps =
      found?.manifest?.dependencies.filter((d) => declared.has(d)) ?? []
    const key = found?.manifest ? basename(found.path) : null
    if (!found || !key || declared.has(key)) return onPlan(null)
    onPlan({
      name: key,
      input: {
        worker: `path://${found.path}`,
        ...(deps.length ? { start_after: deps } : {}),
      },
      summary: `Runs from ${shortPath(found.path)}`,
    })
  }, [found, declared, onPlan])

  const available = (found?.workers ?? []).filter(
    (w) => !declared.has(basename(w.path)),
  )

  return (
    <>
      <div className="wk-inline-field">
        <label htmlFor="wk-add-path" className={eyebrowClassName}>
          Directory
        </label>
        <Input
          id="wk-add-path"
          className="wk-mono"
          value={path}
          onChange={setPath}
          placeholder="/path/to/worker"
          spellCheck={false}
          autoFocus
        />
      </div>
      {!found && settled ? <Skeleton className="wk-skeleton-field" /> : null}
      {found && !found.exists ? (
        <StatusPanel
          variant="alert"
          headline="No such directory"
          detail={shortPath(found.path)}
        />
      ) : null}
      {found?.exists && !found.manifest && !available.length ? (
        <StatusPanel
          variant="warn"
          headline="No worker here"
          detail={`There is no iii.worker.yaml in ${shortPath(found.path)}, nor in the folders inside it.`}
        />
      ) : null}
      {available.length ? (
        <>
          <Eyebrow as="span">
            Not in this project yet · {available.length}
          </Eyebrow>
          <List aria-label="Workers in this folder" className="wk-results">
            {available.map((worker) => (
              <ListItem
                key={worker.path}
                label={<span className="wk-mono">{basename(worker.path)}</span>}
                description={worker.description ?? worker.language ?? undefined}
                onClick={() => setPath(worker.path)}
              />
            ))}
          </List>
        </>
      ) : null}
      {found?.manifest && name ? (
        taken ? (
          <StatusPanel
            variant="alert"
            headline={`${name} is already declared`}
            detail="Point the existing container at this directory from its Source tab."
          />
        ) : (
          <>
            <StatusPanel
              variant="success"
              headline={`iii.worker.yaml found: ${found.manifest.name}${found.manifest.language ? `, ${found.manifest.language}` : ''}`}
              detail={`The container is named ${name}, after the directory.${kept.length ? ` It starts after ${kept.join(', ')}.` : ''}`}
            />
            {found.manifest.dependencies.some((d) => !declared.has(d)) ? (
              <StatusPanel
                variant="warn"
                headline="Compose does not resolve a local worker's dependencies"
                detail={`Not in this project: ${found.manifest.dependencies.filter((d) => !declared.has(d)).join(', ')}. Add them first unless the engine provides them.`}
              />
            ) : null}
          </>
        )
      ) : null}
    </>
  )
}
