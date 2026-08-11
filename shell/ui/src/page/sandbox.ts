/* Guest-target plumbing for the explorer — the `shell::fs::*` /
   `shell::exec` twins of coder.ts, threading the per-call
   `target: { kind: "sandbox", sandbox_id }` selector every shell
   function already accepts (host omits the field — the wire default),
   plus tolerant parsing for the `sandbox::list` fleet read and its
   pushed `{ kind: "fleet" }` snapshots. `coder::*` is host-only by
   contract, so guest mode swaps surfaces instead of adding a field to
   the host payloads. */

import type { Host } from '@iii-dev/console-ui'
import type { FlatTree, SearchParams, SearchResponse, TreeNode } from './coder'

/** Tree root for guests — a microVM has no workspace roots to pick. */
export const GUEST_ROOT = '/'

export interface WireTarget {
  kind: 'sandbox'
  sandbox_id: string
}

export function sandboxTarget(sandboxId: string): WireTarget {
  return { kind: 'sandbox', sandbox_id: sandboxId }
}

/* ── payload builders ───────────────────────────────────────────────────
   `target: null` = host: the key is OMITTED, not sent as `{kind:"host"}` —
   an explicit `null` would be a server-side deserialize error (the field
   is `#[serde(default)]` on a non-Option type). */

function withTarget(
  payload: Record<string, unknown>,
  target: WireTarget | null,
): Record<string, unknown> {
  return target === null ? payload : { ...payload, target }
}

export function lsPayload(
  path: string,
  target: WireTarget | null,
): Record<string, unknown> {
  return withTarget({ path }, target)
}

export function statPayload(
  path: string,
  target: WireTarget | null,
): Record<string, unknown> {
  return withTarget({ path }, target)
}

/** Keep the sidebar's match list bounded. */
export const GUEST_GREP_MAX_MATCHES = 500

export function grepPayload(
  args: { path: string; pattern: string; ignoreCase: boolean },
  target: WireTarget | null,
): Record<string, unknown> {
  return withTarget(
    {
      path: args.path,
      pattern: args.pattern,
      ignore_case: args.ignoreCase,
      max_matches: GUEST_GREP_MAX_MATCHES,
    },
    target,
  )
}

export function catPayload(
  path: string,
  target: WireTarget | null,
): Record<string, unknown> {
  // argv form — the path is never shell-tokenized. cwd/env/stdin stay
  // omitted: all three are host-only (S210 on a sandbox target).
  return withTarget(
    { command: 'cat', args: [path], timeout_ms: 15_000 },
    target,
  )
}

/* ── guest calls ──────────────────────────────────────────────────────── */

/** `iii-shell-proto::FsEntry` — only the fields the page reads. */
export interface FsEntry {
  name: string
  is_dir: boolean
  is_symlink: boolean
  size: number
}

export async function guestLs(
  host: Host,
  sandboxId: string,
  path: string,
): Promise<FsEntry[]> {
  const out = await host.iii.trigger<{ entries?: FsEntry[] }>(
    'shell::fs::ls',
    lsPayload(path, sandboxTarget(sandboxId)),
  )
  return out.entries ?? []
}

interface ExecResponse {
  exit_code: number | null
  stdout: string
  stderr: string
  stdout_truncated?: boolean
}

export interface GuestReadResult {
  content: string
  size: number | null
  binary: boolean
  truncated: boolean
}

/** Guest file read. `shell::fs::read` hands back a stream-channel ref the
    console extension client cannot consume (`host.iii` has no channel
    API), and `coder::read-file` is host-only — so guest reads ride
    `shell::exec` stdout, the same way the git tab already consumes file
    bodies (`git show`). Truncation is caught by comparing byte counts
    against `shell::fs::stat`, since the sandbox exec path does not
    report host-side truncation. */
export async function guestReadFile(
  host: Host,
  sandboxId: string,
  path: string,
): Promise<GuestReadResult> {
  const target = sandboxTarget(sandboxId)
  const stat = await host.iii.trigger<{ size?: number | null }>(
    'shell::fs::stat',
    statPayload(path, target),
  )
  const out = await host.iii.trigger<ExecResponse>(
    'shell::exec',
    catPayload(path, target),
  )
  if (out.exit_code !== 0) {
    throw new Error(
      out.stderr.trim() || `cat exited ${out.exit_code ?? 'without a code'}`,
    )
  }
  const size = typeof stat.size === 'number' ? stat.size : null
  return {
    content: out.stdout,
    size,
    binary: out.stdout.includes('\u0000'),
    truncated:
      out.stdout_truncated === true ||
      (size !== null && new TextEncoder().encode(out.stdout).length < size),
  }
}

/** `shell::fs::grep` patterns are always regexes — literal queries get
    their metacharacters escaped. */
export function escapeRegex(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

interface GrepResponse {
  matches: { path: string; line: number; content: string }[]
  truncated: boolean
}

/** Guest search — `shell::fs::grep` mapped onto the SearchTab's
    `coder::search` response shape (grep has no path matches and no
    column, so those degrade to empty/0). */
export async function guestGrep(
  host: Host,
  sandboxId: string,
  { query, regex, ignoreCase, path }: SearchParams,
): Promise<SearchResponse> {
  const out = await host.iii.trigger<GrepResponse>(
    'shell::fs::grep',
    grepPayload(
      {
        path,
        pattern: regex ? query : escapeRegex(query),
        ignoreCase,
      },
      sandboxTarget(sandboxId),
    ),
  )
  return {
    content_matches: (out.matches ?? []).map((m) => ({
      path: m.path,
      line: m.line,
      column: 0,
      text: m.content,
    })),
    path_matches: [],
    truncated: out.truncated === true,
  }
}

/* ── guest tree ───────────────────────────────────────────────────────── */

/** Flatten per-directory `shell::fs::ls` listings (keyed by root-relative
    dir path, '' = the guest root) into the FilesTab's FlatTree. There is
    no recursive tree function on the `shell::fs::*` surface, so the tree
    fills in lazily — an unlisted directory still renders (expandable)
    via its parent's entry; its children appear once its own listing
    lands. Directories carry the trailing-slash marker for the same
    collision reason as flattenTree. */
export function buildGuestTree(
  loaded: ReadonlyMap<string, FsEntry[]>,
): FlatTree {
  const paths: string[] = []
  const kinds = new Map<string, TreeNode['kind']>()

  const walk = (prefix: string) => {
    const entries = loaded.get(prefix)
    if (!entries) return
    const sorted = [...entries].sort((a, b) =>
      a.name < b.name ? -1 : a.name > b.name ? 1 : 0,
    )
    for (const entry of sorted) {
      const childPath = prefix === '' ? entry.name : `${prefix}/${entry.name}`
      const kind = entry.is_dir ? 'dir' : entry.is_symlink ? 'symlink' : 'file'
      paths.push(kind === 'dir' ? `${childPath}/` : childPath)
      kinds.set(childPath, kind)
      if (kind === 'dir') walk(childPath)
    }
  }
  walk('')

  return { paths, kinds, truncations: [] }
}

/* ── fleet parsing ────────────────────────────────────────────────────── */

const asRecord = (v: unknown): Record<string, unknown> | null =>
  v !== null && typeof v === 'object' && !Array.isArray(v)
    ? (v as Record<string, unknown>)
    : null

const asString = (v: unknown): string | null =>
  typeof v === 'string' && v.length > 0 ? v : null

export interface SandboxSummary {
  sandbox_id: string
  name: string | null
  image: string
  stopped: boolean
}

/** `{ sandboxes: [...] }` → summaries. Rows missing `sandbox_id` are
    dropped; every other field degrades to a sane default. */
export function parseSandboxList(value: unknown): SandboxSummary[] {
  const list = asRecord(value)?.sandboxes
  if (!Array.isArray(list)) return []
  const out: SandboxSummary[] = []
  for (const item of list) {
    const rec = asRecord(item)
    const id = rec && asString(rec.sandbox_id)
    if (!rec || !id) continue
    out.push({
      sandbox_id: id,
      name: asString(rec.name),
      image: asString(rec.image) ?? '',
      stopped: rec.stopped === true,
    })
  }
  return out
}

export interface FleetSnapshot {
  sandboxes: SandboxSummary[]
  daemonAbsent: boolean
}

/** A pushed `{ kind: "fleet" }` snapshot; every other event kind returns
    null (exec/runtime events don't change fleet membership). */
export function parseFleetEvent(payload: unknown): FleetSnapshot | null {
  const rec = asRecord(payload)
  if (!rec || rec.kind !== 'fleet') return null
  return {
    sandboxes: parseSandboxList(rec),
    daemonAbsent: rec.daemon_absent === true,
  }
}

/** Does this error read as "the function does not exist on the bus"?
    "function" must sit next to the not-found phrase (spaced or the
    engine's `function_not_found` spelling) so prose that merely contains
    "not found" (a missing path, say) does not hide the picker. */
export function isFunctionNotFound(message: string): boolean {
  return /\bfunction(\s+\S+)?[\s_-]not[ _-]?(found|registered)\b|\bunknown function\b|\bno such function\b/i.test(
    message,
  )
}

export type FleetStatus = 'loading' | 'ready' | 'absent' | 'error'

export interface FleetState {
  status: FleetStatus
  sandboxes: SandboxSummary[]
}

/** The degrade gate: a selected sandbox that left the fleet (or the whole
    daemon left). `loading`/`error` never degrade — a transient read
    failure must not evict an open view. */
export function sandboxGone(
  sandboxId: string | null,
  fleet: FleetState,
): boolean {
  if (sandboxId === null) return false
  if (fleet.status === 'absent') return true
  if (fleet.status !== 'ready') return false
  return !fleet.sandboxes.some((s) => s.sandbox_id === sandboxId)
}
