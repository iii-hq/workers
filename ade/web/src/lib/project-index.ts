import { getIiiClient } from '@/lib/iii-client'
import {
  WORKSPACE_LIST_FUNCTION_ID,
  WORKSPACE_ROOTS_FUNCTION_ID,
} from '@/lib/working-dir'

/**
 * The folders "Open project" can offer before anyone types a path: a
 * shallow crawl of the shell worker's workspace roots that notes which
 * folders are git repositories. A listing is a few milliseconds, so a few
 * hundred of them fill the index in well under a second; the result is
 * kept for the session and in localStorage, so the next open is instant
 * and refreshes quietly behind the list.
 */

export interface IndexedFolder {
  path: string
  name: string
  /**
   * The folder is a project root: it holds a `.git` folder, or the
   * folders a working copy grows (`.github`, `node_modules`, …). The
   * listing returns folders only, so a worktree's `.git` file is
   * invisible and the markers stand in for it.
   */
  git: boolean
}

export interface ProjectIndex {
  /** The user's home folder when a root looks like one; `~` expands to it. */
  home: string | null
  roots: string[]
  folders: IndexedFolder[]
}

interface Entry {
  name: string
  path: string
  kind: string
}

const STORAGE_KEY = 'iii-project-index-v3'
/** An index younger than this is used as is; older ones refresh behind. */
const FRESH_MS = 5 * 60_000
/** Levels below a root the crawl descends; a repository stops it early. */
const MAX_DEPTH = 4
const MAX_LISTINGS = 2_000
const CONCURRENCY = 12
const BUDGET_MS = 6_000

/** Folders that are never projects and are expensive to walk. */
const SKIP = new Set([
  'node_modules',
  'Library',
  'Applications',
  'Pictures',
  'Music',
  'Movies',
  'Photos',
  'Public',
  'target',
  'dist',
  'build',
  'out',
  'vendor',
  'venv',
  '__pycache__',
  'Pods',
  'DerivedData',
  'site-packages',
  'System',
  'Volumes',
  'private',
  'usr',
  'bin',
  'sbin',
  'etc',
  'var',
  'tmp',
  'opt',
  'cores',
  'dev',
])

/** Subfolders that only a project root has. */
const PROJECT_MARKERS = new Set([
  '.git',
  '.github',
  '.gitlab',
  '.vscode',
  '.idea',
  'node_modules',
  '.venv',
  '.next',
  '.cargo',
])

/** Names that usually hold projects: walked first, at any depth. */
const DEV_NAMES =
  /^(code|dev|developer|projects?|repos?|src|work|workspaces?|github|git|sites|conductor|playground|lab)$/i

/** Names that usually hold everything else: walked last. */
const LOW_NAMES =
  /^(downloads|documents|desktop|dropbox|google drive|onedrive|icloud drive)$/i

let memory: { index: ProjectIndex; at: number } | null = null
let inflight: Promise<ProjectIndex> | null = null

export function homeOf(roots: string[]): string | null {
  return roots.find((root) => /^\/(Users|home)\/[^/]+$/.test(root)) ?? null
}

/** `~/x` → `/Users/me/x` when the home folder is known. */
export function expandHome(path: string, home: string | null): string {
  if (!home) return path
  if (path === '~') return home
  return path.startsWith('~/') ? `${home}${path.slice(1)}` : path
}

/** `/Users/me/x` → `~/x`, for display. */
export function tildify(path: string, home: string | null): string {
  if (!home) return path
  if (path === home) return '~'
  return path.startsWith(`${home}/`) ? `~${path.slice(home.length)}` : path
}

async function list(dir: string): Promise<Entry[]> {
  const client = await getIiiClient()
  const res = await client.trigger<{ entries?: Entry[] }>(
    WORKSPACE_LIST_FUNCTION_ID,
    { path: dir, page_size: 400 },
  )
  return res?.entries ?? []
}

export interface FolderListing {
  /** Subfolders, hidden ones last. `git` is filled from the index. */
  folders: IndexedFolder[]
  /** The listed folder is itself a project root. */
  project: boolean
}

/** One folder's subfolders, for browsing and path completion. */
export async function listFolder(
  dir: string,
  known?: ReadonlyMap<string, IndexedFolder>,
): Promise<FolderListing> {
  const entries = await list(dir)
  const project = entries.some(
    (e) => e.kind === 'dir' && PROJECT_MARKERS.has(e.name),
  )
  const folders = entries
    .filter((e) => e.kind === 'dir' && e.name !== '.git')
    .map((e) => ({
      path: e.path,
      // A symlink arrives under its link name; keep that for browsing,
      // since it is the name the user sees in the folder.
      name: e.name,
      git: known?.get(e.path)?.git ?? false,
    }))
    .sort(
      (a, b) =>
        Number(a.name.startsWith('.')) - Number(b.name.startsWith('.')) ||
        a.name.localeCompare(b.name),
    )
  return { folders, project }
}

/** The subfolders of `dir`, for path completion; hidden ones last. */
export async function listFolders(dir: string): Promise<IndexedFolder[]> {
  return (await listFolder(dir)).folders
}

async function crawl(): Promise<ProjectIndex> {
  const client = await getIiiClient()
  const info = await client.trigger<{ roots?: string[] }>(
    WORKSPACE_ROOTS_FUNCTION_ID,
    {},
  )
  const roots = info?.roots ?? []
  const home = homeOf(roots)
  // Crawling `/` would mostly walk the system; the home folder and any
  // narrower roots are where projects live. `/` alone is crawled shallowly.
  const starts = roots.filter((root) => root !== '/')
  // A queue per priority: dev-looking folders first, then the rest by
  // depth, and Downloads-like trees last, so the budget is spent where
  // projects tend to be.
  const buckets = new Map<
    number,
    Array<{ path: string; depth: number; bias: number }>
  >()
  const push = (item: { path: string; depth: number; bias: number }) => {
    const key = item.depth + item.bias
    const bucket = buckets.get(key)
    if (bucket) bucket.push(item)
    else buckets.set(key, [item])
  }
  const take = () => {
    let best: number | null = null
    for (const [key, bucket] of buckets) {
      if (bucket.length && (best === null || key < best)) best = key
    }
    return best === null ? undefined : buckets.get(best)?.shift()
  }
  for (const path of starts.length ? starts : roots) {
    push({ path, depth: 0, bias: 0 })
  }
  const seen = new Set(starts.length ? starts : roots)
  const folders = new Map<string, IndexedFolder>()
  const deadline = Date.now() + BUDGET_MS
  let listings = 0
  let pending = 0

  const worker = async () => {
    while (listings < MAX_LISTINGS && Date.now() < deadline) {
      const next = take()
      if (!next) {
        // Another worker's listing may still queue more folders.
        if (pending === 0) break
        await new Promise((resolve) => setTimeout(resolve, 5))
        continue
      }
      listings += 1
      pending += 1
      let entries: Entry[]
      try {
        entries = await list(next.path)
      } catch {
        pending -= 1
        continue
      }
      pending -= 1
      // A root is never a project itself: a home folder holds `.cargo`
      // and `.vscode` too.
      const isProject =
        next.depth > 0 &&
        entries.some((e) => e.kind === 'dir' && PROJECT_MARKERS.has(e.name))
      const known = folders.get(next.path)
      if (known) known.git = isProject
      // A project's own subfolders are not separate projects.
      if (isProject || next.depth >= MAX_DEPTH) continue
      for (const e of entries) {
        if (e.kind !== 'dir' || e.name.startsWith('.') || SKIP.has(e.name)) {
          continue
        }
        if (seen.has(e.path)) continue
        seen.add(e.path)
        // A symlinked entry arrives under its link name with the target's
        // path; name it after the path so search finds the real folder.
        const name = e.path.slice(e.path.lastIndexOf('/') + 1) || e.name
        folders.set(e.path, { path: e.path, name, git: false })
        const bias = DEV_NAMES.test(e.name)
          ? Math.min(next.bias, 0) - 2
          : LOW_NAMES.test(e.name)
            ? 3
            : next.bias
        push({ path: e.path, depth: next.depth + 1, bias })
      }
    }
  }
  await Promise.all(Array.from({ length: CONCURRENCY }, worker))
  return { home, roots, folders: [...folders.values()] }
}

function readStored(): { index: ProjectIndex; at: number } | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw) as { index: ProjectIndex; at: number }
    return Array.isArray(parsed?.index?.folders) ? parsed : null
  } catch {
    return null
  }
}

function store(index: ProjectIndex): void {
  memory = { index, at: Date.now() }
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(memory))
  } catch {
    /* a full or disabled storage only costs the instant first paint */
  }
}

/** Whatever index is already at hand, without waiting for a crawl. */
export function cachedProjectIndex(): ProjectIndex | null {
  memory ??= readStored()
  return memory?.index ?? null
}

/**
 * The index, crawling when there is none or it has gone stale. Concurrent
 * callers share one crawl.
 */
export function loadProjectIndex(force = false): Promise<ProjectIndex> {
  memory ??= readStored()
  if (!force && memory && Date.now() - memory.at < FRESH_MS) {
    return Promise.resolve(memory.index)
  }
  inflight ??= crawl()
    .then((index) => {
      store(index)
      return index
    })
    .finally(() => {
      inflight = null
    })
  return inflight
}

export interface FolderMatch {
  folder: IndexedFolder
  score: number
  /** Matched character ranges in `folder.name`. */
  ranges: Array<[number, number]>
}

/**
 * Rank `folders` against what was typed. A name that starts with the query
 * beats one that contains it at a word boundary, which beats a loose
 * in-order match of its letters; a match only in the parent path counts
 * least. Repositories and recent projects get a lift.
 */
export function rankFolders(
  folders: IndexedFolder[],
  query: string,
  boost: (folder: IndexedFolder) => number = () => 0,
): FolderMatch[] {
  const q = query.trim().toLowerCase()
  if (!q) return []
  const out: FolderMatch[] = []
  for (const folder of folders) {
    const name = folder.name.toLowerCase()
    let score = 0
    let ranges: Array<[number, number]> = []
    const at = name.indexOf(q)
    if (at === 0) {
      score = 100
      ranges = [[0, q.length]]
    } else if (at > 0) {
      score = /[\s._-]/.test(name[at - 1]) ? 80 : 60
      ranges = [[at, at + q.length]]
    } else {
      let cursor = 0
      const chars: Array<[number, number]> = []
      for (const char of q) {
        const found = name.indexOf(char, cursor)
        if (found === -1) break
        chars.push([found, found + 1])
        cursor = found + 1
      }
      if (chars.length === q.length) {
        // Tighter matches read as more intentional.
        const span = chars[chars.length - 1][0] - chars[0][0] + 1
        score = 40 - Math.min(20, span - q.length)
        ranges = chars
      } else if (folder.path.toLowerCase().includes(q)) {
        score = 15
      }
    }
    if (score === 0) continue
    score += boost(folder) + (folder.git ? 12 : 0)
    // Shallower paths are more likely the one meant.
    score -= folder.path.split('/').length * 0.5
    out.push({ folder, score, ranges })
  }
  // Once the name itself matches somewhere, loose letter-order and
  // path-only hits are noise.
  const strong = out.some((m) => m.ranges.length === 1)
  return out
    .filter((m) => !strong || m.ranges.length === 1)
    .sort(
      (a, b) => b.score - a.score || a.folder.name.localeCompare(b.folder.name),
    )
}
