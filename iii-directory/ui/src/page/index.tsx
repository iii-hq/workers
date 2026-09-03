/**
 * The directory page (#/ext/directory): a full-height application shell —
 * slim product top bar, a navigation sidebar carrying the directory
 * switcher, and a document workspace that opens one entry in the shared
 * CodeEditor/MarkdownPreview pair, saving through the worker's update
 * functions.
 *
 * Both collections stay MOUNTED (the inactive one is display:none) so an
 * unsaved draft survives switching between them.
 */

import { type Host, PageHeader, type PageRenderProps, PageShell, SegmentedControl } from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import { formatBytes, formatRelativeTime } from '../lib/format'
import { MarkdownFileIcon } from '../lib/widgets'
import { AgentForm, AgentFormSkeleton } from './agent-fields'
import { type BrowserAdapter, CollectionBrowser } from './browser'
import { TokenIcon } from './token-icons'

interface SkillRow {
  id: string
  title: string
  description: string
  bytes: number
  modified_at: string
}

interface AgentRow {
  id: string
  name: string
  description: string
  logo: string | null
  icon: string | null
  color: string | null
  modified_at: string
  /** Bundled with the worker (`iii`, the base identity): editing creates
   * the local file that shadows it; nothing to delete. */
  builtin?: boolean
}

const skillsAdapter: BrowserAdapter = {
  noun: 'skill',
  crumbRoot: 'skills',
  nameKeys: ['name', 'title'],
  defaultNameKey: 'name',
  modelInvocationOption: true,
  // Skill ids are slash-separated lowercase segments (ns/skill/…).
  namePattern: /^[a-z0-9_-]+(\/[a-z0-9_-]+)*$/,
  nameHint: 'enter an id of lowercase slash-separated segments (a–z, 0–9, hyphens or underscores)',
  onChangeType: 'directory::skills::on-change',
  emptyTitle: 'Select a skill',
  emptyBody:
    'Choose a skill from the sidebar to view and edit its markdown. New skills arrive through the new-skill button, downloads (directory::skills::download), or direct edits to the skills folder.',
  async list(host) {
    const out = await host.iii.trigger<{ skills: SkillRow[] }>('directory::skills::list', { include_description: true })
    return (out.skills ?? []).map((s) => ({
      key: s.id,
      title: s.title,
      description: s.description,
      fine: `${formatBytes(s.bytes)} · ${formatRelativeTime(s.modified_at)}`,
    }))
  },
  async load(host, id) {
    const out = await host.iii.trigger<{ body: string; raw?: string | null }>('directory::skills::get', {
      id,
      raw: true,
    })
    // `raw` is the exact on-disk file; body (frontmatter-stripped) is the
    // fallback against a not-yet-updated worker.
    return out.raw ?? out.body
  },
  async save(host, id, content) {
    const out = await host.iii.trigger<{ id: string }>('directory::skills::update', { id, content })
    return out.id ?? id
  },
  async create(host, id, content) {
    const out = await host.iii.trigger<{ id: string }>('directory::skills::create', { id, content })
    return out.id ?? id
  },
  async remove(host, id) {
    await host.iii.trigger('directory::skills::delete', { id })
  },
}

export const agentsAdapter: BrowserAdapter = {
  noun: 'agent profile',
  crumbRoot: 'agents',
  // The id (file stem) and the display name are different things for an
  // agent: "Release Captain" lives in frontmatter `name`, the file is
  // `release-captain.md`.
  separateId: {
    pattern: /^[a-z0-9_-]+$/,
    hint: 'enter a name containing at least one letter or number — it becomes the file name',
  },
  nameRequired: true,
  newTemplate: '---\nname: \ndescription: ""\n---\n\n',
  newTemplateStartsClean: true,
  extraManagedKeys: ['logo', 'skills', 'model', 'reasoning_effort', 'icon', 'color', 'extends'],
  customForm: (ctx) => <AgentForm {...ctx} />,
  customLoading: () => <AgentFormSkeleton />,
  customFormOwnsContent: true,
  customFormOwnsWorkspaceHeader: true,
  prominentListItems: true,
  onChangeType: 'directory::agents::on-change',
  emptyTitle: 'Select an agent profile',
  emptyBody: 'Choose an agent profile from the sidebar to edit its identity, default model, system prompt, and skills.',
  async list(host) {
    const out = await host.iii.trigger<{ agents: AgentRow[] }>('directory::agents::list')
    return (out.agents ?? []).map((a) => ({
      key: a.id,
      // The row glyph is the SAME token glyph the avatar picker and the
      // console session tree render — one identity, one pictogram.
      icon: <TokenIcon token={a.icon || 'agent'} size={20} />,
      title: a.name,
      description: a.description,
      fine: a.builtin ? 'Built-in · edits save a local override' : formatRelativeTime(a.modified_at),
      ...(a.builtin ? { noDelete: true } : {}),
    }))
  },
  async load(host, id) {
    // `raw` is the profile's OWN file; `system_prompt` would be the
    // inheritance-resolved prompt, never what the editor should save.
    const out = await host.iii.trigger<{
      system_prompt: string
      raw?: string | null
    }>('directory::agents::get', { id, raw: true })
    return out.raw ?? out.system_prompt
  },
  async save(host, id, content) {
    const out = await host.iii.trigger<{ id: string }>('directory::agents::update', { id, content })
    return out.id ?? id
  },
  async create(host, id, content) {
    const out = await host.iii.trigger<{ id: string }>('directory::agents::create', { id, content })
    return out.id ?? id
  },
  async remove(host, id) {
    await host.iii.trigger('directory::agents::delete', { id })
  },
}

type Collection = 'skills' | 'agents'

interface PendingCollectionAction {
  id: number
  collection: Collection
  key?: string
  action?: 'create'
}

export const COLLECTIONS: { value: Collection; label: string }[] = [
  { value: 'skills', label: 'Skills' },
  { value: 'agents', label: 'Agent Profiles' },
]

const ADAPTERS: Record<Collection, BrowserAdapter> = {
  skills: skillsAdapter,
  agents: agentsAdapter,
}

export function DirectoryPage({
  host,
  panelSide = 'left',
  tabId = '',
  onRequestClose,
  panelContext,
  commands,
}: { host: Host } & Partial<PageRenderProps>) {
  const [collection, setCollection] = useState<Collection>('skills')
  const [pendingOpen, setPendingOpen] = useState<PendingCollectionAction | null>(null)

  // Panel context can open a palette entry or start a collection's creation
  // flow. `panelContext.id` is monotonic, so a repeated action still applies.
  const appliedContextRef = useRef(0)
  useEffect(() => {
    if (!panelContext || panelContext.id === appliedContextRef.current) return
    appliedContextRef.current = panelContext.id
    const context = panelContext.context as {
      collection?: string
      key?: string
      action?: string
    } | null
    const collectionValue = context?.collection
    if (
      !context ||
      typeof collectionValue !== 'string' ||
      !COLLECTIONS.some((c) => c.value === collectionValue)
    ) {
      return
    }
    const nextCollection = collectionValue as Collection
    const opensEntry = typeof context.key === 'string'
    const startsCreate = context.action === 'create'
    if (!opensEntry && !startsCreate) return
    setCollection(nextCollection)
    setPendingOpen({
      id: panelContext.id,
      collection: nextCollection,
      ...(opensEntry ? { key: context.key } : {}),
      ...(startsCreate ? { action: 'create' } : {}),
    })
  }, [panelContext])

  const switcher = (
    <SegmentedControl<Collection>
      value={collection}
      onChange={setCollection}
      options={COLLECTIONS}
      iconOnly
      className="dir-ui-collection-tabs"
      aria-label="Browse skills or agent profiles"
    />
  )

  return (
    <PageShell className="dir-ui-shell">
      <PageHeader
        icon={<MarkdownFileIcon />}
        title="Directory"
        description="Filesystem-backed skills and agent profiles"
        onClose={onRequestClose}
      />
      {COLLECTIONS.map((c) => (
        <div key={c.value} className="dir-ui-shell-body" hidden={collection !== c.value}>
          <CollectionBrowser
            host={host}
            adapter={ADAPTERS[c.value]}
            nav={switcher}
            panelSide={panelSide}
            storageKey={`iii-directory-ui:${tabId || 'page'}:${c.value}`}
            commands={commands}
            active={collection === c.value}
            pendingOpen={pendingOpen && pendingOpen.collection === c.value ? pendingOpen : null}
          />
        </div>
      ))}
    </PageShell>
  )
}
