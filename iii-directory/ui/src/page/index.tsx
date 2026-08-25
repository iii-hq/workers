/**
 * The directory page (#/ext/directory): a full-height application shell —
 * slim product top bar, a navigation sidebar carrying the skills/prompts
 * switcher, and a document workspace that opens one entry in the shared
 * CodeEditor/MarkdownPreview pair, saving through the worker's update
 * functions.
 *
 * Both collections stay MOUNTED (the inactive one is display:none) so an
 * unsaved draft survives switching between skills and prompts.
 */

import {
  type Host,
  PageHeader,
  type PageRenderProps,
  PageShell,
  SegmentedControl,
} from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import { formatBytes, formatRelativeTime } from '../lib/format'
import { MarkdownFileIcon } from '../lib/widgets'
import { AgentForm } from './agent-fields'
import { TokenIcon } from './token-icons'
import { type BrowserAdapter, CollectionBrowser } from './browser'

interface SkillRow {
  id: string
  title: string
  description: string
  bytes: number
  modified_at: string
}

interface PromptRow {
  name: string
  description: string
  modified_at: string
}

interface AgentRow {
  id: string
  name: string
  description: string
  logo: string | null
  icon: string | null
  modified_at: string
}

interface SystemPromptPreview {
  parts: { kind: string; body: string }[]
}

const skillsAdapter: BrowserAdapter = {
  noun: 'skill',
  crumbRoot: 'skills',
  nameKeys: ['name', 'title'],
  defaultNameKey: 'name',
  modelInvocationOption: true,
  // Skill ids are slash-separated lowercase segments (ns/skill/…).
  namePattern: /^[a-z0-9_-]+(\/[a-z0-9_-]+)*$/,
  nameHint:
    'enter an id of lowercase slash-separated segments (a–z, 0–9, hyphens or underscores)',
  onChangeType: 'directory::skills::on-change',
  emptyTitle: 'Select a skill',
  emptyBody:
    'Choose a skill from the sidebar to view and edit its markdown. New skills arrive through the new-skill button, downloads (directory::skills::download), or direct edits to the skills folder.',
  async list(host) {
    const out = await host.iii.trigger<{ skills: SkillRow[] }>(
      'directory::skills::list',
      { include_description: true },
    )
    return (out.skills ?? []).map((s) => ({
      key: s.id,
      title: s.title,
      description: s.description,
      fine: `${formatBytes(s.bytes)} · ${formatRelativeTime(s.modified_at)}`,
    }))
  },
  async load(host, id) {
    const out = await host.iii.trigger<{ body: string; raw?: string | null }>(
      'directory::skills::get',
      {
        id,
        raw: true,
      },
    )
    // `raw` is the exact on-disk file; body (frontmatter-stripped) is the
    // fallback against a not-yet-updated worker.
    return out.raw ?? out.body
  },
  async save(host, id, content) {
    const out = await host.iii.trigger<{ id: string }>(
      'directory::skills::update',
      { id, content },
    )
    return out.id ?? id
  },
  async create(host, id, content) {
    const out = await host.iii.trigger<{ id: string }>(
      'directory::skills::create',
      { id, content },
    )
    return out.id ?? id
  },
  async remove(host, id) {
    await host.iii.trigger('directory::skills::delete', { id })
  },
}

const promptsAdapter: BrowserAdapter = {
  noun: 'prompt',
  crumbRoot: 'prompts',
  slugName: true,
  descriptionRequired: true,
  onChangeType: 'directory::prompts::on-change',
  emptyTitle: 'Select a prompt',
  emptyBody:
    'Choose a prompt from the sidebar to view and edit its markdown. Prompts are filesystem-backed — files added to the prompts folders appear here automatically.',
  async list(host) {
    const out = await host.iii.trigger<{ prompts: PromptRow[] }>(
      'directory::prompts::list',
    )
    return (out.prompts ?? []).map((p) => ({
      key: p.name,
      title: '',
      description: p.description,
      fine: formatRelativeTime(p.modified_at),
    }))
  },
  async load(host, name) {
    const out = await host.iii.trigger<{ body: string; raw?: string | null }>(
      'directory::prompts::get',
      {
        name,
        raw: true,
      },
    )
    return out.raw ?? out.body
  },
  async save(host, name, content) {
    // The effective name after the write follows a frontmatter rename.
    const out = await host.iii.trigger<{ name: string }>(
      'directory::prompts::update',
      { name, content },
    )
    return out.name ?? name
  },
  async create(host, name, content) {
    const out = await host.iii.trigger<{ name: string }>(
      'directory::prompts::create',
      { name, content },
    )
    return out.name ?? name
  },
  async remove(host, name) {
    await host.iii.trigger('directory::prompts::delete', { name })
  },
}

export const HARNESS_DEFAULT_SYSTEM_PROMPT_KEY = 'harness/default'

export const systemPromptsAdapter: BrowserAdapter = {
  noun: 'system prompt',
  crumbRoot: 'system-prompts',
  slugName: true,
  descriptionRequired: true,
  onChangeType: 'directory::system-prompts::on-change',
  emptyTitle: 'Select a system prompt',
  emptyBody:
    'Choose a system prompt from the sidebar to view and edit its markdown. These are what the chat picker offers as an identity prompt — filesystem-backed, so files added to the system-prompts folders appear here automatically.',
  async list(host) {
    const out = await host.iii.trigger<{ prompts: PromptRow[] }>(
      'directory::system-prompts::list',
    )
    return [
      {
        key: HARNESS_DEFAULT_SYSTEM_PROMPT_KEY,
        title: 'default',
        description: 'Harness default system prompt',
        fine: 'Read only',
        readOnly: true,
      },
      ...(out.prompts ?? []).map((p) => ({
        key: p.name,
        title: '',
        description: p.description,
        fine: formatRelativeTime(p.modified_at),
      })),
    ]
  },
  async load(host, name) {
    if (name === HARNESS_DEFAULT_SYSTEM_PROMPT_KEY) {
      const out = await host.iii.trigger<SystemPromptPreview>(
        'harness::system-prompt::get',
        {
          session_id: `iii-directory:${host.iii.browserId}`,
          default_only: true,
        },
      )
      const builtIn = out.parts.find((part) => part.kind === 'built_in')
      if (!builtIn) throw new Error('Harness default system prompt is unavailable')
      return builtIn.body
    }
    const out = await host.iii.trigger<{ body: string; raw?: string | null }>(
      'directory::system-prompts::get',
      { name, raw: true },
    )
    return out.raw ?? out.body
  },
  async save(host, name, content) {
    // The effective name after the write follows a frontmatter rename.
    const out = await host.iii.trigger<{ name: string }>(
      'directory::system-prompts::update',
      { name, content },
    )
    return out.name ?? name
  },
  async create(host, name, content) {
    const out = await host.iii.trigger<{ name: string }>(
      'directory::system-prompts::create',
      { name, content },
    )
    return out.name ?? name
  },
  async remove(host, name) {
    await host.iii.trigger('directory::system-prompts::delete', { name })
  },
}

export const agentsAdapter: BrowserAdapter = {
  noun: 'agent',
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
  extraManagedKeys: ['logo', 'skills', 'delegates_to', 'leaf', 'model', 'icon'],
  customForm: (ctx) => <AgentForm {...ctx} />,
  sourceLabel: 'Instructions · system prompt',
  onChangeType: 'directory::agents::on-change',
  emptyTitle: 'Select an agent',
  emptyBody:
    'Choose an agent from the sidebar to view and edit its profile. The content below the fields is the system prompt the session runs as; name, logo and skill selection live in the fields above it.',
  async list(host) {
    const out = await host.iii.trigger<{ agents: AgentRow[] }>(
      'directory::agents::list',
    )
    return (out.agents ?? []).map((a) => ({
      key: a.id,
      // The row glyph is the SAME token glyph the avatar picker and the
      // console session tree render — one identity, one pictogram.
      icon: a.icon ? <TokenIcon token={a.icon} size={14} /> : undefined,
      title: a.name,
      description: a.description,
      fine: formatRelativeTime(a.modified_at),
    }))
  },
  async load(host, id) {
    const out = await host.iii.trigger<{
      system_prompt: string
      raw?: string | null
    }>('directory::agents::get', { id, raw: true })
    return out.raw ?? out.system_prompt
  },
  async save(host, id, content) {
    const out = await host.iii.trigger<{ id: string }>(
      'directory::agents::update',
      { id, content },
    )
    return out.id ?? id
  },
  async create(host, id, content) {
    const out = await host.iii.trigger<{ id: string }>(
      'directory::agents::create',
      { id, content },
    )
    return out.id ?? id
  },
  async remove(host, id) {
    await host.iii.trigger('directory::agents::delete', { id })
  },
}

type Collection = 'skills' | 'prompts' | 'system-prompts' | 'agents'

const COLLECTIONS: { value: Collection; label: string }[] = [
  { value: 'skills', label: 'Skills' },
  { value: 'prompts', label: 'Prompts' },
  { value: 'system-prompts', label: 'System Prompts' },
  { value: 'agents', label: 'Agents' },
]

const ADAPTERS: Record<Collection, BrowserAdapter> = {
  skills: skillsAdapter,
  prompts: promptsAdapter,
  'system-prompts': systemPromptsAdapter,
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
  const [pendingOpen, setPendingOpen] = useState<{
    id: number
    collection: Collection
    key: string
  } | null>(null)

  // A palette row (see palette.ts) selects a collection + entry here once
  // the page is open. `panelContext.id` is monotonic, so a repeated
  // identical click still re-applies.
  const appliedContextRef = useRef(0)
  useEffect(() => {
    if (!panelContext || panelContext.id === appliedContextRef.current) return
    appliedContextRef.current = panelContext.id
    const context = panelContext.context as {
      collection?: string
      key?: string
    } | null
    const collectionValue = context?.collection
    if (
      !context ||
      typeof collectionValue !== 'string' ||
      typeof context.key !== 'string' ||
      !COLLECTIONS.some((c) => c.value === collectionValue)
    ) {
      return
    }
    const nextCollection = collectionValue as Collection
    setCollection(nextCollection)
    setPendingOpen({
      id: panelContext.id,
      collection: nextCollection,
      key: context.key,
    })
  }, [panelContext])

  const switcher = (
    <SegmentedControl<Collection>
      value={collection}
      onChange={setCollection}
      options={COLLECTIONS}
      className="dir-ui-collection-tabs"
      aria-label="Browse skills, prompts, system prompts or agents"
    />
  )

  return (
    <PageShell className="dir-ui-shell">
      <PageHeader
        icon={<MarkdownFileIcon />}
        title="Directory"
        description="Filesystem-backed skills, prompts, system prompts and agents"
        onClose={onRequestClose}
      />
      {COLLECTIONS.map((c) => (
        <div
          key={c.value}
          className="dir-ui-shell-body"
          hidden={collection !== c.value}
        >
          <CollectionBrowser
            host={host}
            adapter={ADAPTERS[c.value]}
            nav={switcher}
            panelSide={panelSide}
            storageKey={`iii-directory-ui:${tabId || 'page'}:${c.value}`}
            commands={commands}
            active={collection === c.value}
            pendingOpen={
              pendingOpen && pendingOpen.collection === c.value
                ? pendingOpen
                : null
            }
          />
        </div>
      ))}
    </PageShell>
  )
}
