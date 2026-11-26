import type { Host, PageRenderProps } from '@iii-dev/console-ui'
import { Button, EmptyState, PageBody, PageHeader, PageMain, PageShell, Skeleton } from '@iii-dev/console-ui'
import { useContainerNarrow, usePaneState } from '@iii-dev/console-ui/hooks'
import { Cloud } from 'lucide-react'
import { useCallback, useEffect, useRef } from 'react'
import { LaunchNote } from './AgentRow'
import { DeployArea } from './DeployFlow'
import { useActions, useStatus } from './hub'
import { ManageArea } from './Manage'
import { needsConnect, needsProject, whoLine } from './model'
import { ErrorPanel, Notice } from './parts'
import { type AreaProps, defaultRoute, envState, type Route, routeFromContext } from './route'
import { SetupArea } from './Setup'
import { ProjectShell } from './Sidebar'
import { StartArea } from './Start'
import type { Status } from './types'

const NARROW_BELOW = 640

/**
 * The route the page shows: the stored one (else the default) with its '' env filled in. Start and setup
 * open always; any other place needs a sign-in and a project in use, else the default.
 */
function shown(stored: Route | null, s: Status): Route {
  const r = stored ?? defaultRoute(s)
  if (r.at === 'start' || r.at === 'setup') return r
  if (needsConnect(s) || needsProject(s)) return defaultRoute(s)
  if (!r.env) {
    const id = s.project?.environment?.id
    return id ? { ...r, env: id } : defaultRoute(s)
  }
  return r
}

/**
 * The one `iii-cloud` page: a router over Projects, first time, check and deploy, and an environment (with
 * its project's sidebar) or its secrets. The route comes from the pane's
 * context (each new `panels.open` re-navigates) and stays per pane; areas move with `go`.
 */
export function CloudPage({
  host,
  onRequestClose,
  panelContext,
  panelSide,
  paneId,
  tabId,
  workingDir,
}: PageRenderProps & { host: Host }) {
  const paneKey = paneId || tabId
  const [stored, setStored] = usePaneState<Route | null>(
    `iii-cloud:route:${paneKey}`,
    routeFromContext(panelContext?.context),
  )
  // The last `panels.open` this pane followed: the same id again (a reload restoring the pane) is not a new one.
  const [seen, setSeen] = usePaneState<number | null>(`iii-cloud:context:${paneKey}`, null)
  const { ref, narrow } = useContainerNarrow({ below: NARROW_BELOW })
  const { status, error, loaded, refresh } = useStatus(host, { poll: true })
  const { error: actionError, call } = useActions(host)
  const ctxId = panelContext?.id
  // The person sent this pane where it is (`go`, or a new `panels.open`): only then may it select an environment.
  const navigated = useRef(false)
  const go = useCallback(
    (r: Route) => {
      navigated.current = true
      setStored(r)
    },
    [setStored],
  )

  useEffect(() => {
    if (ctxId === undefined || ctxId === seen) return
    setSeen(ctxId)
    navigated.current = true
    setStored(routeFromContext(panelContext?.context))
    // A new `panels.open` call re-navigates; the context object itself is not a dependency.
  }, [ctxId])

  const route = status ? shown(stored, status) : null
  // An env or deploy route for another of the project's environments: the person's own move selects it
  // (once per env); a route restored on mount, or one another pane moved the selection away from, goes home.
  const env = route && (route.at === 'env' || route.at === 'deploy' || route.at === 'secrets') ? route.env : null
  const state = status && env ? envState(status, env) : null
  const want = state === 'switching' ? env : null
  const asked = useRef<string | null>(null)
  useEffect(() => {
    if (state === 'selected') navigated.current = false
    if (!want) {
      asked.current = null
      return
    }
    if (!navigated.current) {
      setStored({ at: 'env', env: '' })
      return
    }
    const p = status?.project
    if (asked.current === want || !p?.workspace) return
    asked.current = want
    void call('env', 'iii-cloud::project::use', {
      path: p.path,
      environment_id: want,
      mode: p.workspace.mode,
    }).then((out) => {
      if (out === null) setStored({ at: 'env', env: '' })
    })
  }, [want, state])

  let body: React.ReactNode
  if (!status || !route) {
    body = (
      <PageMain className="iii-cloud-main">
        <div className="iii-cloud-body">
          {error ? (
            <Notice
              variant="alert"
              headline="The iii cloud worker is not answering"
              detail="Check that the worker is running. This page keeps trying."
              action={
                <Button size="sm" variant="secondary" onClick={() => refresh(true)}>
                  Retry
                </Button>
              }
            />
          ) : loaded ? (
            <EmptyState icon={Cloud} title="No status yet" description="The iii-cloud worker has not reported." />
          ) : (
            <Skeleton className="iii-cloud-skeleton" />
          )}
        </div>
      </PageMain>
    )
  } else {
    const props: Omit<AreaProps, 'route'> = { host, status, go, narrow, paneKey, workingDir }
    body = (
      <PageBody side={panelSide} className="iii-cloud-pagebody">
        {route.at === 'start' ? (
          <StartArea {...props} route={route} />
        ) : route.at === 'setup' ? (
          <SetupArea {...props} route={route} />
        ) : route.at === 'deploy' ? (
          <DeployArea key={route.env} {...props} route={route} />
        ) : route.at === 'env' ? (
          <ProjectShell host={host} status={status} route={route} go={go} narrow={narrow}>
            <ManageArea {...props} route={route} />
          </ProjectShell>
        ) : (
          <ManageArea {...props} route={route} />
        )}
      </PageBody>
    )
  }

  return (
    <PageShell ref={ref} className="iii-cloud-shell" data-narrow={narrow ? '' : undefined}>
      <PageHeader
        icon={<Cloud />}
        title="iii cloud"
        description={status ? whoLine(status) : undefined}
        onClose={onRequestClose}
      />
      {status ? <LaunchNote /> : null}
      {actionError ? (
        <div className="iii-cloud-body">
          <ErrorPanel error={actionError} />
        </div>
      ) : null}
      {body}
    </PageShell>
  )
}
