import { Button, Input, List, ListGroupLabel, ListItem, SearchField, Skeleton, StatusPanel } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useWorkerLive } from '@iii-dev/console-ui/hooks'
import { useEffect, useId, useRef, useState } from 'react'
import type { Api } from './api'
import { currentRow } from './columns'
import { firstVariable, matchesQuery, middleTruncate, resourceMeta, searchNote, uniqueBy } from './presentation'
import type { Resource, ResourcePage, ResourceTemplate, Server } from './types'

/** Per-tab handler id of the open picker's live read (the host namespaces it `::<browserId>`). */
const RESOURCES_FN = 'iii::mcp-client-ui::resources'
const SKELETON_LINES = [0, 1, 2]
// The mono characters a uri line holds in one half of the Test tab at 1440: past them CSS would cut the tail too.
const URI_CHARS = 44

/** The first page of resources and every template, as one live read answers them. */
interface Listing {
  page: ResourcePage | null
  templates: ResourceTemplate[]
}

/** The pages "Load more" added, held against the first page they follow: a re-read drops them. */
interface More {
  first: Listing
  resources: Resource[]
  cursor: string | undefined
  loading: boolean
  error: string | null
}

/** The Input half of `resources::read`: the uri Read sends, and the server's resources to pick it from. */
export function ResourcePicker({
  api,
  server,
  uri,
  onUri,
}: {
  api: Api
  server: Server
  uri: string
  onUri: (uri: string) => void
}) {
  const id = useId()
  const input = useRef<HTMLInputElement>(null)
  const [query, setQuery] = useState('')
  const [more, setMore] = useState<More | null>(null)
  // The template just chosen; a new object each time, so choosing the same one again selects again.
  const [picked, setPicked] = useState<{ template: string } | null>(null)
  const registered = (name: string) => server.resource_functions.some((fn) => fn.name === name && fn.registered)
  const listId = `mcp::${server.name}::resources::list`
  const listed = registered('list')
  const templated = registered('templates')

  // The worker sends a `tools` event on the server's `resources/list_changed`: the open list reads again.
  const live = useWorkerLive<Listing>({
    iii: api.iii,
    handlerId: RESOURCES_FN,
    sources: listed
      ? [
          {
            function_id: 'mcp-client::on-servers-changed',
            filters: { 'input.server': { $eq: server.name }, 'input.kind': { $eq: 'tools' } },
          },
        ]
      : [],
    fetch: async () => {
      const [page, templates] = await Promise.all([
        listed ? api.call(listId, {}) : null,
        // Column 2 already says when the server's template list failed.
        templated
          ? api.call(`mcp::${server.name}::resources::templates`, {}).then(
              (reply) => (reply as { templates?: ResourceTemplate[] }).templates ?? [],
              () => [],
            )
          : [],
      ])
      return { page: page as ResourcePage | null, templates }
    },
  })
  const data = live.data
  const held = more?.first === data ? more : null
  const cursor = held ? held.cursor : data?.page?.next_cursor
  const resources = uniqueBy([...(data?.page?.resources ?? []), ...(held?.resources ?? [])], (r) => r.uri)

  const loadMore = () => {
    if (data === null || cursor === undefined) return
    const first = data
    const loaded = held?.resources ?? []
    setMore({ first, resources: loaded, cursor, loading: true, error: null })
    // A reply to a list the page has since read again is dropped.
    api.call(listId, { cursor }).then(
      (reply) => {
        const page = reply as ResourcePage
        setMore((prev) =>
          prev?.first === first
            ? {
                first,
                resources: [...loaded, ...page.resources],
                cursor: page.next_cursor,
                loading: false,
                error: null,
              }
            : prev,
        )
      },
      (err) =>
        setMore((prev) => (prev?.first === first ? { ...prev, loading: false, error: errorMessage(err) } : prev)),
    )
  }

  // Runs once the chosen template is the input's value, so the selection lands on the new text.
  useEffect(() => {
    const el = input.current
    if (!picked || !el || uri !== picked.template) return
    el.focus()
    const range = firstVariable(picked.template)
    if (range) el.setSelectionRange(range.start, range.end)
    setPicked(null)
  }, [picked, uri])

  const shown = resources.filter((r) => matchesQuery(query, r.name, r.uri))
  const allTemplates = uniqueBy(data?.templates ?? [], (t) => t.uri_template)
  const templates = allTemplates.filter((t) => matchesQuery(query, t.name, t.uri_template))
  const note = searchNote(resources.length, cursor !== undefined)

  let list = null
  if (live.error) {
    list = (
      <StatusPanel
        variant="alert"
        headline="Could not list resources"
        detail={live.error}
        action={
          <Button variant="secondary" size="sm" onClick={live.refresh}>
            Retry
          </Button>
        }
      />
    )
  } else if (data === null) {
    list = listed ? SKELETON_LINES.map((n) => <Skeleton key={n} className="mcp-ui-skeleton-line" />) : null
  } else {
    list = (
      <>
        {listed && resources.length === 0 ? (
          <p className="mcp-ui-quiet-line">This server lists no resources. Type a uri to read one.</p>
        ) : null}
        {resources.length + allTemplates.length > 0 ? (
          <div className="mcp-ui-field">
            <SearchField aria-label="Filter resources" value={query} onChange={setQuery} />
            {note ? <p className="mcp-ui-quiet-line">{note}</p> : null}
          </div>
        ) : null}
        {shown.length > 0 || cursor !== undefined ? (
          <List role="group" aria-label="Resources" className="mcp-ui-picker">
            {shown.map((r) => (
              <ListItem
                key={r.uri}
                className="mcp-ui-resource"
                {...currentRow(r.uri === uri)}
                title={r.uri}
                label={r.name}
                description={middleTruncate(r.uri, URI_CHARS)}
                trailing={resourceMeta(r.mime_type, r.size)}
                onClick={() => onUri(r.uri)}
              />
            ))}
            {cursor !== undefined ? (
              <div className="mcp-ui-more-row">
                <Button variant="ghost" size="sm" disabled={held?.loading} onClick={loadMore}>
                  Load more
                </Button>
                {held?.error ? (
                  <p className="mcp-ui-input-error" role="alert">
                    {held.error}
                  </p>
                ) : null}
              </div>
            ) : null}
          </List>
        ) : null}
        {/* Outside the scrolling list, so a long resource list never hides the templates under it. */}
        {templates.length > 0 ? (
          <List role="group" aria-labelledby={`${id}-templates`} className="mcp-ui-group mcp-ui-templates">
            <ListGroupLabel id={`${id}-templates`}>Templates</ListGroupLabel>
            {templates.map((t) => (
              <ListItem
                key={t.uri_template}
                className="mcp-ui-resource"
                {...currentRow(t.uri_template === uri)}
                title={t.uri_template}
                label={t.name}
                description={middleTruncate(t.uri_template, URI_CHARS)}
                trailing={t.mime_type}
                onClick={() => {
                  onUri(t.uri_template)
                  setPicked({ template: t.uri_template })
                }}
              />
            ))}
          </List>
        ) : null}
      </>
    )
  }

  return (
    <>
      <div className="mcp-ui-field">
        <label className="mcp-ui-field-label" htmlFor={id}>
          Resource uri
        </label>
        <Input
          ref={input}
          id={id}
          className="mcp-ui-uri"
          value={uri}
          onChange={onUri}
          spellCheck={false}
          autoComplete="off"
        />
      </div>
      {list}
    </>
  )
}
