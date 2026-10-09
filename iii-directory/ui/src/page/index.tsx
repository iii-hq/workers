/**
 * The directory page (page `directory`): a full-height application shell —
 * slim product top bar, a navigation sidebar carrying the directory
 * switcher, and a document workspace that opens one entry in the shared
 * CodeEditor/MarkdownPreview pair, saving through the worker's update
 * functions.
 *
 * Both collections stay MOUNTED (the inactive one is display:none) so an
 * unsaved draft survives switching between them.
 */

import { type Host, PageHeader, type PageRenderProps, PageShell, SegmentedControl } from '@iii-dev/console-ui'
import { formatBytes } from '@iii-dev/console-ui/format'
import { FileText, Puzzle } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { kitsApi, useKitsChange } from '../kits/api'
import { type KitsRoute, KitsView } from '../kits/KitsView'
import { ago } from '../lib/format'
import { AgentForm, AgentFormSkeleton, TokenIcon } from './agent-fields'
import { type BrowserAdapter, CollectionBrowser } from './browser'

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
  /** Where the file came from (kit, worker, local, global, builtin). */
  source?: {
    kind: 'kit' | 'worker' | 'local' | 'global' | 'builtin'
    kit?: string
    worker?: string
    version?: string
    modified?: boolean
  }
}

/** Short origin label for the list row: `kit acme/team · edited`,
 * `worker kanban`, `local`, `global` or `built-in`. */
export function originLabel(a: AgentRow): string | undefined {
  const s = a.source
  if (a.builtin || s?.kind === 'builtin') return 'built-in'
  const edited = s?.modified ? ' · edited' : ''
  switch (s?.kind) {
    case 'kit':
      return `kit ${s.kit ?? ''}${edited}`
    case 'worker':
      return `worker ${s.worker ?? ''}${edited}`
    case 'global':
      return 'global'
    case 'local':
      return 'local'
    default:
      return undefined
  }
}

/** The list's fine print: where the profile came from, then when it changed. */
export function agentOrigin(a: AgentRow): string {
  if (a.builtin) return 'Built-in · edits save a local override'
  const s = a.source
  const edited = s?.modified ? ' · edited' : ''
  const when = ago(a.modified_at)
  switch (s?.kind) {
    case 'kit':
      return `kit ${s.kit ?? ''}${edited} · ${when}`
    case 'worker':
      return `worker ${s.worker ?? ''}${edited} · ${when}`
    case 'global':
      return `global · ${when}`
    case 'local':
      return `local · ${when}`
    default:
      return when
  }
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
      fine: `${formatBytes(s.bytes)} · ${ago(s.modified_at)}`,
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
  extraManagedKeys: [
    'logo',
    'skills',
    'functions',
    'model',
    'reasoning_effort',
    'icon',
    'color',
    'extends',
    'hidden',
    'composer_placeholder',
  ],
  customForm: (ctx) => <AgentForm {...ctx} />,
  customLoading: () => <AgentFormSkeleton />,
  customFormOwnsContent: true,
  customFormOwnsWorkspaceHeader: true,
  prominentListItems: true,
  onChangeType: 'directory::agents::on-change',
  emptyTitle: 'Select an agent profile',
  emptyBody:
    'Choose an agent profile from the sidebar to edit its identity, default model, system prompt, skills, and preloaded functions.',
  async list(host) {
    const out = await host.iii.trigger<{ agents: AgentRow[] }>('directory::agents::list')
    return (out.agents ?? []).map((a) => ({
      key: a.id,
      // The row glyph is the SAME token glyph the avatar picker and the
      // console session tree render — one identity, one pictogram.
      icon: <TokenIcon token={a.icon || 'agent'} />,
      iconTone: a.color ?? 'neutral',
      title: a.name,
      description: a.description,
      fine: agentOrigin(a),
      origin: originLabel(a),
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

type Collection = 'skills' | 'agents' | 'kits'
type FileCollection = Exclude<Collection, 'kits'>

interface PendingCollectionAction {
  id: number
  collection: FileCollection
  key?: string
  action?: 'create'
}

export const COLLECTIONS: { value: Collection; label: string }[] = [
  { value: 'skills', label: 'Skills' },
  { value: 'agents', label: 'Agent Profiles' },
  { value: 'kits', label: 'Kits' },
]

const FILE_COLLECTIONS: FileCollection[] = ['skills', 'agents']

const ADAPTERS: Record<FileCollection, BrowserAdapter> = {
  skills: skillsAdapter,
  agents: agentsAdapter,
}

/** The Kits tab glyph, with a count of what needs attention (pending
 * reviews plus available updates). */
function KitsTabIcon({ attention }: { attention: number }) {
  return (
    <span className="dir-ui-kits-tab">
      <Puzzle aria-hidden />
      {attention > 0 ? (
        <span className="dir-ui-kits-badge" aria-label={`${attention} need attention`}>
          {attention > 9 ? '9+' : attention}
        </span>
      ) : null}
    </span>
  )
}

/** Kits route from a `host.panels.open` context: `{ collection: 'kits',
 * plan_id? , kit? }` (the chat cards' "Review install" button). */
export function kitsRouteFromContext(context: { plan_id?: unknown; kit?: unknown }): KitsRoute {
  if (typeof context.plan_id === 'string' && context.plan_id) return { view: 'plan', planId: context.plan_id }
  if (typeof context.kit === 'string' && context.kit) return { view: 'kit', kit: context.kit }
  return { view: 'home' }
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
  const [kitsRoute, setKitsRoute] = useState<KitsRoute>({ view: 'home' })
  const [attention, setAttention] = useState(0)

  // The Kits badge follows pending reviews and available updates even
  // while another collection is showing.
  const refreshAttention = useCallback(() => {
    kitsApi(host)
      .list()
      .then((l) => setAttention(l.attention))
      .catch(() => undefined)
  }, [host])
  useEffect(refreshAttention, [refreshAttention])
  useKitsChange(host, 'badge', (change) => {
    if (change.op !== 'progress') refreshAttention()
  })

  // Panel context can open a palette entry, start a collection's creation
  // flow, or open a kit / kit plan. `panelContext.id` is monotonic, so a
  // repeated action still applies.
  const appliedContextRef = useRef(0)
  useEffect(() => {
    if (!panelContext || panelContext.id === appliedContextRef.current) return
    appliedContextRef.current = panelContext.id
    const context = panelContext.context as {
      collection?: string
      key?: string
      action?: string
      plan_id?: string
      kit?: string
    } | null
    const collectionValue = context?.collection
    if (
      !context ||
      typeof collectionValue !== 'string' ||
      !COLLECTIONS.some((c) => c.value === collectionValue)
    ) {
      return
    }
    if (collectionValue === 'kits') {
      setCollection('kits')
      setKitsRoute(kitsRouteFromContext(context))
      return
    }
    const nextCollection = collectionValue as FileCollection
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

  // Kits screens open a profile or skill in its own collection.
  const openEntry = useCallback((target: FileCollection, key: string) => {
    setCollection(target)
    setPendingOpen({ id: Date.now(), collection: target, key })
  }, [])

  // Skills and Agent Profiles keep the control's inferred glyphs; Kits
  // brings its own, with the attention count.
  const options = COLLECTIONS.map((c) =>
    c.value === 'kits'
      ? {
          value: c.value,
          label: attention > 0 ? `Kits · ${attention} need attention` : c.label,
          icon: <KitsTabIcon attention={attention} />,
        }
      : c,
  )
  const switcher = (
    <SegmentedControl<Collection>
      value={collection}
      onChange={setCollection}
      options={options}
      iconOnly
      className="dir-ui-collection-tabs"
      aria-label="Browse skills, agent profiles or kits"
    />
  )

  return (
    <PageShell className="dir-ui-shell">
      <PageHeader
        icon={<FileText />}
        title="Directory"
        description="Filesystem-backed skills, agent profiles and kits"
        onClose={onRequestClose}
      />
      {FILE_COLLECTIONS.map((c) => (
        <div key={c} className="dir-ui-shell-body" hidden={collection !== c}>
          <CollectionBrowser
            host={host}
            adapter={ADAPTERS[c]}
            nav={switcher}
            panelSide={panelSide}
            storageKey={`iii-directory-ui:${tabId || 'page'}:${c}`}
            commands={commands}
            active={collection === c}
            pendingOpen={pendingOpen && pendingOpen.collection === c ? pendingOpen : null}
          />
        </div>
      ))}
      <div className="dir-ui-shell-body" hidden={collection !== 'kits'}>
        <KitsView
          host={host}
          nav={switcher}
          panelSide={panelSide}
          route={kitsRoute}
          onRoute={setKitsRoute}
          onOpenEntry={openEntry}
          active={collection === 'kits'}
        />
      </div>
    </PageShell>
  )
}
