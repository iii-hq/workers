/**
 * The Artifacts page (directory editor archetype): artifacts in the sidebar,
 * the selected one's preview in the main column with Share…, Delete and its
 * live links (Revoke). `host.panels.open({ context: { id } })` from the chat
 * card selects an artifact.
 */

import {
  Button,
  ConfirmDialog,
  EmptyState,
  type Host,
  IconButton,
  List,
  ListItem,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  SearchField,
  StatusPanel,
  Toolbar,
} from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState, useWorkerLive } from '@iii-dev/console-ui/hooks'
import { FileCode2, Files, FileText, Image, Share2, Trash2, X } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { ArtifactPreview } from '../components/ArtifactPreview'
import { ShareDialog, ShareLinks } from '../components/ShareDialog'
import { VersionSelect } from '../components/VersionSelect'
import { useShareRefresh } from '../lib/refresh'
import {
  type ArtifactFormat,
  type ArtifactRecord,
  createApi,
  expiresLabel,
  type IndexEntry,
  isRecord,
  MUTATING_FUNCTIONS,
  type ShareView,
} from '../lib/api'

const FORMAT_ICON: Record<ArtifactFormat, typeof FileCode2> = {
  html: FileCode2,
  markdown: FileText,
  svg: Image,
}

interface Snapshot {
  artifacts: IndexEntry[]
  /** Every artifact, before the list cap. */
  total: number
  truncated: boolean
  shares: ShareView[]
}

export function ArtifactPage({ host, ...props }: PageRenderProps & { host: Host }) {
  const api = useMemo(() => createApi(host), [host])
  const paneKey = props.paneId || props.tabId || 'default'
  const [selected, setSelected] = usePaneState<string | null>(`artifact-ui:selected:${paneKey}`, null)
  const [query, setQuery] = useState('')
  const { ref, narrow } = useContainerNarrow()

  const live = useWorkerLive<Snapshot>({
    iii: host.iii,
    sources: MUTATING_FUNCTIONS,
    handlerId: 'iii::artifact-ui::events',
    fetch: async () => {
      const [list, shares] = await Promise.all([api.list(), api.shares()])
      return { artifacts: list.artifacts, total: list.total, truncated: list.truncated, shares: shares.shares }
    },
  })
  useShareRefresh(live.data?.shares, live.refresh)

  // A chat card's Open delivers { id }: select it.
  const contextId = props.panelContext?.id
  useEffect(() => {
    const context = props.panelContext?.context
    if (isRecord(context) && typeof context.id === 'string') setSelected(context.id)
    // eslint-disable-next-line react-hooks/exhaustive-deps -- one selection per delivered context
  }, [contextId])

  const artifacts = live.data?.artifacts ?? []
  const needle = query.trim().toLowerCase()
  const visible = needle
    ? artifacts.filter((a) => `${a.title} ${a.kind}`.toLowerCase().includes(needle))
    : artifacts
  const shareCount = (id: string) => live.data?.shares.filter((s) => s.artifact_id === id).length ?? 0

  return (
    <PageShell className="artifact-ui-shell" ref={ref}>
      <PageHeader
        icon={<Files />}
        title="Artifacts"
        description="Designs, prototypes and docs you can share"
        onClose={props.onRequestClose}
      />
      <PageBody side={props.panelSide}>
        <PageSidebar
          label="Artifacts"
          side={props.panelSide}
          narrow={narrow}
          narrowMode="drawer"
          header={<SearchField value={query} onChange={setQuery} aria-label="Filter artifacts" />}
        >
          {live.error ? (
            <StatusPanel variant="alert" headline="Could not list artifacts" detail={live.error} />
          ) : visible.length === 0 && !live.loading ? (
            <EmptyState
              compact
              icon={Files}
              title={needle ? 'No match' : 'No artifacts yet'}
              description={
                needle ? 'Try another title or kind.' : 'Ask the agent to create a design, prototype or RFC.'
              }
            />
          ) : (
            <List aria-label="Artifacts">
              {visible.map((a) => {
                const Icon = FORMAT_ICON[a.format] ?? FileCode2
                const links = shareCount(a.id)
                return (
                  <ListItem
                    key={a.id}
                    selected={a.id === selected}
                    leading={<Icon />}
                    label={a.title}
                    description={`${a.kind} · v${a.latest_version} · ${formatRelative(a.updated_at * 1000)}`}
                    trailing={links ? <Share2 aria-label={`${links} live links`} /> : undefined}
                    onClick={() => setSelected(a.id)}
                  />
                )
              })}
            </List>
          )}
          {live.data?.truncated ? (
            <p className="artifact-ui-muted artifact-ui-truncated">
              Showing {live.data.artifacts.length} of {live.data.total}. Raise <code>max_list</code> in the
              Artifacts settings to list the rest.
            </p>
          ) : null}
        </PageSidebar>
        <PageMain className="artifact-ui-main">
          {selected ? (
            <ArtifactDetail
              key={selected}
              host={host}
              id={selected}
              updatedAt={artifacts.find((a) => a.id === selected)?.updated_at ?? 0}
              latestVersion={artifacts.find((a) => a.id === selected)?.latest_version ?? 0}
              shares={(live.data?.shares ?? []).filter((s) => s.artifact_id === selected)}
              onDeleted={() => setSelected(null)}
              onChanged={live.refresh}
            />
          ) : (
            <EmptyState
              icon={Files}
              title="Pick an artifact"
              description="Preview any version, share it with an expiring link, or revoke links."
            />
          )}
        </PageMain>
      </PageBody>
    </PageShell>
  )
}

/** `3 views, last 5m ago from BR` / `not opened yet`. */
function viewsLabel(share: ShareView): string {
  if (share.views === 0) return 'not opened yet'
  const count = `${share.views} ${share.views === 1 ? 'view' : 'views'}`
  const when = share.last_viewed_at ? `, last ${formatRelative(share.last_viewed_at * 1000)}` : ''
  const where = share.last_country ? ` from ${share.last_country}` : ''
  return count + when + where
}

function ArtifactDetail({
  host,
  id,
  updatedAt,
  latestVersion,
  shares,
  onDeleted,
  onChanged,
}: {
  host: Host
  id: string
  /** The index row's `updated_at` and `latest_version`: either changing
      re-reads the record (two saves can share one second). */
  updatedAt: number
  latestVersion: number
  shares: ShareView[]
  onDeleted(): void
  onChanged(): void
}) {
  const api = useMemo(() => createApi(host), [host])
  const [record, setRecord] = useState<ArtifactRecord | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [version, setVersion] = useState<number | undefined>(undefined)
  const [sharing, setSharing] = useState(false)
  const [deleting, setDeleting] = useState(false)
  const [revokeError, setRevokeError] = useState<string | null>(null)

  const revoke = async (token: string) => {
    try {
      await api.revoke(token)
      setRevokeError(null)
    } catch (err) {
      setRevokeError(errorMessage(err))
    } finally {
      // Reconcile either way: a revoke can fail after partial progress.
      onChanged()
    }
  }

  useEffect(() => {
    let current = true
    api.get(id).then(
      (res) => {
        if (!current) return
        setRecord(res.artifact)
        setError(null)
      },
      (err) => current && setError(errorMessage(err)),
    )
    return () => {
      current = false
    }
  }, [api, id, updatedAt, latestVersion])

  if (error) return <StatusPanel variant="alert" headline="Could not load this artifact" detail={error} />
  if (!record) return null

  const shown = version ?? record.latest_version

  return (
    <div className="artifact-ui-detail">
      <Toolbar
        aria-label="Artifact actions"
        end={
          <>
            <Button variant="primary" size="sm" onClick={() => setSharing(true)}>
              <Share2 />
              Share…
            </Button>
            <IconButton variant="ghost" label="Delete artifact" onClick={() => setDeleting(true)}>
              <Trash2 />
            </IconButton>
          </>
        }
      >
        <span className="artifact-ui-detail-title">{record.title}</span>
        <VersionSelect record={record} value={shown} onChange={setVersion} />
      </Toolbar>
      <ArtifactPreview api={api} id={record.id} version={shown} size="page" />
      <section className="artifact-ui-shares" aria-label="Live links">
        <h3 className="artifact-ui-section-title">Live links</h3>
        {revokeError ? (
          <StatusPanel variant="alert" headline="Could not revoke the link" detail={revokeError} />
        ) : null}
        {shares.length === 0 ? (
          <p className="artifact-ui-muted">No live links. Share… creates one that expires on its own.</p>
        ) : (
          shares.map((share) => (
            <div key={share.token} className="artifact-ui-share-row">
              <div className="artifact-ui-share-meta">
                <span>
                  v{share.version} · {share.public ? 'public' : 'local'}
                  {share.protected ? ' · passcode' : ''}
                  {share.once ? ' · single view' : ''} · expires {expiresLabel(share.expires_at)} ·{' '}
                  {viewsLabel(share)}
                </span>
                <IconButton variant="ghost" label="Revoke link" onClick={() => revoke(share.token)}>
                  <X />
                </IconButton>
              </div>
              <ShareLinks share={share} />
            </div>
          ))
        )}
      </section>
      <ShareDialog
        api={api}
        artifact={record}
        version={shown}
        open={sharing}
        onOpenChange={setSharing}
        onShared={onChanged}
      />
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        tone="danger"
        title={`Delete ${record.title}?`}
        description="Every version is removed and its links stop working. This cannot be undone."
        confirmLabel="Delete"
        busyLabel="Deleting…"
        onConfirm={async () => {
          await api.remove(record.id)
          onDeleted()
          onChanged()
        }}
      />
    </div>
  )
}
