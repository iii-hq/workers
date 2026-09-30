import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { useDebounce } from '@iii-dev/console-ui/hooks'
import { Folder, Package } from 'lucide-react'
import { type ReactNode, useEffect, useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { FileDiff } from '@/components/ui/FileDiff'
import { Input } from '@/components/ui/Input'
import { List, ListItem } from '@/components/ui/List'
import { SegmentedControl } from '@/components/ui/ModeToggle'
import { Selector } from '@/components/ui/Selector'
import { SettingsSection } from '@/components/ui/Settings'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { Card, CardBody, CardHeader } from '@/components/ui/Surface'
import type {
  ComposeApi,
  DeclaredContainer,
  Inspection,
  Versions,
} from './compose-api'
import type { Actions } from './index'
import { basename, entryYaml, shortPath } from './model'

type Props = {
  api: ComposeApi
  actions: Actions
  declared: DeclaredContainer
  onOpenSettings: () => void
}

const FILE = 'worker-compose.yaml'
const CHECKOUTS = 5
const KINDS = [
  { value: 'package' as const, label: 'Registry package', icon: <Package /> },
  { value: 'path' as const, label: 'Local path', icon: <Folder /> },
]

export function SourceTab(props: Props) {
  const [kind, setKind] = useState<'package' | 'path'>(
    props.declared.source === 'package' ? 'package' : 'path',
  )
  const switching = kind !== props.declared.source
  return (
    <div className="wk-form">
      <SettingsSection title="Source">
        <SegmentedControl
          variant="radio"
          aria-label="Where the worker comes from"
          value={kind}
          onChange={setKind}
          options={KINDS}
        />
      </SettingsSection>
      {switching ? (
        <StatusPanel
          variant="info"
          headline="That replaces the container"
          detail={`A registry package and a local checkout are not versions of each other. To switch, remove ${props.declared.name} and add the other kind from Add worker; its environment and scripts do not carry over.`}
        />
      ) : props.declared.source === 'package' ? (
        <PackageSource {...props} />
      ) : (
        <PathSource {...props} />
      )}
    </div>
  )
}

function Preview({
  before,
  after,
  children,
}: {
  before: string
  after: string
  children: ReactNode
}) {
  return (
    <Card className="wk-preview">
      <CardHeader className="wk-card-header">
        Change preview
        <span className="wk-card-note wk-mono">{FILE}</span>
      </CardHeader>
      <CardBody className="wk-preview-body">
        <FileDiff
          oldFile={{ name: FILE, contents: before }}
          newFile={{ name: FILE, contents: after }}
          disableFileHeader
          expandUnchanged
        />
        {children}
      </CardBody>
    </Card>
  )
}

type Policy = 'pin' | 'latest' | 'next'

function PackageSource({ api, actions, declared }: Props) {
  const [versions, setVersions] = useState<Versions | null>(null)
  const [error, setError] = useState<string | null>(null)
  const current = declared.version
  const [policy, setPolicy] = useState<Policy>(
    current === 'latest' || current === 'next' ? current : 'pin',
  )
  const [pick, setPick] = useState<string | undefined>(undefined)

  useEffect(() => {
    let cancelled = false
    api
      .versions(declared.name)
      .then((result) => {
        if (cancelled) return
        setVersions(result)
        const newest =
          result.versions.find((v) => v.tags.includes('latest')) ??
          result.versions[0]
        setPick((prev) => prev ?? newest?.version)
      })
      .catch((cause) => !cancelled && setError(errorMessage(cause)))
    return () => {
      cancelled = true
    }
  }, [api, declared.name])

  const selector = policy === 'pin' ? pick : policy
  const newer = versions
    ? versions.versions.findIndex((v) => v.version === current)
    : -1
  const worker = `package://${declared.ref}`
  const unchanged = !selector || selector === current

  return (
    <>
      <SettingsSection
        title="Version"
        description={
          current
            ? `Declared ${current}${newer > 0 ? `; the registry has ${newer} newer release${newer === 1 ? '' : 's'}` : ''}.`
            : 'Not pinned: Compose resolves the newest version on every update.'
        }
      >
        <SegmentedControl
          variant="radio"
          aria-label="Version policy"
          value={policy}
          onChange={setPolicy}
          options={[
            { value: 'pin', label: 'Pin a version', icon: false },
            { value: 'latest', label: 'Follow latest', icon: false },
            { value: 'next', label: 'Follow next', icon: false },
          ]}
        />
        {policy === 'pin' ? (
          error ? (
            <StatusPanel
              variant="alert"
              headline="The registry did not answer"
              detail={error}
            />
          ) : !versions ? (
            <Skeleton className="wk-skeleton-field" />
          ) : (
            <Selector
              aria-label="Version"
              className="wk-version"
              value={pick}
              onChange={setPick}
              searchPlaceholder={`Search ${versions.versions.length} versions`}
              options={versions.versions.map((v) => ({
                value: v.version,
                label: v.version,
                description: [
                  ...v.tags,
                  v.version === current ? 'declared now' : null,
                  v.created_at ? formatRelative(v.created_at) : null,
                ]
                  .filter(Boolean)
                  .join(' · '),
              }))}
            />
          )
        ) : (
          <p className="wk-note">
            Writes <span className="wk-mono">version: {policy}</span>; Compose
            resolves the channel again on every update.
          </p>
        )}
      </SettingsSection>
      {unchanged ? null : (
        <Preview
          before={entryYaml({ name: declared.name, worker, version: current })}
          after={entryYaml({ name: declared.name, worker, version: selector })}
        >
          <StatusPanel
            variant="warn"
            headline="The project may restart"
            detail={`Compose resolves ${declared.name} ${selector} and its dependencies again, and restarts the project when what runs changes.`}
          />
          <div className="wk-buttons">
            <Button
              variant="ghost"
              size="sm"
              onClick={() =>
                setPolicy(
                  current === 'latest' || current === 'next' ? current : 'pin',
                )
              }
            >
              Cancel
            </Button>
            <Button
              variant="primary"
              size="sm"
              disabled={actions.busy}
              onClick={() =>
                void actions.track(
                  `Updating ${declared.name} to ${selector}`,
                  () => api.update([`${declared.name}@${selector}`]),
                )
              }
            >
              Update to {selector}
            </Button>
          </div>
        </Preview>
      )}
    </>
  )
}

function PathSource({ api, actions, declared, onOpenSettings }: Props) {
  const [path, setPath] = useState(declared.ref)
  const [current, setCurrent] = useState<Inspection | null>(null)
  const [allCheckouts, setAllCheckouts] = useState(false)
  const [target, setTarget] = useState<Inspection | null>(null)
  const settled = useDebounce(path.trim(), 300)
  const moved =
    settled !== '' &&
    settled.replace(/\/+$/, '') !== declared.ref.replace(/\/+$/, '')

  useEffect(() => {
    let cancelled = false
    api
      .inspect(declared.ref, declared.run)
      .then((result) => !cancelled && setCurrent(result))
      .catch(() => undefined)
    return () => {
      cancelled = true
    }
  }, [api, declared.ref, declared.run])

  useEffect(() => {
    setTarget(null)
    if (!moved) return
    let cancelled = false
    api
      .inspect(settled, declared.run)
      .then((result) => !cancelled && setTarget(result))
      .catch(() => undefined)
    return () => {
      cancelled = true
    }
  }, [api, settled, moved, declared.run])

  const renamed = moved && basename(settled) !== declared.name
  const ready = moved && !renamed && !!target?.manifest
  const shortTarget = target ? shortPath(target.path) : shortPath(settled)

  return (
    <>
      <SettingsSection
        title="Directory"
        description="The worker's directory on the daemon host."
      >
        <Input
          aria-label="Worker directory"
          className="wk-mono"
          value={path}
          onChange={setPath}
          spellCheck={false}
        />
      </SettingsSection>

      {current?.checkouts.length ? (
        <SettingsSection
          title={`Other checkouts of ${declared.name}`}
          description="From git worktree list in the current checkout."
        >
          <List aria-label={`Checkouts of ${declared.name}`}>
            {(allCheckouts
              ? current.checkouts
              : current.checkouts.slice(0, CHECKOUTS)
            ).map((checkout) => {
              const other = basename(checkout.path) !== declared.name
              const isCurrent = checkout.path === current.path
              return (
                <ListItem
                  key={checkout.path}
                  selected={checkout.path === settled}
                  disabled={other}
                  label={
                    <span className="wk-mono">{shortPath(checkout.path)}</span>
                  }
                  description={
                    other
                      ? `Would be a new container named ${basename(checkout.path)}`
                      : (checkout.branch ?? 'detached')
                  }
                  trailing={isCurrent ? <Badge>current</Badge> : undefined}
                  onClick={() => setPath(checkout.path)}
                />
              )
            })}
          </List>
          {!allCheckouts && current.checkouts.length > CHECKOUTS ? (
            <Button
              variant="ghost"
              size="sm"
              className="wk-suggestion"
              onClick={() => setAllCheckouts(true)}
            >
              Show all {current.checkouts.length}
            </Button>
          ) : null}
        </SettingsSection>
      ) : null}

      {moved ? (
        <div className="wk-checks">
          {renamed ? (
            <StatusPanel
              variant="alert"
              headline={`That would add a container named ${basename(settled)}`}
              detail={`Compose names a local worker after its directory. Pick a directory called ${declared.name}.`}
            />
          ) : !target ? (
            <Skeleton className="wk-skeleton-field" />
          ) : !target.exists ? (
            <StatusPanel
              variant="alert"
              headline="No such directory"
              detail={shortTarget}
            />
          ) : !target.manifest ? (
            <StatusPanel
              variant="alert"
              headline="No worker in this directory"
              detail={`There is no iii.worker.yaml in ${shortTarget}.`}
            />
          ) : (
            <>
              <StatusPanel
                variant="success"
                headline={`iii.worker.yaml found: ${target.manifest.name}${target.manifest.language ? `, ${target.manifest.language}` : ''}`}
                detail="Same container name, so its environment and scripts stay."
              />
              {target.run_found === false ? (
                <StatusPanel
                  variant="warn"
                  headline="The run command is not built here"
                  detail={`${declared.run} does not exist in this checkout, so ${declared.name} will not start until it is.`}
                  action={
                    <Button variant="ghost" size="sm" onClick={onOpenSettings}>
                      Change it
                    </Button>
                  }
                />
              ) : null}
            </>
          )}
        </div>
      ) : null}

      {ready && target ? (
        <Preview
          before={entryYaml({
            name: declared.name,
            worker: `path://${declared.ref}`,
            run: declared.run,
          })}
          after={entryYaml({
            name: declared.name,
            worker: `path://${target.path}`,
            run: declared.run,
          })}
        >
          <p className="wk-note">
            {declared.name} restarts from the new directory.
          </p>
          <div className="wk-buttons">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setPath(declared.ref)}
            >
              Cancel
            </Button>
            <Button
              variant="primary"
              size="sm"
              disabled={actions.busy}
              onClick={() =>
                void actions.track(
                  `Pointing ${declared.name} to ${shortTarget}`,
                  () =>
                    api.edit(declared.name, {
                      worker: `path://${target.path}`,
                    }),
                )
              }
            >
              Point to {shortTarget}
            </Button>
          </div>
        </Preview>
      ) : null}
    </>
  )
}
