import type { ExtensionIii } from '@iii-dev/console-ui'
import { type ReactNode, useEffect, useState, useCallback } from 'react'

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

/** Per-tab live subscriptions: `stories:changed` and `stories:build`. */
export function subscribe(iii: ExtensionIii, onChanged: () => void, onBuild: (event: BuildEvent) => void): () => void {
  const changedId = `iii::stories-ui::changed::${iii.browserId}`
  const buildId = `iii::stories-ui::build::${iii.browserId}`
  const offs = [
    iii.on(changedId, () => onChanged()),
    iii.registerTrigger({ type: 'stories:changed', function_id: changedId, config: {} }),
    iii.on<BuildEvent>(buildId, (event) => onBuild(event)),
    iii.registerTrigger({ type: 'stories:build', function_id: buildId, config: {} }),
  ]
  return () => {
    for (const off of offs) off()
  }
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

export function changeTone(kind: ChangeKind | undefined): 'neutral' | 'accent' | 'success' | 'warning' | 'danger' {
  switch (kind) {
    case 'direct':
      return 'warning'
    case 'indirect':
      return 'accent'
    case 'new':
      return 'success'
    case 'removed':
      return 'danger'
    default:
      return 'neutral'
  }
}

export const shortSha = (value?: string | null) => (value ? value.slice(0, 10) : '')

export function lineLabel(line: LineInfo): string {
  if (line.kind === 'worktree') return line.dirty ? 'working tree (dirty)' : 'working tree'
  if (line.kind === 'ref') return `${line.label} · ${shortSha(line.sha)}`
  return line.label
}

export type WidthTier = 'narrow' | 'medium' | 'wide'

/** Coarse container width: `narrow` stacks sidebar and main, `wide` earns
    the props inspector its own column. Only tier changes re-render. */
export function useContainerTier(narrowBelow: number, wideFrom: number): [(node: HTMLElement | null) => void, WidthTier | null] {
  const [node, setNode] = useState<HTMLElement | null>(null)
  const [tier, setTier] = useState<WidthTier | null>(null)
  useEffect(() => {
    if (!node) return
    const measure = () => {
      const width = node.getBoundingClientRect().width
      if (width <= 0) return
      const next: WidthTier = width < narrowBelow ? 'narrow' : width >= wideFrom ? 'wide' : 'medium'
      setTier((current) => (current === next ? current : next))
    }
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(node)
    return () => observer.disconnect()
  }, [node, narrowBelow, wideFrom])
  return [setNode, tier]
}

export function usePaneState<T>(key: string, initial: T): [T, (next: T) => void] {
  const [value, setValue] = useState<T>(() => {
    try {
      const raw = window.localStorage.getItem(key)
      return raw === null ? initial : (JSON.parse(raw) as T)
    } catch {
      return initial
    }
  })
  // Stable identity: callers list the updater in effect dependencies, and a
  // fresh closure per render would re-run those effects forever.
  const update = useCallback(
    (next: T) => {
      setValue(next)
      try {
        window.localStorage.setItem(key, JSON.stringify(next))
      } catch {
        undefined
      }
    },
    [key],
  )
  return [value, update]
}

export function errorMessage(error: unknown): string {
  if (!error) return 'Something went wrong.'
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  const record = error as Record<string, unknown>
  if (typeof record.message === 'string') return record.message
  return JSON.stringify(error)
}

export const isBuildingError = (message: string) => /still building|is building/i.test(message)

type IconProps = { className?: string }

function Glyph({ className, children }: IconProps & { children: ReactNode }) {
  return (
    <svg
      aria-hidden="true"
      className={className}
      fill="none"
      height="16"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="2"
      viewBox="0 0 24 24"
      width="16"
    >
      {children}
    </svg>
  )
}

export function StoriesIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" />
      <path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2Z" />
    </Glyph>
  )
}

export function FolderIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" />
    </Glyph>
  )
}

export function FileIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M14 3v5h5M6 3h8l5 5v13H6Z" />
    </Glyph>
  )
}

export function ComponentIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M12 3 4 7.5v9L12 21l8-4.5v-9Z" />
      <path d="M4 7.5 12 12l8-4.5M12 12v9" />
    </Glyph>
  )
}

export function StateIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <circle cx="12" cy="12" r="3" />
    </Glyph>
  )
}

export function ChevronRightIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="m9 18 6-6-6-6" />
    </Glyph>
  )
}

export function RefreshIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
      <path d="M21 3v5h-5" />
    </Glyph>
  )
}

export function HammerIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="m15 12-8.5 8.5a2.1 2.1 0 0 1-3-3L12 9" />
      <path d="M17.6 15 21 11.6 12.4 3 9 6.4Z" />
    </Glyph>
  )
}

export function AlertIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M12 9v4M12 17h.01" />
      <path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z" />
    </Glyph>
  )
}

export function PlusIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M5 12h14M12 5v14" />
    </Glyph>
  )
}

export function TrashIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6" />
    </Glyph>
  )
}

export function FilterIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M3 5h18l-7 8v6l-4-2v-4Z" />
    </Glyph>
  )
}

export function UndoIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M3 7v6h6" />
      <path d="M3 13a9 9 0 1 0 3-6.7L3 9" />
    </Glyph>
  )
}

export function XIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M18 6 6 18M6 6l12 12" />
    </Glyph>
  )
}
