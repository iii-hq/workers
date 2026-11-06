/**
 * The directory page (page `directory`): everything that shapes what an
 * agent knows and does, in one place — agent profiles, the skills they
 * read, and kits that install both together.
 *
 * One layout for every section: a library column (section tabs, search,
 * grouped entries) beside the open document. All three sections stay
 * MOUNTED (the inactive ones are hidden) so an unsaved draft survives
 * switching between them.
 */

import {
  type Host,
  PageHeader,
  type PageRenderProps,
  PageShell,
  ShadcnTabs,
  ShadcnTabsList,
  ShadcnTabsTrigger,
} from '@iii-dev/console-ui'
import { formatBytes } from '@iii-dev/console-ui/format'
import { usePaneState } from '@iii-dev/console-ui/hooks'
import { Library as LibraryIcon } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { kitsApi, useKitsChange } from '../kits/api'
import { type KitsRoute, KitsView } from '../kits/KitsView'
import { ago } from '../lib/format'
import { AgentForm, AgentFormSkeleton, TokenIcon } from './agent-fields'
import { type BrowserAdapter, CollectionBrowser } from './browser'

interface SkillRow {
  id: string
  title: string
  type?: string | null
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

/** Short origin label: `kit acme/team · edited`, `worker kanban`, `local`,
 * `global` or `built-in`. */
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

/** The document's fine print: where the profile came from, then when it changed. */
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

/** Library group for a profile: yours first, then what kits and workers
 * brought, then what ships built in. */
export function agentGroup(a: AgentRow): { group: string; groupLabel: string } {
  const kind = a.builtin ? 'builtin' : a.source?.kind
  switch (kind) {
    case 'kit':
      return { group: 'kit', groupLabel: 'From kits' }
    case 'worker':
      return { group: 'worker', groupLabel: 'From workers' }
    case 'builtin':
      return { group: 'builtin', groupLabel: 'Built-in' }
    default:
      return { group: 'yours', groupLabel: 'Yours' }
  }
}

/** Library group for a skill: its first id segment (`kanban/tickets/…` →
 * kanban); single-segment ids are standalone. */
export function skillGroup(id: string): string {
  const slash = id.indexOf('/')
  return slash === -1 ? '' : id.slice(0, slash)
}

const skillsAdapter: BrowserAdapter = {
  noun: 'skill',
  nounPlural: 'Skills',
  crumbRoot: 'skills',
  nameKeys: ['name', 'title'],
  defaultNameKey: 'name',
  modelInvocationOption: true,
  // Skill ids are slash-separated lowercase segments (ns/skill/…).
  namePattern: /^[a-z0-9_-]+(\/[a-z0-9_-]+)*$/,
  nameHint: 'Use lowercase words separated by slashes, like team/release-notes (a–z, 0–9, - and _).',
  nameHelp: 'Lowercase words separated by slashes, like team/release-notes. The first word groups it.',
  filePath: (id) => `skills/${id}.md`,
  // An untouched new skill is not unsaved work: leaving it never asks.
  newTemplateStartsClean: true,
  fallbackGroupLabel: 'Standalone',
  onChangeFunction: 'directory::skills::on-change',
  emptyTitle: 'Skills teach agents how to work',
  emptyBody:
    'A skill is a markdown file an agent reads when a task calls for it. Pick one to read or edit it, or write a new one.',
  emptyListTitle: 'No skills yet',
  emptyListBody: 'Write one here, or drop markdown files into the skills folder.',
  async list(host) {
    const out = await host.iii.call<{ skills: SkillRow[] }>('directory::skills::list', { include_description: true })
    return (out.skills ?? []).map((s) => ({
      key: s.id,
      title: s.title,
      description: s.description,
      fine: `${formatBytes(s.bytes)} · updated ${ago(s.modified_at)}`,
      group: skillGroup(s.id),
    }))
  },
  async load(host, id) {
    const out = await host.iii.call<{ body: string; raw?: string | null }>('directory::skills::get', {
      id,
      raw: true,
    })
    // `raw` is the exact on-disk file; body (frontmatter-stripped) is the
    // fallback against a not-yet-updated worker.
    return out.raw ?? out.body
  },
  async save(host, id, content) {
    const out = await host.iii.call<{ id: string }>('directory::skills::update', { id, content })
    return out.id ?? id
  },
  async create(host, id, content) {
    const out = await host.iii.call<{ id: string }>('directory::skills::create', { id, content })
    return out.id ?? id
  },
  async remove(host, id) {
    await host.iii.call('directory::skills::delete', { id })
  },
}

export const agentsAdapter: BrowserAdapter = {
  noun: 'agent',
  nounPlural: 'Agents',
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
  groupOrder: ['yours', 'kit', 'worker', 'builtin'],
  onChangeFunction: 'directory::agents::on-change',
  emptyTitle: 'Agents give a chat its identity',
  emptyBody:
    'An agent sets the instructions, the default model, and the skills and functions a chat starts with. Pick one to edit it, or create your own.',
  emptyListTitle: 'No agents yet',
  emptyListBody: 'Create one to give a chat its own instructions, model and skills.',
  duplicate: true,
  async list(host) {
    const out = await host.iii.call<{ agents: AgentRow[] }>('directory::agents::list')
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
      ...agentGroup(a),
      ...(a.builtin ? { noDelete: true } : {}),
    }))
  },
  async load(host, id) {
    // `raw` is the profile's OWN file; `system_prompt` would be the
    // inheritance-resolved prompt, never what the editor should save.
    const out = await host.iii.call<{
      system_prompt: string
      raw?: string | null
    }>('directory::agents::get', { id, raw: true })
    return out.raw ?? out.system_prompt
  },
  async save(host, id, content) {
    const out = await host.iii.call<{ id: string }>('directory::agents::update', { id, content })
    return out.id ?? id
  },
  async create(host, id, content) {
    const out = await host.iii.call<{ id: string }>('directory::agents::create', { id, content })
    return out.id ?? id
  },
  async remove(host, id) {
    await host.iii.call('directory::agents::delete', { id })
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

/** The sections, in tab order: who the agent is, what it knows, and
 * bundles of both. */
export const COLLECTIONS: { value: Collection; label: string }[] = [
  { value: 'agents', label: 'Agents' },
  { value: 'skills', label: 'Skills' },
  { value: 'kits', label: 'Kits' },
]

const FILE_COLLECTIONS: FileCollection[] = ['agents', 'skills']

const ADAPTERS: Record<FileCollection, BrowserAdapter> = {
  skills: skillsAdapter,
  agents: agentsAdapter,
}

/** Kits route from a `host.panels.open` context: `{ collection: 'kits',
 * plan_id? , kit? }` (the chat cards' "Review install" button). */
export function kitsRouteFromContext(context: { plan_id?: unknown; kit?: unknown }): KitsRoute {
  if (typeof context.plan_id === 'string' && context.plan_id) return { view: 'plan', planId: context.plan_id }
  if (typeof context.kit === 'string' && context.kit) return { view: 'kit', kit: context.kit }
  return { view: 'home' }
}

/** The section switcher at the top of the library column: line tabs (the
 * same pattern as the agent page), counts trailing, and an attention count
 * on Kits only while a plan waits for review. ←/→ come from the tabs. */
function SectionTabs({
  value,
  onChange,
  counts,
  attention,
  focusTick,
}: {
  value: Collection
  onChange: (next: Collection) => void
  counts: Partial<Record<Collection, number>>
  attention: number
  /** Bumped when the keyboard switched sections: each section renders its
   * own copy of these tabs, so focus must follow to the copy now showing
   * (the one it was on just went hidden). */
  focusTick: number
}) {
  const wrapRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    const wrap = wrapRef.current
    if (!focusTick || !wrap || wrap.closest('[hidden]')) return
    wrap.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')?.focus()
  }, [focusTick])
  return (
    <div ref={wrapRef} className="dir-sections-wrap">
      <ShadcnTabs value={value} onValueChange={(next) => onChange(next as Collection)} className="dir-sections-root">
        <ShadcnTabsList variant="line" activateOnFocus className="dir-sections" aria-label="Directory sections">
          {COLLECTIONS.map((c) => {
            const count = counts[c.value]
            return (
              <ShadcnTabsTrigger key={c.value} value={c.value} className="dir-section">
                {c.label}
                {c.value === 'kits' && attention > 0 ? (
                  <span className="dir-section-attention">
                    {attention > 9 ? '9+' : attention}
                    <span className="dir-sr-only"> waiting for review</span>
                  </span>
                ) : count !== undefined ? (
                  <span className="dir-section-count">{count}</span>
                ) : null}
              </ShadcnTabsTrigger>
            )
          })}
        </ShadcnTabsList>
      </ShadcnTabs>
    </div>
  )
}

/** The `directory` page: section tabs over three always-mounted sections (agents, skills, kits), routed by panel context. */
export function DirectoryPage({
  host,
  panelSide = 'left',
  tabId = '',
  onRequestClose,
  panelContext,
  commands,
}: { host: Host } & Partial<PageRenderProps>) {
  const [storedCollection, setCollection] = usePaneState<Collection>('iii-directory:section', 'agents')
  const collection: Collection = COLLECTIONS.some((c) => c.value === storedCollection) ? storedCollection : 'agents'
  const [pendingOpen, setPendingOpen] = useState<PendingCollectionAction | null>(null)
  const [kitsRoute, setKitsRoute] = useState<KitsRoute>({ view: 'home' })
  const [attention, setAttention] = useState(0)
  const [counts, setCounts] = useState<Partial<Record<Collection, number>>>({})

  // The Kits tab follows pending reviews and available updates even while
  // another section is showing.
  const refreshAttention = useCallback(() => {
    kitsApi(host)
      .list()
      .then((l) => {
        setAttention(l.attention)
        setCounts((c) => ({ ...c, kits: l.kits.length }))
      })
      .catch(() => undefined)
  }, [host])
  useEffect(refreshAttention, [refreshAttention])
  useKitsChange(host, 'badge', (change) => {
    if (change.op !== 'progress') refreshAttention()
  })

  // Panel context can open a palette entry, start a section's creation
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
    if (!context || typeof collectionValue !== 'string' || !COLLECTIONS.some((c) => c.value === collectionValue)) {
      return
    }
    if (collectionValue === 'kits') {
      setCollection('kits')
      setKitsRoute(kitsRouteFromContext(context))
      return
    }
    const nextCollection = collectionValue as FileCollection
    setCollection(nextCollection)
    const opensEntry = typeof context.key === 'string'
    const startsCreate = context.action === 'create'
    if (!opensEntry && !startsCreate) return
    setPendingOpen({
      id: panelContext.id,
      collection: nextCollection,
      ...(opensEntry ? { key: context.key } : {}),
      ...(startsCreate ? { action: 'create' } : {}),
    })
  }, [panelContext, setCollection])

  // Kits screens open a profile or skill in its own section.
  const openEntry = useCallback(
    (target: FileCollection, key: string) => {
      setCollection(target)
      setPendingOpen({ id: Date.now(), collection: target, key })
    },
    [setCollection],
  )

  const [focusTick, setFocusTick] = useState(0)
  const changeSection = useCallback(
    (next: Collection) => {
      const fromTabs = (document.activeElement as HTMLElement | null)?.closest('.dir-sections')
      setCollection(next)
      if (fromTabs) setFocusTick((t) => t + 1)
    },
    [setCollection],
  )
  const switcher = (
    <SectionTabs
      value={collection}
      onChange={changeSection}
      counts={counts}
      attention={attention}
      focusTick={focusTick}
    />
  )

  return (
    <PageShell className="dir-shell">
      <PageHeader
        icon={<LibraryIcon />}
        title="Directory"
        description="Agents, skills and kits"
        onClose={onRequestClose}
      />
      {FILE_COLLECTIONS.map((c) => (
        <div key={c} className="dir-shell-body" hidden={collection !== c}>
          <CollectionBrowser
            host={host}
            adapter={ADAPTERS[c]}
            nav={switcher}
            panelSide={panelSide}
            storageKey={`iii-directory-ui:${tabId || 'page'}:${c}`}
            commands={commands}
            active={collection === c}
            pendingOpen={pendingOpen && pendingOpen.collection === c ? pendingOpen : null}
            onCount={(n) => setCounts((prev) => (prev[c] === n ? prev : { ...prev, [c]: n }))}
          />
        </div>
      ))}
      <div className="dir-shell-body" hidden={collection !== 'kits'}>
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
