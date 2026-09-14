import {
  Button,
  Chip,
  EmptyState,
  type Host,
  IconButton,
  Input,
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
  SegmentedControl,
  Select,
  Skeleton,
  StatusPanel,
  Tabs,
  TabsList,
  TabsTrigger,
  uiClasses,
  useTheme,
} from '@iii-dev/console-ui'
import { type CSSProperties, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Inspector, InspectorReset, PatchView, SideBySide, StoryFrame } from './preview'
import {
  AlertIcon,
  api,
  type BuildEvent,
  type Change,
  type ChangeKind,
  ChevronRightIcon,
  type CompareOutput,
  ComponentIcon,
  type ComponentSummary,
  changeTone,
  errorMessage,
  FileIcon,
  FilterIcon,
  FolderIcon,
  type FsNode,
  type FsTree,
  type GetOutput,
  HammerIcon,
  isBuildingError,
  lineLabel,
  RefreshIcon,
  type Selection,
  StateIcon,
  StoriesIcon,
  shortSha,
  subscribe,
  useContainerTier,
  usePaneState,
  type Workspace,
} from './shared'

type Mode = 'explorer' | 'compare'
type Tab = 'preview' | 'changes' | 'props'

/** Below this the sidebar and the main column stack; from `WIDE` up the
    props inspector earns its own column instead of a tab. */
const NARROW_BELOW = 760
const WIDE_FROM = 1080

const isChanged = (kind: ChangeKind | undefined) => kind !== undefined && kind !== 'unchanged'

/** Drop everything without a change below it. */
function impacted(node: FsNode): FsNode | null {
  if (node.kind === 'file') {
    const components = (node.components ?? []).filter((c) => isChanged(c.change?.kind))
    return components.length ? { ...node, components } : null
  }
  const children = (node.children ?? []).map(impacted).filter((child): child is FsNode => child !== null)
  return children.length ? { ...node, children } : null
}

/** An inline line selector; a custom ref opens a text field on its own row. */
function LinePicker({
  value,
  onChange,
  workspace,
  label,
  allowPrev,
}: {
  value: string
  onChange: (next: string) => void
  workspace: Workspace | null
  label: string
  allowPrev: boolean
}) {
  const known = useMemo(() => {
    const options = new Map<string, string>()
    options.set('worktree', 'working tree')
    if (allowPrev) options.set('prev', 'previous build')
    // HEAD is labelled with the commit it points at now; built ref lines
    // are keyed by their sha, because several may carry the same ref name
    // from the commits it pointed at before.
    options.set('HEAD', workspace?.head ? `HEAD · ${shortSha(workspace.head)}` : 'HEAD')
    if (workspace?.branch) options.set(`origin/${workspace.branch}`, `origin/${workspace.branch}`)
    for (const line of workspace?.lines ?? []) {
      if (line.kind !== 'ref' || !line.sha || line.sha === workspace?.head) continue
      options.set(line.sha, `${line.label} · ${shortSha(line.sha)}`)
    }
    return options
  }, [workspace, allowPrev])
  const custom = !known.has(value)
  return (
    <>
      <Select
        appearance="inline"
        aria-label={label}
        onChange={(next) => onChange(next === '__custom' ? '' : next)}
        options={[...[...known].map(([v, l]) => ({ value: v, label: l })), { value: '__custom', label: 'Other ref…' }]}
        showChevron
        value={custom ? '__custom' : value}
      />
      {custom ? (
        <Input
          aria-label={`${label} ref`}
          className="stories-sidebar__ref"
          onChange={onChange}
          placeholder="branch, tag or sha"
          value={value}
        />
      ) : null}
    </>
  )
}

export function StoriesPage({ host, onRequestClose, paneId, tabId, panelSide }: PageRenderProps & { host: Host }) {
  const paneKey = paneId || tabId || 'default'
  const theme = useTheme()
  const [rootRef, tier] = useContainerTier(NARROW_BELOW, WIDE_FROM)
  const narrow = tier === 'narrow'
  const wide = tier === null || tier === 'wide'
  const [workspaces, setWorkspaces] = useState<Workspace[] | null>(null)
  const [workspaceName, setWorkspaceName] = usePaneState<string>(`stories:workspace:${paneKey}`, '')
  const [mode, setMode] = usePaneState<Mode>(`stories:mode:${paneKey}`, 'explorer')
  const [base, setBase] = usePaneState<string>(`stories:base:${paneKey}`, 'HEAD')
  const [impactedOnly, setImpactedOnly] = usePaneState<boolean>(`stories:impacted:${paneKey}`, false)
  const [compareA, setCompareA] = usePaneState<string>(`stories:compare-a:${paneKey}`, 'HEAD')
  const [compareB, setCompareB] = usePaneState<string>(`stories:compare-b:${paneKey}`, 'worktree')
  const [selection, setSelection] = usePaneState<Selection | null>(`stories:selection:${paneKey}`, null)
  const [tab, setTab] = useState<Tab>('preview')
  const [tree, setTree] = useState<FsTree | null>(null)
  const [treeError, setTreeError] = useState<string | null>(null)
  const [treeLoading, setTreeLoading] = useState(false)
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set())
  const [compare, setCompare] = useState<CompareOutput | null>(null)
  const [compareError, setCompareError] = useState<string | null>(null)
  const [compareLoading, setCompareLoading] = useState(false)
  const [detail, setDetail] = useState<GetOutput | null>(null)
  const [detailA, setDetailA] = useState<GetOutput | null>(null)
  const [detailError, setDetailError] = useState<string | null>(null)
  const [overrides, setOverrides] = useState<Record<string, unknown>>({})
  const [building, setBuilding] = useState<BuildEvent | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [diffPath, setDiffPath] = useState<string | null>(null)
  const [diff, setDiff] = useState<{ path: string; text: string } | null>(null)
  const [diffLoading, setDiffLoading] = useState(false)

  const workspace = workspaces?.find((w) => w.name === workspaceName) ?? workspaces?.[0] ?? null
  const ws = workspace?.name
  const lineB = mode === 'compare' ? compareB : 'worktree'
  const lineA = mode === 'compare' ? compareA : base

  // The props tab only exists while the inspector has no column of its own.
  useEffect(() => {
    if (wide && tab === 'props') setTab('preview')
  }, [wide, tab])

  const loadWorkspaces = useCallback(async () => {
    try {
      const result = await api.workspaces(host.iii)
      // Same payload, same state: a fresh array per fetch would re-render
      // the whole page for nothing.
      setWorkspaces((current) => (JSON.stringify(current) === JSON.stringify(result.workspaces) ? current : result.workspaces))
    } catch (error) {
      setTreeError(errorMessage(error))
    }
  }, [host])

  useEffect(() => {
    if (workspaces && !workspaces.some((w) => w.name === workspaceName) && workspaces[0]) {
      setWorkspaceName(workspaces[0].name)
    }
  }, [workspaces, workspaceName, setWorkspaceName])

  const loadTree = useCallback(async () => {
    if (!ws) return
    setTreeLoading(true)
    setTreeError(null)
    try {
      setTree(await api.tree(host.iii, { workspace: ws, base }))
    } catch (error) {
      setTreeError(errorMessage(error))
    } finally {
      setTreeLoading(false)
    }
  }, [host, ws, base])

  const loadCompare = useCallback(async () => {
    if (!ws) return
    setCompareLoading(true)
    setCompareError(null)
    try {
      setCompare(await api.compare(host.iii, { workspace: ws, a: compareA || 'HEAD', b: compareB || 'worktree' }))
    } catch (error) {
      setCompareError(errorMessage(error))
    } finally {
      setCompareLoading(false)
    }
  }, [host, ws, compareA, compareB])

  useEffect(() => {
    void loadWorkspaces()
  }, [loadWorkspaces])

  useEffect(() => {
    if (mode === 'explorer') void loadTree()
  }, [mode, loadTree])

  useEffect(() => {
    if (mode === 'compare') void loadCompare()
  }, [mode, loadCompare])

  // One subscription per pane: the handlers read the latest loaders through
  // a ref so a base or mode change never re-registers the triggers.
  const latest = useRef({ ws, mode, loadTree, loadCompare, loadWorkspaces })
  latest.current = { ws, mode, loadTree, loadCompare, loadWorkspaces }
  useEffect(() => {
    return subscribe(
      host.iii,
      () => {
        const { mode, loadTree, loadCompare, loadWorkspaces } = latest.current
        void loadWorkspaces()
        if (mode === 'explorer') void loadTree()
        else void loadCompare()
      },
      (event) => {
        const { ws, mode, loadTree, loadCompare, loadWorkspaces } = latest.current
        if (ws && event.workspace !== ws) return
        setBuilding(event.status === 'done' ? null : event)
        if (event.status === 'done') {
          void loadWorkspaces()
          if (mode === 'explorer') void loadTree()
          else void loadCompare()
        }
      },
    )
  }, [host])

  useEffect(() => {
    setOverrides({})
    setDiffPath(null)
    setDiff(null)
    setDetail(null)
    setDetailA(null)
    setDetailError(null)
    if (!ws || !selection) return
    let cancelled = false
    const payload = { workspace: ws, id: selection.id, project: selection.project }
    void api
      .get(host.iii, { ...payload, line: lineB, base: lineA })
      .then((result) => {
        if (!cancelled) setDetail(result)
      })
      .catch((error) => {
        if (!cancelled) setDetailError(errorMessage(error))
      })
    if (mode === 'compare') {
      void api
        .get(host.iii, { ...payload, line: lineA })
        .then((result) => {
          if (!cancelled) setDetailA(result)
        })
        .catch(() => {
          if (!cancelled) setDetailA(null)
        })
    }
    return () => {
      cancelled = true
    }
  }, [host, ws, selection?.id, selection?.project, lineA, lineB, mode])

  useEffect(() => {
    if (!ws || !diffPath) return
    let cancelled = false
    setDiffLoading(true)
    void api
      .diffFile(host.iii, { workspace: ws, a: lineA, b: lineB, path: diffPath })
      .then((result) => {
        if (!cancelled) setDiff({ path: diffPath, text: result.identical ? '' : result.diff })
      })
      .catch((error) => {
        if (!cancelled) setDiff({ path: diffPath, text: errorMessage(error) })
      })
      .finally(() => {
        if (!cancelled) setDiffLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [host, ws, diffPath, lineA, lineB])

  const rebuild = (line: string) => {
    if (!ws) return
    setActionError(null)
    void api.build(host.iii, { workspace: ws, line, wait: false }).catch((error) => setActionError(errorMessage(error)))
  }

  const select = (component: ComponentSummary, state?: string) =>
    setSelection({ project: component.project, id: component.id, state: state ?? component.states[0]?.id ?? null })

  const toggle = (path: string) =>
    setCollapsed((current) => {
      const next = new Set(current)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      return next
    })

  const globals = useMemo(() => ({ theme }), [theme])

  const renderComponent = (component: ComponentSummary, depth: number) => {
    const key = `${component.project}:${component.id}`
    const open = !collapsed.has(key)
    const active = selection?.project === component.project && selection?.id === component.id
    return (
      <div key={key}>
        {/* biome-ignore lint/a11y/useSemanticElements: the row hosts a nested caret button. */}
        <div
          aria-current={active && !selection?.state ? 'true' : undefined}
          className={uiClasses.treeItem}
          onClick={() => select(component)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault()
              select(component)
            }
          }}
          role="button"
          style={{ '--iii-ui-tree-depth': depth } as CSSProperties}
          tabIndex={0}
        >
          <span className={uiClasses.treeItemIcon} data-color={component.error ? 'rose' : 'purple'}>
            <ComponentIcon />
          </span>
          <span className={uiClasses.treeItemLabel}>{component.title.split('/').pop()}</span>
          {component.states.length > 0 ? (
            <button
              aria-expanded={open}
              aria-label={open ? 'collapse states' : 'expand states'}
              className={uiClasses.treeItemCaret}
              onClick={(event) => {
                event.stopPropagation()
                toggle(key)
              }}
              type="button"
            >
              <ChevronRightIcon />
            </button>
          ) : null}
          <span className={uiClasses.treeItemTrailing}>
            {isChanged(component.change?.kind) ? (
              <Chip tone={changeTone(component.change?.kind)}>{component.change?.kind}</Chip>
            ) : null}
          </span>
        </div>
        {open
          ? component.states.map((state) => {
              const selected = active && selection?.state === state.id
              return (
                <button
                  aria-current={selected ? 'true' : undefined}
                  className={uiClasses.treeItem}
                  key={state.id}
                  onClick={() => select(component, state.id)}
                  style={{ '--iii-ui-tree-depth': depth + 1 } as CSSProperties}
                  type="button"
                >
                  <span className={uiClasses.treeItemIcon} data-color="teal">
                    <StateIcon />
                  </span>
                  <span className={uiClasses.treeItemLabel}>{state.name}</span>
                </button>
              )
            })
          : null}
      </div>
    )
  }

  const renderNode = (node: FsNode, depth: number): React.ReactNode => {
    const open = !collapsed.has(node.path)
    const isFile = node.kind === 'file'
    return (
      <div key={node.path}>
        <button
          aria-expanded={open}
          className={uiClasses.treeItem}
          onClick={() => toggle(node.path)}
          style={{ '--iii-ui-tree-depth': depth } as CSSProperties}
          type="button"
        >
          <span className={uiClasses.treeItemIcon} data-color={isFile ? 'blue' : 'amber'}>
            {isFile ? <FileIcon /> : <FolderIcon />}
          </span>
          <span className={uiClasses.treeItemLabel}>{node.name}</span>
          <span aria-hidden className={uiClasses.treeItemCaret} data-open={open ? 'true' : undefined}>
            <ChevronRightIcon />
          </span>
          <span className={uiClasses.treeItemTrailing}>
            {node.git ? <span className={uiClasses.treeItemMeta}>{node.git}</span> : null}
            {!isFile && isChanged(node.change) ? <Chip tone={changeTone(node.change)}>{node.change}</Chip> : null}
          </span>
        </button>
        {open
          ? isFile
            ? (node.components ?? []).map((component) => renderComponent(component, depth + 1))
            : (node.children ?? []).map((child) => renderNode(child, depth + 1))
          : null}
      </div>
    )
  }

  const visibleRoot = useMemo(() => {
    if (!tree) return null
    return impactedOnly ? impacted(tree.root) : tree.root
  }, [tree, impactedOnly])

  /** Only the kinds that occurred; four zero-chips said nothing. */
  const summaryRow = (summary: { direct: number; indirect: number; new: number; removed: number } | undefined, note?: string) => {
    if (!summary && !note) return null
    const kinds = summary ? (['direct', 'indirect', 'new', 'removed'] as const).filter((kind) => summary[kind] > 0) : []
    return (
      <div className="stories-summary">
        {kinds.map((kind) => (
          <Chip key={kind} tone={changeTone(kind)}>
            {summary?.[kind]} {kind}
          </Chip>
        ))}
        {summary && kinds.length === 0 ? <span className="stories-summary__note">no changes</span> : null}
        {note ? <span className="stories-summary__note">{note}</span> : null}
      </div>
    )
  }

  const buildingPanel = (message: string) => (
    <div className="stories-pad">
      <StatusPanel detail={message} headline="Building" variant="info" />
      <div className="stories-actions">
        <Button onClick={() => (mode === 'explorer' ? void loadTree() : void loadCompare())} size="sm" variant="pill">
          Retry
        </Button>
        <Button
          onClick={() => {
            rebuild(lineA)
            rebuild(lineB)
          }}
          size="sm"
          variant="pill"
        >
          Build
        </Button>
      </div>
    </div>
  )

  const sidebar = (
    <PageSidebar
      className="stories-sidebar"
      collapsible
      defaultWidth={280}
      header={
        <div className="stories-sidebar__head">
          <SegmentedControl
            aria-label="Mode"
            onChange={setMode}
            options={[
              { value: 'explorer', label: 'Explorer', icon: false },
              { value: 'compare', label: 'Compare', icon: false },
            ]}
            value={mode}
            variant="radio"
          />
          {mode === 'explorer' ? (
            <IconButton
              aria-pressed={impactedOnly}
              className="stories-filter"
              label={impactedOnly ? 'Show every component' : 'Impacted only'}
              onClick={() => setImpactedOnly(!impactedOnly)}
              tooltipSide="bottom"
              variant="ghost"
            >
              <FilterIcon />
            </IconButton>
          ) : null}
        </div>
      }
      label="Stories"
      maxWidth={480}
      minWidth={220}
      narrow={narrow}
      resizable
      side={panelSide}
      storageKey="stories:sidebar"
    >
      <div className="stories-sidebar__bar">
        {workspaces && workspaces.length > 1 ? (
          <Select
            appearance="inline"
            aria-label="Workspace"
            onChange={setWorkspaceName}
            options={workspaces.map((w) => ({ value: w.name, label: w.name }))}
            showChevron
            value={workspace?.name}
          />
        ) : null}
        {mode === 'explorer' ? (
          <>
            <span className="stories-sidebar__bar-label">working tree vs</span>
            <LinePicker allowPrev label="Base line" onChange={setBase} value={base} workspace={workspace} />
          </>
        ) : (
          <>
            <LinePicker allowPrev label="Line A" onChange={setCompareA} value={compareA} workspace={workspace} />
            <span className="stories-sidebar__bar-label">→</span>
            <LinePicker allowPrev label="Line B" onChange={setCompareB} value={compareB} workspace={workspace} />
          </>
        )}
      </div>
      {mode === 'explorer'
        ? summaryRow(tree?.summary, tree?.base_pending ? 'base line building…' : undefined)
        : summaryRow(compare?.summary)}
      <div className="stories-sidebar__body">
        {mode === 'explorer' ? (
          <>
            {treeError ? (
              isBuildingError(treeError) ? (
                buildingPanel(treeError)
              ) : (
                <div className="stories-pad">
                  <StatusPanel
                    detail={treeError}
                    headline="Stories could not be read"
                    icon={<AlertIcon />}
                    variant="alert"
                  />
                  <Button onClick={() => void loadTree()} size="sm" variant="pill">
                    Retry
                  </Button>
                </div>
              )
            ) : null}
            {treeLoading && !tree ? (
              <div className="stories-pad">
                <Skeleton className="stories-skeleton" />
                <Skeleton className="stories-skeleton" />
                <Skeleton className="stories-skeleton" />
              </div>
            ) : null}
            {visibleRoot ? (
              <div className={uiClasses.tree} data-narrow={narrow ? '' : undefined} role="tree">
                {(visibleRoot.children ?? []).map((child) => renderNode(child, 0))}
              </div>
            ) : tree && impactedOnly ? (
              <div className="stories-pad">
                <StatusPanel
                  headline="Nothing impacted"
                  detail="No component differs from the base line."
                  variant="success"
                />
              </div>
            ) : null}
          </>
        ) : (
          <>
            {compareError ? (
              isBuildingError(compareError) ? (
                buildingPanel(compareError)
              ) : (
                <div className="stories-pad">
                  <StatusPanel
                    detail={compareError}
                    headline="Lines could not be compared"
                    icon={<AlertIcon />}
                    variant="alert"
                  />
                  <Button onClick={() => void loadCompare()} size="sm" variant="pill">
                    Retry
                  </Button>
                </div>
              )
            ) : null}
            {compareLoading && !compare ? (
              <div className="stories-pad">
                <Skeleton className="stories-skeleton" />
                <Skeleton className="stories-skeleton" />
              </div>
            ) : null}
            {compare && compare.changes.length === 0 ? (
              <div className="stories-pad">
                <StatusPanel
                  headline="No differences"
                  detail="Every component renders from the same inputs."
                  variant="success"
                />
              </div>
            ) : null}
            {compare && compare.changes.length > 0 ? (
              <List className="stories-compare-list">
                {[...new Set(compare.changes.map((c) => c.project))].map((project) => (
                  <ListGroup key={project}>
                    <ListGroupLabel>{project}</ListGroupLabel>
                    {compare.changes
                      .filter((c) => c.project === project)
                      .map((change: Change) => (
                        <ListItem
                          description={<span className="stories-mono">{change.file}</span>}
                          key={`${change.project}:${change.id}`}
                          label={change.title}
                          onClick={() =>
                            setSelection({
                              project: change.project,
                              id: change.id,
                              state: change.states[0]?.id ?? null,
                            })
                          }
                          selected={selection?.project === change.project && selection?.id === change.id}
                          trailing={<Chip tone={changeTone(change.kind)}>{change.kind}</Chip>}
                        />
                      ))}
                  </ListGroup>
                ))}
              </List>
            ) : null}
          </>
        )}
      </div>
    </PageSidebar>
  )

  const component = detail?.component ?? null
  const state = component?.states.find((s) => s.id === selection?.state) ?? component?.states[0] ?? null
  const change = detail?.change
  const frameFor = (get: GetOutput | null) =>
    get?.preview_url && state
      ? {
          previewUrl: get.preview_url,
          story: state.id,
          args: overrides,
          globals,
          title: `${get.component.title} · ${state.name}`,
        }
      : null

  const main = (
    <PageMain className="stories-main">
      {building ? (
        <div className="stories-banner">
          <StatusPanel
            detail={building.message ?? `${building.line} · ${building.status}`}
            headline={building.status === 'failed' ? 'Build failed' : 'Building'}
            variant={building.status === 'failed' ? 'alert' : 'info'}
          />
        </div>
      ) : null}
      {actionError ? (
        <div className="stories-banner">
          <StatusPanel detail={actionError} headline="That action failed" icon={<AlertIcon />} variant="alert" />
        </div>
      ) : null}
      {!selection ? (
        <div className="stories-empty">
          <EmptyState
            description={
              mode === 'explorer'
                ? 'Pick a component or a state in the tree to preview it here.'
                : 'Pick a changed component to see both lines side by side.'
            }
            icon={StoriesIcon}
            title="No component selected"
          />
        </div>
      ) : detailError ? (
        <div className="stories-pad">
          <StatusPanel
            detail={detailError}
            headline="The component could not be read"
            icon={<AlertIcon />}
            variant="alert"
          />
        </div>
      ) : !component || !state ? (
        <div className="stories-pad">
          <Skeleton className="stories-skeleton stories-skeleton--title" />
          <Skeleton className="stories-skeleton stories-skeleton--frame" />
        </div>
      ) : (
        <>
          <header className="stories-masthead">
            {narrow ? (
              <Button className="stories-masthead__back" onClick={() => setSelection(null)} size="sm" variant="ghost">
                Back
              </Button>
            ) : null}
            <div className="stories-masthead__identity">
              <h2 className="stories-masthead__title">{component.title}</h2>
              <div className="stories-masthead__meta">
                <span>{component.project}</span>
                <span className="stories-mono">{component.path}</span>
                <span className="stories-mono" title={component.version}>
                  {shortSha(component.version)}
                </span>
                {isChanged(change?.kind) ? <Chip tone={changeTone(change?.kind)}>{change?.kind}</Chip> : null}
              </div>
            </div>
            <Select
              aria-label="State"
              className="stories-masthead__state"
              onChange={(next) => setSelection({ ...selection, state: next })}
              options={component.states.map((s) => ({ value: s.id, label: s.name }))}
              value={state.id}
            />
          </header>
          {component.error ? (
            <div className="stories-banner">
              <StatusPanel detail={component.error} headline="This story file did not build cleanly" variant="warn" />
            </div>
          ) : null}
          <Tabs className="stories-tabs" onValueChange={(next) => setTab(next as Tab)} value={tab}>
            <TabsList variant="line">
              <TabsTrigger icon={false} value="preview">
                {mode === 'compare' ? 'Side by side' : 'Preview'}
              </TabsTrigger>
              <TabsTrigger icon={false} value="changes">
                Changes{change?.files.length ? ` (${change.files.length})` : ''}
              </TabsTrigger>
              {wide ? null : (
                <TabsTrigger icon={false} value="props">
                  Props{Object.keys(overrides).length ? ` (${Object.keys(overrides).length})` : ''}
                </TabsTrigger>
              )}
            </TabsList>
          </Tabs>
          <div className="stories-work">
            {tab === 'preview' ? (
              mode === 'compare' ? (
                <SideBySide
                  a={frameFor(detailA)}
                  aLabel={detailA ? lineLabel(detailA.line) : lineA}
                  b={frameFor(detail)}
                  bLabel={detail ? lineLabel(detail.line) : lineB}
                  narrow={narrow}
                />
              ) : frameFor(detail) ? (
                <StoryFrame {...(frameFor(detail) as NonNullable<ReturnType<typeof frameFor>>)} />
              ) : (
                <div className="stories-pad">
                  <StatusPanel headline="No preview" detail="This story file produced no document." variant="warn" />
                </div>
              )
            ) : null}
            {tab === 'changes' ? (
              <div className="stories-changes">
                {!change || change.files.length === 0 ? (
                  <div className="stories-pad">
                    <StatusPanel
                      headline={change ? 'No input changed' : 'No base line'}
                      detail={
                        change
                          ? 'Every file this component depends on is identical in both lines.'
                          : 'Pick a base line to see which inputs changed.'
                      }
                      variant="info"
                    />
                  </div>
                ) : (
                  <List className="stories-files">
                    {change.files.map((file) => (
                      <ListItem
                        description={`${file.status} · ${file.hop === 0 ? 'story file' : `${file.hop} hop${file.hop === 1 ? '' : 's'} away`}`}
                        key={file.path}
                        label={<span className="stories-mono">{file.path}</span>}
                        onClick={() => setDiffPath(file.path)}
                        selected={diffPath === file.path}
                      />
                    ))}
                  </List>
                )}
                {diffPath ? (
                  diffLoading ? (
                    <div className="stories-pad">
                      <Skeleton className="stories-skeleton stories-skeleton--frame" />
                    </div>
                  ) : diff?.text ? (
                    <PatchView patch={diff.text} />
                  ) : diff ? (
                    <div className="stories-pad">
                      <StatusPanel headline="Identical" detail={diff.path} variant="success" />
                    </div>
                  ) : null
                ) : null}
              </div>
            ) : null}
            {tab === 'props' ? (
              <div className="stories-inspector-inline">
                <div className="stories-inspector__head">
                  <span>Props</span>
                  <InspectorReset onChange={setOverrides} overrides={overrides} />
                </div>
                <Inspector controls={state.controls} onChange={setOverrides} overrides={overrides} />
              </div>
            ) : null}
          </div>
        </>
      )}
    </PageMain>
  )

  // The inspector hugs the edge opposite the navigation sidebar.
  const inspector =
    wide && component && state ? (
      <PageSidebar
        className="stories-inspector-pane"
        collapsible
        defaultWidth={300}
        header={
          <div className="stories-inspector__head">
            <span>Props</span>
            <InspectorReset onChange={setOverrides} overrides={overrides} />
          </div>
        }
        label="Props"
        maxWidth={520}
        minWidth={220}
        resizable
        side={panelSide === 'right' ? 'left' : 'right'}
        storageKey="stories:inspector"
      >
        <div className="stories-inspector__scroll">
          <Inspector controls={state.controls} onChange={setOverrides} overrides={overrides} />
        </div>
      </PageSidebar>
    ) : null

  const showSidebar = !narrow || selection === null
  const showMain = !narrow || selection !== null

  return (
    <PageShell className="stories-shell">
      <PageHeader
        actions={
          <>
            <IconButton
              label="Refresh"
              onClick={() => {
                void loadWorkspaces()
                if (mode === 'explorer') void loadTree()
                else void loadCompare()
              }}
              variant="ghost"
            >
              <RefreshIcon />
            </IconButton>
            <IconButton label="Rebuild working tree" onClick={() => rebuild('worktree')} variant="ghost">
              <HammerIcon />
            </IconButton>
          </>
        }
        description={
          workspace
            ? `${workspace.name} · ${workspace.branch ?? shortSha(workspace.head)}${workspace.dirty ? ' · dirty' : ''}`
            : undefined
        }
        icon={<StoriesIcon />}
        onClose={onRequestClose}
        title="Stories"
      />
      <div className={`stories-root${narrow ? ' narrow' : ''}`} ref={rootRef}>
        <PageBody className="stories-body" side={panelSide}>
          {showSidebar ? sidebar : null}
          {showMain ? main : null}
          {showMain ? inspector : null}
        </PageBody>
      </div>
    </PageShell>
  )
}
