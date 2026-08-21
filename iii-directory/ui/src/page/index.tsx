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
import { useState } from 'react'
import { formatBytes, formatRelativeTime } from '../lib/format'
import { MarkdownFileIcon } from '../lib/widgets'
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

interface SystemPromptPreview {
  parts: { kind: string; body: string }[]
}

const skillsAdapter: BrowserAdapter = {
  noun: 'skill',
  crumbRoot: 'skills',
  nameKeys: ['name', 'title'],
  defaultNameKey: 'name',
  // Skill ids are slash-separated lowercase segments (ns/skill/…).
  namePattern: /^[a-z0-9_-]+(\/[a-z0-9_-]+)*$/,
  nameHint: 'enter an id of lowercase slash-separated segments (a–z, 0–9, hyphens or underscores)',
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
    const out = await host.iii.trigger<{ id: string }>('directory::skills::create', { id, content })
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
        { session_id: `iii-directory:${host.iii.browserId}` },
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

type Collection = 'skills' | 'prompts' | 'system-prompts'

const COLLECTIONS: { value: Collection; label: string }[] = [
  { value: 'skills', label: 'Skills' },
  { value: 'prompts', label: 'Prompts' },
  { value: 'system-prompts', label: 'System Prompts' },
]

const ADAPTERS: Record<Collection, BrowserAdapter> = {
  skills: skillsAdapter,
  prompts: promptsAdapter,
  'system-prompts': systemPromptsAdapter,
}

export function DirectoryPage({
  host,
  panelSide = 'left',
  tabId = '',
  onRequestClose,
}: { host: Host } & Partial<PageRenderProps>) {
  const [collection, setCollection] = useState<Collection>('skills')

  const switcher = (
    <SegmentedControl<Collection>
      value={collection}
      onChange={setCollection}
      options={COLLECTIONS}
      className="dir-ui-collection-tabs"
      aria-label="Browse skills, prompts or system prompts"
    />
  )

  return (
    <PageShell className="dir-ui-shell">
      <PageHeader
        icon={<MarkdownFileIcon />}
        title="Directory"
        description="Filesystem-backed skills, prompts and system prompts"
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
          />
        </div>
      ))}
    </PageShell>
  )
}
