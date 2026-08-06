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

import { type Host, PageHeader, type PageRenderProps, PageShell } from '@iii-dev/console-ui'
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

const skillsAdapter: BrowserAdapter = {
  noun: 'skill',
  crumbRoot: 'skills',
  onChangeType: 'directory::skills::on-change',
  emptyTitle: 'Select a skill',
  emptyBody:
    'Choose a skill from the sidebar to view and edit its markdown. New skills arrive through downloads (directory::skills::download) or direct edits to the skills folder.',
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
}

const promptsAdapter: BrowserAdapter = {
  noun: 'prompt',
  crumbRoot: 'prompts',
  onChangeType: 'directory::prompts::on-change',
  emptyTitle: 'Select a prompt',
  emptyBody:
    'Choose a prompt from the sidebar to view and edit its markdown. Prompts are filesystem-backed — files added to the prompts folders appear here automatically.',
  async list(host) {
    const out = await host.iii.trigger<{ prompts: PromptRow[] }>('directory::prompts::list')
    return (out.prompts ?? []).map((p) => ({
      key: p.name,
      title: '',
      description: p.description,
      fine: formatRelativeTime(p.modified_at),
    }))
  },
  async load(host, name) {
    const out = await host.iii.trigger<{ body: string; raw?: string | null }>('directory::prompts::get', {
      name,
      raw: true,
    })
    return out.raw ?? out.body
  },
  async save(host, name, content) {
    // The effective name after the write follows a frontmatter rename.
    const out = await host.iii.trigger<{ name: string }>('directory::prompts::update', { name, content })
    return out.name ?? name
  },
}

type Collection = 'skills' | 'prompts'

const COLLECTIONS: { value: Collection; label: string }[] = [
  { value: 'skills', label: 'skills' },
  { value: 'prompts', label: 'prompts' },
]

export function DirectoryPage({
  host,
  panelSide = 'left',
  tabId = '',
  onRequestClose,
}: { host: Host } & Partial<PageRenderProps>) {
  const [collection, setCollection] = useState<Collection>('skills')

  const switcher = (
    // biome-ignore lint/a11y/useSemanticElements: segmented control of buttons; fieldset chrome (min-content sizing) breaks the sidebar column
    <div className="dir-ui-seg block" role="group" aria-label="browse skills or prompts">
      {COLLECTIONS.map((c) => (
        <button
          key={c.value}
          type="button"
          className={`dir-ui-seg-btn${collection === c.value ? ' active' : ''}`}
          aria-pressed={collection === c.value}
          onClick={() => setCollection(c.value)}
        >
          {c.label}
        </button>
      ))}
    </div>
  )

  return (
    <PageShell className="dir-ui-shell">
      <PageHeader
        icon={<MarkdownFileIcon />}
        title="directory"
        description="filesystem-backed skills & prompts"
        onClose={onRequestClose}
      />
      {COLLECTIONS.map((c) => (
        <div key={c.value} className="dir-ui-shell-body" hidden={collection !== c.value}>
          <CollectionBrowser
            host={host}
            adapter={c.value === 'skills' ? skillsAdapter : promptsAdapter}
            nav={switcher}
            panelSide={panelSide}
            storageKey={`iii-directory-ui:${tabId || 'page'}:${c.value}`}
          />
        </div>
      ))}
    </PageShell>
  )
}
