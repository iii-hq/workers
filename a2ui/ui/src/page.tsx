import { useCallback, useEffect, useRef, useState } from 'react'
import { strToU8, zipSync } from 'fflate'
import {
  Badge,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  EmptyState,
  List,
  ListItem,
  PageBody,
  PageHeader,
  PageMain,
  PageShell,
  PageSidebar,
  StatusPanel,
  type Host,
  type PageRenderProps,
} from '@iii-dev/console-ui'
import { exportCode, exportSurface, getSurface, listSurfaces, listTemplates, mutateSurface } from './data'
import { useSurfaceEvents } from './live'
import { Surface } from './surface'
import type { SurfaceExport, SurfaceRecord, SurfaceSummary, SurfaceTemplate } from './types'
import { writeCodeBundle, type WorkspaceExport } from './workspace'

export function A2uiPage({ host, panelSide, tabId, onRequestClose, conversationId, workingDir }: PageRenderProps & { host: Host }) {
  const [surfaces, setSurfaces] = useState<SurfaceSummary[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [selected, setSelected] = useState<SurfaceRecord | null>(null)
  const [loading, setLoading] = useState(false)
  const [exporting, setExporting] = useState(false)
  const [templates, setTemplates] = useState<SurfaceTemplate[]>([])
  const [error, setError] = useState<string | null>(null)
  const [workspaceExport, setWorkspaceExport] = useState<WorkspaceExport | null>(null)
  const importInput = useRef<HTMLInputElement>(null)

  const refresh = useCallback(async () => {
    if (!conversationId) {
      setSurfaces([])
      setSelectedId(null)
      setSelected(null)
      return
    }
    setLoading(true)
    try {
      const response = await listSurfaces(host, conversationId)
      setSurfaces(response.surfaces)
      setTemplates((await listTemplates(host, conversationId)).templates)
      setSelectedId((current) =>
        response.surfaces.some((surface) => surface.surface_id === current)
          ? current
          : (response.surfaces[0]?.surface_id ?? null),
      )
      setError(null)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause))
    } finally {
      setLoading(false)
    }
  }, [host, conversationId])

  useEffect(() => void refresh(), [refresh])
  useSurfaceEvents(host, conversationId, () => void refresh())

  useEffect(() => {
    if (!conversationId || !selectedId) {
      setSelected(null)
      return
    }
    let alive = true
    void getSurface(host, conversationId, selectedId)
      .then((surface) => alive && setSelected(surface))
      .catch((cause) => alive && setError(cause instanceof Error ? cause.message : String(cause)))
    return () => {
      alive = false
    }
  }, [host, conversationId, selectedId, surfaces])

  const downloadSelected = useCallback(async () => {
    if (!conversationId || !selected) return
    setExporting(true)
    try {
      const portable = await exportSurface(host, conversationId, selected.surface_id)
      downloadText(
        `${JSON.stringify(portable, null, 2)}\n`,
        `${safeFileName(selected.surface_id)}.a2ui.json`,
        'application/json',
      )
      setError(null)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause))
    } finally {
      setExporting(false)
    }
  }, [host, conversationId, selected])

  const writeReactApp = useCallback(async () => {
    if (!conversationId || !selected) return
    if (!workingDir) {
      setError('Choose a working directory in Harness before writing the React app.')
      return
    }
    setExporting(true)
    try {
      const bundle = await exportCode(host, conversationId, selected.surface_id, 'react')
      setWorkspaceExport(await writeCodeBundle(host, workingDir, bundle, selected.revision))
      setError(null)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause))
    } finally {
      setExporting(false)
    }
  }, [host, conversationId, selected, workingDir])

  const run = useCallback(
    async (functionId: string, payload: Record<string, unknown>) => {
      if (!conversationId) return
      setExporting(true)
      try {
        await mutateSurface(host, functionId, conversationId, payload)
        await refresh()
        setError(null)
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : String(cause))
      } finally {
        setExporting(false)
      }
    },
    [host, conversationId, refresh],
  )

  const downloadCode = useCallback(
    async (target: 'react' | 'worker') => {
      if (!conversationId || !selected) return
      setExporting(true)
      try {
        const bundle = await exportCode(host, conversationId, selected.surface_id, target)
        const archive = zipSync(
          Object.fromEntries(bundle.files.map((file) => [file.path, strToU8(file.content)])),
        )
        downloadBytes(
          archive,
          `${safeFileName(selected.surface_id)}-${target}.zip`,
          'application/zip',
        )
        setError(null)
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : String(cause))
      } finally {
        setExporting(false)
      }
    },
    [host, conversationId, selected],
  )

  const importFile = useCallback(
    (file: File | undefined) => {
      if (!file || !conversationId) return
      void file
        .text()
        .then((text) => run('a2ui::surface::import', { package: JSON.parse(text) as SurfaceExport }))
        .catch((cause) => setError(cause instanceof Error ? cause.message : String(cause)))
    },
    [conversationId, run],
  )

  return (
    <PageShell className="a2ui-page">
      <PageHeader
        title="A2UI"
        description={<span className="a2ui-page-description">Generative interfaces for this conversation</span>}
        actions={<Badge variant="accent">v0.9.1</Badge>}
        onClose={onRequestClose}
      />
      <PageBody side={panelSide}>
        <PageSidebar
          label="Interfaces"
          side={panelSide}
          header={
            <div className="a2ui-sidebar-head">
              <span>Interfaces</span>
              <span>{surfaces.length}</span>
            </div>
          }
          defaultWidth={272}
          minWidth={220}
          maxWidth={420}
          collapsible
          resizable
          storageKey={`a2ui:${tabId || 'page'}:interfaces`}
          className="a2ui-sidebar"
        >
          {!conversationId ? (
            <div className="a2ui-note">Open the page beside a Harness conversation.</div>
          ) : surfaces.length === 0 ? (
            <div className="a2ui-note">{loading ? 'Loading…' : 'Ask the agent to create an interface.'}</div>
          ) : (
            <List className="a2ui-list">
              {surfaces.map((surface) => (
                <ListItem
                  key={surface.surface_id}
                  selected={selectedId === surface.surface_id}
                  label={<span className="a2ui-list-title">{surface.title}</span>}
                  description={
                    <span className="a2ui-list-meta">
                      {surface.component_count} components · r{surface.revision}
                      {surface.pinned ? ' · pinned' : ''}
                    </span>
                  }
                  onClick={() => setSelectedId(surface.surface_id)}
                />
              ))}
            </List>
          )}
        </PageSidebar>
        <PageMain className="a2ui-main">
          {surfaces.length > 0 ? (
            <label className="a2ui-mobile-switcher">
              <span>Interface</span>
              <select
                aria-label="surface"
                value={selectedId ?? ''}
                onChange={(event) => setSelectedId(event.currentTarget.value)}
              >
                {surfaces.map((surface) => (
                  <option key={surface.surface_id} value={surface.surface_id}>
                    {surface.title} · r{surface.revision}
                  </option>
                ))}
              </select>
            </label>
          ) : null}
          <div className="a2ui-library-bar">
            <Button
              variant="ghost"
              size="sm"
              disabled={!conversationId}
              onClick={() => importInput.current?.click()}
            >
              Import JSON
            </Button>
            <input
              ref={importInput}
              className="a2ui-file-input"
              type="file"
              tabIndex={-1}
              aria-hidden="true"
              accept="application/json,.json"
              onChange={(event) => {
                importFile(event.currentTarget.files?.[0])
                event.currentTarget.value = ''
              }}
            />
            {templates.map((template) => (
              <span className="a2ui-template" key={template.template_id}>
                <Button
                  variant="ghost"
                  size="sm"
                  title="Create a new surface from this saved template"
                  onClick={() =>
                    void run('a2ui::template::apply', {
                      template_id: template.template_id,
                      surface_id: `template-${safeFileName(template.template_id).slice(0, 80)}-${Date.now()}`,
                    })
                  }
                >
                  {template.title}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  aria-label={`delete template ${template.title}`}
                  title="Delete this saved template"
                  onClick={() => {
                    if (window.confirm(`Delete template “${template.title}”?`)) {
                      void run('a2ui::template::delete', { template_id: template.template_id })
                    }
                  }}
                >
                  ×
                </Button>
              </span>
            ))}
          </div>
          {error ? (
            <div className="a2ui-page-error" role="alert">
              <StatusPanel variant="alert" headline="A2UI page error" detail={error} />
              <Button variant="ghost" size="sm" onClick={() => setError(null)}>
                Dismiss
              </Button>
            </div>
          ) : null}
          {workspaceExport ? (
            <div className="a2ui-workspace-export" role="status">
              <div>
                <strong>React app ready in the workspace</strong>
                <span title={workspaceExport.directory}>{workspaceExport.directory}</span>
                <small>
                  {workspaceExport.written} files written
                  {workspaceExport.existing > 0 ? ` · ${workspaceExport.existing} existing files preserved` : ''}
                  {' · '}run it in Shell, then open the local URL in Browser
                </small>
              </div>
              <a href="#/ext/shell">open Shell</a>
            </div>
          ) : null}
          {!conversationId ? (
            <EmptyState
              title="No Harness conversation selected"
              description="Open A2UI beside a Harness conversation to create and manage its generated interfaces."
            />
          ) : selected ? (
            <>
              <div className="a2ui-surface-head">
                <div className="a2ui-surface-title">
                  <h2 title={selected.title}>{selected.title}</h2>
                  <div className="a2ui-surface-meta">
                    <span title={selected.surface_id}>{selected.surface_id}</span>
                    <Badge>r{selected.revision}</Badge>
                  </div>
                </div>
                <div className="a2ui-surface-actions">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button variant="ghost" size="sm" disabled={exporting} aria-label="surface actions">
                        {exporting ? 'Working…' : 'Actions'}
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      <DropdownMenuLabel>Interface</DropdownMenuLabel>
                      <DropdownMenuItem
                        title="Keep this surface at the top of the conversation library"
                        onSelect={() =>
                          void run('a2ui::surface::pin', {
                            surface_id: selected.surface_id,
                            pinned: !selected.pinned,
                          })
                        }
                      >
                        {selected.pinned ? 'Unpin interface' : 'Pin interface'}
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        title="Restore the previous saved revision"
                        disabled={selected.history.length === 0}
                        onSelect={() =>
                          void run('a2ui::surface::undo', {
                            surface_id: selected.surface_id,
                          })
                        }
                      >
                        Undo latest change
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        title="Create an independent copy in this conversation"
                        onSelect={() =>
                          void run('a2ui::surface::duplicate', {
                            surface_id: selected.surface_id,
                            new_surface_id: `${selected.surface_id.slice(0, 88)}-copy-${Date.now()}`,
                          })
                        }
                      >
                        Duplicate interface
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        title="Reuse this surface as a starting point later"
                        onSelect={() =>
                          void run('a2ui::template::save', {
                            surface_id: selected.surface_id,
                            template_id: selected.surface_id,
                            description: `Saved from ${selected.title}`,
                          })
                        }
                      >
                        Save as template
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuLabel>Workspace</DropdownMenuLabel>
                      <DropdownMenuItem
                        disabled={!workingDir}
                        title={workingDir ? 'Create a complete editable React app as workspace files' : 'Choose a working directory in Harness first'}
                        onSelect={() => void writeReactApp()}
                      >
                        Write React app to workspace
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuLabel>Export</DropdownMenuLabel>
                      <DropdownMenuItem title="Portable A2UI data for re-import" onSelect={() => void downloadSelected()}>Download JSON package</DropdownMenuItem>
                      <DropdownMenuItem title="ZIP with a complete runnable React app" onSelect={() => void downloadCode('react')}>Download React app ZIP</DropdownMenuItem>
                      <DropdownMenuItem title="ZIP with an iii worker template that serves this surface as data" onSelect={() => void downloadCode('worker')}>Download data worker template</DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem
                        title="Permanently remove this surface"
                        onSelect={() => {
                          if (window.confirm(`Delete surface “${selected.title}”?`)) {
                            void run('a2ui::surface::delete', { surface_id: selected.surface_id })
                          }
                        }}
                      >
                        Delete interface
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </div>
              </div>
              <div className="a2ui-stage">
                <Surface host={host} surface={selected} />
              </div>
            </>
          ) : (
            <EmptyState
              title="No generated interface"
              description="Ask the Harness agent for a dashboard, form, summary, or decision surface."
            />
          )}
        </PageMain>
      </PageBody>
    </PageShell>
  )
}

function safeFileName(value: string): string {
  return value.replace(/[^a-z0-9._-]+/gi, '-').replace(/^-+|-+$/g, '') || 'surface'
}

function downloadText(content: string, name: string, type: string) {
  downloadBytes(content, name, type)
}

function downloadBytes(content: BlobPart, name: string, type: string) {
  const url = URL.createObjectURL(new Blob([content], { type }))
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = name
  document.body.appendChild(anchor)
  anchor.click()
  anchor.remove()
  window.setTimeout(() => URL.revokeObjectURL(url), 1_000)
}
