import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { useDebounce } from '@iii-dev/console-ui/hooks'
import {
  ChevronDown,
  ChevronRight,
  Folder,
  GitBranch,
  Package,
} from 'lucide-react'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { Badge } from '@/components/shadcn/badge'
import { Button } from '@/components/shadcn/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/shadcn/card'
import { FileDiff } from '@/components/ui/FileDiff'
import { List, ListItem } from '@/components/ui/List'
import { SegmentedControl } from '@/components/ui/ModeToggle'
import { Selector } from '@/components/ui/Selector'
import { SettingsSection } from '@/components/ui/Settings'
import { Skeleton } from '@/components/ui/Skeleton'
import { Notice, SearchBox, TextInput } from './components/kit'
import type {
  Checkout,
  ComposeApi,
  DeclaredContainer,
  Inspection,
  Versions,
} from './compose-api'
import type { Actions } from './index'
import {
  arrangeCheckouts,
  basename,
  branchLabel,
  dirname,
  entryYaml,
  inPlaceVersion,
  matchParts,
  shortPath,
  withAlsoStarts,
} from './model'

type Props = {
  api: ComposeApi
  actions: Actions
  declared: DeclaredContainer
  /** Containers that start after this one. */
  neededBy: readonly string[]
  /** How many containers the project declares. */
  total: number
  onOpenSettings: () => void
}

const FILE = 'worker-compose.yaml'
const KINDS = [
  { value: 'package' as const, label: 'Registry package', icon: <Package /> },
  { value: 'path' as const, label: 'Local path', icon: <Folder /> },
]

export function SourceTab(props: Props) {
  const [kind, setKind] = useState<'package' | 'path'>(
    props.declared.source === 'package' ? 'package' : 'path',
  )
  const source = (
    <SettingsSection title="Source">
      <SegmentedControl
        variant="radio"
        className="wk-seg"
        aria-label="Where the worker comes from"
        value={kind}
        onChange={setKind}
        options={KINDS}
      />
    </SettingsSection>
  )
  if (props.declared.source === 'unknown')
    return (
      <Split
        form={
          <Notice
            variant="info"
            headline="This source is not editable here"
            detail={`${props.declared.name} is declared as ${props.declared.ref}, which is neither a registry package nor a local path. Edit it in the compose file.`}
          />
        }
      />
    )
  if (kind !== props.declared.source)
    return (
      <Split
        form={
          <>
            {source}
            <Notice
              variant="info"
              headline="That replaces the container"
              detail={`A registry package and a local checkout are not versions of each other. To switch, remove ${props.declared.name} and add the other kind from Add worker; its environment and scripts do not carry over.`}
            />
          </>
        }
      />
    )
  return props.declared.source === 'package' ? (
    <PackageSource {...props} source={source} />
  ) : (
    <PathSource {...props} source={source} />
  )
}

/** The form on the left, what the change does and its preview on the right. */
function Split({ form, aside }: { form: ReactNode; aside?: ReactNode }) {
  return (
    <div className="wk-settings">
      <div className="wk-form">{form}</div>
      {aside ? <div className="wk-aside">{aside}</div> : null}
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
    <Card size="sm" className="wk-card wk-preview">
      <CardHeader>
        <CardTitle className="wk-card-title">Change preview</CardTitle>
        <CardDescription className="wk-card-note wk-machine">
          {FILE}
        </CardDescription>
      </CardHeader>
      <CardContent className="wk-preview-body flex flex-col gap-3">
        <div className="wk-diff">
          <FileDiff
            oldFile={{ name: FILE, contents: before }}
            newFile={{ name: FILE, contents: after }}
            disableFileHeader
            expandUnchanged
          />
        </div>
        {children}
      </CardContent>
    </Card>
  )
}

type Policy = 'pin' | 'latest' | 'next'

function PackageSource({
  api,
  actions,
  declared,
  total,
  source,
}: Props & { source: ReactNode }) {
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
        // Start on what is declared: a change is always something the operator picked.
        setPick((prev) => prev ?? declared.version ?? undefined)
      })
      .catch((cause) => !cancelled && setError(errorMessage(cause)))
    return () => {
      cancelled = true
    }
  }, [api, declared.name, declared.version])

  const selector = policy === 'pin' ? pick : policy

  const inPlace = inPlaceVersion(declared)
  const update = async () => {
    if (!selector) return
    const ok = await actions.confirm({
      title: `Update ${declared.name} to ${selector}?`,
      description: withAlsoStarts(
        inPlace
          ? `Compose installs it and restarts only ${declared.name}; workers that depend on it see it reconnect.`
          : `${declared.name} is not named after its package, so Compose updates it through the whole project and restarts every container.`,
        actions.idle,
        [declared.name],
      ),
      confirmLabel: inPlace
        ? `Update ${declared.name}`
        : 'Update and restart all',
    })
    if (ok)
      await actions.track(
        `Updating ${declared.name} to ${selector}`,
        (operationId) =>
          inPlace
            ? api.setVersions(
                [{ ref: declared.ref, version: selector }],
                operationId,
              )
            : api.update([`${declared.name}@${selector}`], operationId),
      )
  }
  const newer = versions
    ? versions.versions.findIndex((v) => v.version === current)
    : -1
  const worker = `package://${declared.ref}`
  const unchanged = !selector || selector === current

  return (
    <Split
      form={
        <>
          {source}
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
              className="wk-seg"
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
                <Notice
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
                  contentClassName="wk-popover max-h-[min(22rem,var(--radix-popover-content-available-height))]"
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
                Writes <span className="wk-mono">version: {policy}</span>
                {'; '}Compose resolves the channel again on every update.
              </p>
            )}
          </SettingsSection>
        </>
      }
      aside={
        unchanged ? null : (
          <Preview
            before={entryYaml({
              name: declared.name,
              worker,
              version: current,
            })}
            after={entryYaml({
              name: declared.name,
              worker,
              version: selector,
            })}
          >
            <Notice
              variant="warn"
              headline={
                inPlace
                  ? `Only ${declared.name} restarts`
                  : 'The whole project restarts'
              }
              detail={
                inPlace
                  ? `Compose installs ${declared.name} ${selector} and restarts that one container; the other ${total - 1} keep running.`
                  : `${declared.name} is not named after its package, so Compose updates it through the whole project and restarts all ${total} containers.`
              }
            />
            <div className="wk-buttons">
              <Button
                variant="ghost"
                className="wk-press font-inter"
                onClick={() => {
                  setPolicy(
                    current === 'latest' || current === 'next'
                      ? current
                      : 'pin',
                  )
                  setPick(current ?? undefined)
                }}
              >
                Cancel
              </Button>
              <Button
                variant="default"
                className="wk-press font-inter"
                disabled={actions.busy}
                onClick={() => void update()}
              >
                Update to {selector}
              </Button>
            </div>
          </Preview>
        )
      }
    />
  )
}

function PathSource({
  api,
  actions,
  declared,
  neededBy,
  onOpenSettings,
  source,
}: Props & { source: ReactNode }) {
  const [path, setPath] = useState(declared.ref)
  const [current, setCurrent] = useState<Inspection | null>(null)
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

  const pointTo = async (path: string) => {
    const ok = await actions.confirm({
      title: `Point ${declared.name} to ${shortPath(path)}?`,
      description: withAlsoStarts(
        `${declared.name} restarts from that directory. If it does not build or start there, it stays down until you point it back.`,
        actions.idle,
        [declared.name],
      ),
      confirmLabel: 'Point and restart',
    })
    if (ok)
      await actions.track(
        `Pointing ${declared.name} to ${shortPath(path)}`,
        (operationId) =>
          api.edit(declared.name, { worker: `path://${path}` }, operationId),
      )
  }

  const renamed = moved && basename(settled) !== declared.name
  const ready = moved && !renamed && !!target?.manifest
  const shortTarget = target ? shortPath(target.path) : shortPath(settled)

  return (
    <Split
      form={
        <>
          {source}
          <SettingsSection
            title="Directory"
            description="The worker's directory on the daemon host."
          >
            <TextInput
              aria-label="Worker directory"
              className="wk-machine"
              value={path}
              onChange={setPath}
              spellCheck={false}
            />
          </SettingsSection>

          {current?.checkouts.length ? (
            <CheckoutPicker
              name={declared.name}
              checkouts={current.checkouts}
              current={current.path}
              picked={settled}
              onPick={setPath}
            />
          ) : null}
        </>
      }
      aside={
        <>
          {moved ? (
            <div className="wk-checks">
              {renamed ? (
                <Notice
                  variant="alert"
                  headline={`That would add a container named ${basename(settled)}`}
                  detail={`Compose names a local worker after its directory. Pick a directory called ${declared.name}.`}
                />
              ) : !target ? (
                <Skeleton className="wk-skeleton-field" />
              ) : !target.exists ? (
                <Notice
                  variant="alert"
                  headline="No such directory"
                  detail={shortTarget}
                />
              ) : !target.manifest ? (
                <Notice
                  variant="alert"
                  headline="No worker in this directory"
                  detail={`There is no iii.worker.yaml in ${shortTarget}.`}
                />
              ) : (
                <>
                  <Notice
                    variant="success"
                    headline={`iii.worker.yaml found: ${target.manifest.name}${target.manifest.language ? `, ${target.manifest.language}` : ''}`}
                    detail="Same container name, so its environment and scripts stay."
                  />
                  {target.run_found === false ? (
                    <Notice
                      variant="warn"
                      headline="The run command is not built here"
                      detail={`${declared.run} does not exist in this checkout, so ${declared.name} will not start until it is.`}
                      action={
                        <Button
                          variant="outline"
                          className="wk-press font-inter"
                          onClick={onOpenSettings}
                        >
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
                {declared.name} restarts from the new directory.{' '}
                {neededBy.length
                  ? `${neededBy.join(', ')} restart${neededBy.length === 1 ? 's' : ''} after it.`
                  : 'Nothing else depends on it.'}
              </p>
              <div className="wk-buttons">
                <Button
                  variant="ghost"
                  className="wk-press font-inter"
                  onClick={() => setPath(declared.ref)}
                >
                  Cancel
                </Button>
                <Button
                  variant="default"
                  className="wk-press font-inter"
                  disabled={actions.busy}
                  onClick={() => void pointTo(target.path)}
                >
                  Apply
                </Button>
              </div>
            </Preview>
          ) : null}
        </>
      }
    />
  )
}

/**
 * Every git checkout of the worker, searchable by branch or folder. The ones
 * whose folder carries the container's name can take it; the rest would add
 * a new container, so they wait, dimmed, behind a toggle.
 */
function CheckoutPicker({
  name,
  checkouts,
  current,
  picked,
  onPick,
}: {
  name: string
  checkouts: readonly Checkout[]
  current: string
  picked: string
  onPick: (path: string) => void
}) {
  const [query, setQuery] = useState('')
  const [showOther, setShowOther] = useState(false)
  const list = useRef<HTMLDivElement>(null)
  const { usable, other } = arrangeCheckouts(checkouts, name, current, query)
  const filtering = query.trim() !== ''
  const otherOpen = showOther || (filtering && other.length > 0)

  const row = (checkout: Checkout, canPoint: boolean) => (
    <ListItem
      key={checkout.path}
      selected={canPoint && checkout.path === picked}
      disabled={!canPoint}
      leading={<GitBranch />}
      label={
        <Highlight
          className="wk-mono"
          text={branchLabel(checkout)}
          query={query}
        />
      }
      description={
        <Highlight
          className="wk-mono"
          // The folder name is the container's for every usable row: show where it is.
          text={shortPath(canPoint ? dirname(checkout.path) : checkout.path)}
          query={query}
        />
      }
      trailing={
        <span className="wk-checkout-meta">
          {checkout.dirty ? (
            <span
              className="wk-dirty"
              role="img"
              aria-label="Uncommitted changes"
              title="Uncommitted changes"
            />
          ) : null}
          {checkout.path === current ? (
            <Badge variant="secondary" className="wk-badge font-inter">
              current
            </Badge>
          ) : null}
          {checkout.committed_at ? (
            <span className="wk-mono wk-faint" title="Last commit">
              {formatRelative(checkout.committed_at * 1000)}
            </span>
          ) : null}
        </span>
      }
      onClick={canPoint ? () => onPick(checkout.path) : undefined}
    />
  )

  return (
    <SettingsSection
      title={`Checkouts of ${name}`}
      description="From git worktree list: the one it runs from first, then the newest commit."
    >
      <div className="wk-picker">
        <SearchBox
          aria-label="Find a checkout by branch or folder"
          placeholder={`Find by branch or folder · ${checkouts.length} checkouts`}
          value={query}
          onChange={setQuery}
          onKeyDown={(event) => {
            if (event.key !== 'ArrowDown') return
            event.preventDefault()
            list.current
              ?.querySelector<HTMLElement>('[data-list-item]:not([disabled])')
              ?.focus()
          }}
        />
        <div ref={list} className="wk-picker-list">
          {usable.length ? (
            <List aria-label={`Checkouts of ${name}`}>
              {usable.map((checkout) => row(checkout, true))}
            </List>
          ) : (
            <p className="wk-note wk-picker-note">
              No checkout with a <span className="wk-mono">{name}</span> folder
              matches “{query.trim()}”.
            </p>
          )}
          {other.length ? (
            <>
              <Button
                variant="ghost"
                className="wk-picker-group wk-press font-inter"
                aria-expanded={otherOpen}
                onClick={() => setShowOther((open) => !open)}
              >
                {otherOpen ? <ChevronDown /> : <ChevronRight />}
                {filtering
                  ? `${other.length} more match${other.length === 1 ? '' : 'es'}`
                  : `${other.length} more`}{' '}
                in folders with another name
              </Button>
              {otherOpen ? (
                <>
                  <p className="wk-note wk-picker-note">
                    Their folder has another name, so pointing here would add a
                    new container named after it instead of moving {name}.
                  </p>
                  <List aria-label={`Other checkouts of ${name}`}>
                    {other.map((checkout) => row(checkout, false))}
                  </List>
                </>
              ) : null}
            </>
          ) : null}
        </div>
      </div>
    </SettingsSection>
  )
}

function Highlight({
  text,
  query,
  className,
}: {
  text: string
  query: string
  className?: string
}) {
  const [before, hit, after] = matchParts(text, query)
  return (
    <span className={className}>
      {before}
      {hit ? <mark className="wk-mark">{hit}</mark> : null}
      {after}
    </span>
  )
}
