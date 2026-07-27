/**
 * The directory page (#/ext/directory): browse every filesystem-backed
 * skill and prompt, open one in the shared CodeEditor/MarkdownPreview
 * pair, and save through the worker's update functions.
 */

import {
  type Host,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@iii-dev/console-ui'
import { formatBytes, formatRelativeTime } from '../lib/format'
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
  onChangeType: 'directory::skills::on-change',
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
      { id, raw: true },
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
}

const promptsAdapter: BrowserAdapter = {
  noun: 'prompt',
  onChangeType: 'directory::prompts::on-change',
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
      { name, raw: true },
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
}

export function DirectoryPage({ host }: { host: Host }) {
  return (
    <div className="dir-ui-page">
      <div className="dir-ui-page-head">
        <span className="dir-ui-page-title">directory</span>
        <span className="dir-ui-page-sub">
          filesystem-backed skills &amp; prompts — edit the markdown, save
          writes through directory::*::update
        </span>
      </div>
      <Tabs defaultValue="skills">
        <TabsList>
          <TabsTrigger value="skills">skills</TabsTrigger>
          <TabsTrigger value="prompts">prompts</TabsTrigger>
        </TabsList>
        <TabsContent value="skills">
          <CollectionBrowser host={host} adapter={skillsAdapter} />
        </TabsContent>
        <TabsContent value="prompts">
          <CollectionBrowser host={host} adapter={promptsAdapter} />
        </TabsContent>
      </Tabs>
    </div>
  )
}
