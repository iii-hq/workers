import type { ExtensionIii } from '@iii-dev/console-ui'

export const PAGE_ID = 'stories-explorer'
export const CONFIGURATION_ID = 'stories'

export type ChangeKind = 'unchanged' | 'direct' | 'indirect' | 'new' | 'removed'
export type LineInfo = {
  key: string
  kind: 'worktree' | 'prev' | 'ref' | 'turn'
  label: string
  sha?: string
  dirty?: boolean
  built_at: string
}
export type ChangedFile = { path: string; hop: number; status: string }
export type StateChange = { id: string; name: string; status: string }
export type ChangeInfo = { kind: ChangeKind; files: ChangedFile[]; states: StateChange[] }
export type StateSummary = { id: string; name: string; tags: string[] }
export type ComponentSummary = {
  id: string
  project: string
  title: string
  group: string
  path: string
  tags: string[]
  component?: string
  version: string
  preview_url?: string
  states: StateSummary[]
  change?: ChangeInfo
  error?: string
}
export type FsNode = {
  name: string
  path: string
  kind: 'dir' | 'file'
  git?: string
  change?: ChangeKind
  components?: ComponentSummary[]
  children?: FsNode[]
}
export type ChangeSummary = { direct: number; indirect: number; new: number; removed: number; unchanged: number }
export type FsTree = {
  workspace: string
  line: LineInfo
  base?: LineInfo
  base_pending?: string
  summary?: ChangeSummary
  root: FsNode
}
export type Control = {
  name: string
  type: 'select' | 'boolean' | 'number' | 'text' | 'object' | 'element' | 'function' | 'readonly'
  options?: unknown[]
  default: unknown
  description?: string
}
export type StateDetail = StateSummary & {
  export_name: string
  args: Record<string, unknown>
  controls: Control[]
  has_render: boolean
}
export type ComponentDetail = Omit<ComponentSummary, 'states'> & {
  states: StateDetail[]
  inputs: { path: string; hop: number; sha256: string }[]
}
export type GetOutput = {
  line: LineInfo
  base?: LineInfo
  component: ComponentDetail
  preview_url?: string
  change?: ChangeInfo
}
export type Change = {
  id: string
  project: string
  title: string
  file: string
  kind: ChangeKind
  files: ChangedFile[]
  a_version?: string
  b_version?: string
  states: StateChange[]
}
export type CompareOutput = { a: LineInfo; b: LineInfo; summary: ChangeSummary; changes: Change[] }
export type Workspace = {
  name: string
  path: string
  base: string
  branch?: string
  head?: string
  dirty?: boolean
  components: number
  lines: LineInfo[]
}
export type BuildStatus = {
  build_id: string
  workspace: string
  line: string
  status: 'queued' | 'running' | 'done' | 'failed'
  message?: string
}
export type BuildEvent = BuildStatus & { at: string }

export type Selection = { project: string; id: string; state: string | null }

export const api = {
  workspaces: (iii: ExtensionIii) => iii.trigger<{ workspaces: Workspace[] }>('stories::workspaces', {}),
  tree: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<FsTree>('stories::tree::fs', payload, { timeoutMs: 150_000 }),
  get: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<GetOutput>('stories::components::get', payload, { timeoutMs: 150_000 }),
  compare: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<CompareOutput>('stories::compare', payload, { timeoutMs: 150_000 }),
  diffFile: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<{ path: string; identical: boolean; diff: string }>('stories::diff::file', payload, {
      timeoutMs: 150_000,
    }),
  build: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<BuildStatus>('stories::builds::create', payload),
}

export function b64url(value: unknown): string {
  const bytes = new TextEncoder().encode(JSON.stringify(value))
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

export function storyUrl(previewUrl: string, story: string, globals: Record<string, unknown>): string {
  const url = new URL(previewUrl.replace(/^\//, ''), document.baseURI)
  url.searchParams.set('story', story)
  url.searchParams.set('globals', b64url(globals))
  return url.toString()
}

const TONES = { direct: 'warning', indirect: 'accent', new: 'success', removed: 'danger' } as const

export const changeTone = (kind?: ChangeKind) => (kind && kind !== 'unchanged' ? TONES[kind] : 'neutral')

export const shortSha = (value?: string | null) => (value ? value.slice(0, 10) : '')

export function lineLabel(line: LineInfo): string {
  if (line.kind === 'worktree') return line.dirty ? 'working tree (dirty)' : 'working tree'
  if (line.kind === 'ref') return `${line.label} · ${shortSha(line.sha)}`
  return line.label
}

export const isBuildingError = (message: string) => /still building|is building/i.test(message)
