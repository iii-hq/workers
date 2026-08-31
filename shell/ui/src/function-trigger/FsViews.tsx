/* The single-outcome shell::fs::* views: ls, stat, read, mkdir, rm, mv,
   chmod. Grep/sed (match lists) live in FsSearchViews.tsx; write (with
   its batch table) in FsWriteView.tsx. */

import { File, FileText, Folder, Link as LinkIcon } from 'lucide-react'
import { formatBytes, formatMode, formatMtime, truncateMiddle } from '../lib/format'
import { Chip, FooterPill } from '../lib/terminal'
import {
  type FsEntry,
  fsChmodRequestSchema,
  fsChmodResponseSchema,
  fsLsRequestSchema,
  fsLsResponseSchema,
  fsMkdirRequestSchema,
  fsMkdirResponseSchema,
  fsMvRequestSchema,
  fsMvResponseSchema,
  fsReadRequestSchema,
  fsReadResponseSchema,
  fsRmRequestSchema,
  fsRmResponseSchema,
  fsStatRequestSchema,
  fsStatResponseSchema,
  safeParseResponse,
} from './parsers'
import { displayPath, isSandboxTarget, TargetChip } from './shared'

interface ViewProps {
  input: unknown
  output: unknown
}

/* ---------------- fs::ls ---------------- */

export function FsLsView({ input, output }: ViewProps) {
  const req = fsLsRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsLsResponseSchema, output)
  if (!resp) return null
  const entries = resp.entries

  return (
    <div className="shui-card">
      <div className="shui-head">
        <span className="shui-chips">
          <Chip label="path">{req.data.path}</Chip>
          <TargetChip target={req.data.target} />
          <Chip label="entries">{entries.length}</Chip>
        </span>
      </div>
      {entries.length === 0 ? (
        <div className="shui-empty">· directory is empty</div>
      ) : (
        <FsEntriesTable entries={entries} />
      )}
    </div>
  )
}

/** Directory-listing table shared by the ls view. */
export function FsEntriesTable({ entries }: { entries: FsEntry[] }) {
  return (
    <table className="shui-table plain">
      <tbody>
        {entries.map((e) => {
          const Icon = e.is_symlink
            ? LinkIcon
            : e.is_dir
              ? Folder
              : iconForFile(e.name)
          return (
            <tr key={`${e.name}:${e.size}:${e.mtime}`}>
              <td className="pad-l icon">
                <Icon aria-hidden className="shui-fs-icon" />
              </td>
              <td className="t-ink">{e.name}</td>
              <td className="t-faint num r">
                {e.is_dir ? '—' : formatBytes(e.size)}
              </td>
              <td className="t-faint num">
                {`${e.is_dir ? 'd' : '-'}${formatMode(e.mode)}`}
              </td>
              <td className="t-faint pad-r">{formatMtime(e.mtime)}</td>
            </tr>
          )
        })}
      </tbody>
    </table>
  )
}

function iconForFile(name: string) {
  const lower = name.toLowerCase()
  if (/\.(md|txt|json|yml|yaml|toml|csv|log)$/.test(lower)) return FileText
  if (/\.(js|jsx|ts|tsx|py|rs|go|rb|sh|bash)$/.test(lower)) return FileText
  return File
}

/* ---------------- fs::stat ---------------- */

/** Response is the bare FsEntry (`StatResponse` is serde-transparent). */
export function FsStatView({ input, output }: ViewProps) {
  const req = fsStatRequestSchema.safeParse(input)
  if (!req.success) return null
  const e = safeParseResponse(fsStatResponseSchema, output)
  if (!e) return null

  return (
    <div className="shui-card">
      <div className="shui-slab">
        <div className="shui-line">
          <span className="t-faint">stat </span>
          <span>{req.data.path}</span>
        </div>
        <div className="shui-row">
          <Chip label="size">{e.is_dir ? '—' : formatBytes(e.size)}</Chip>
          <Chip label="mode">{`${e.is_dir ? 'd' : '-'}${formatMode(e.mode)}`}</Chip>
          <Chip label="mtime">{formatMtime(e.mtime)}</Chip>
          <TargetChip target={req.data.target} />
          {e.is_dir ? <FooterPill tone="default">dir</FooterPill> : null}
          {e.is_symlink ? <FooterPill tone="warn">symlink</FooterPill> : null}
        </div>
      </div>
    </div>
  )
}

/* ---------------- fs::read ---------------- */

/** shell::fs::read never inlines content — the response `content` is
    always a channel ref, so the body is the stream row promoted to the
    only branch. The console cannot dereference the channel, so there is
    no "view content" affordance. */
export function FsReadView({ input, output }: ViewProps) {
  const req = fsReadRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsReadResponseSchema, output)
  if (!resp) return null

  return (
    <div className="shui-card">
      <div className="shui-head">
        <span className="shui-chips">
          <span className="shui-err-label">file</span>
          <code className="t-ink">{req.data.path}</code>
          <TargetChip target={req.data.target} />
        </span>
      </div>

      <div className="shui-body stream-row">
        <span className="t-faint">content streamed via channel</span>
        <code className="shui-inline-code">
          {truncateMiddle(resp.content.channel_id, 18)}
        </code>
        <span className="t-ghost">({resp.content.direction ?? 'read'})</span>
      </div>

      <div className="shui-foot">
        <Chip label="size">{formatBytes(resp.size)}</Chip>
        <Chip label="mode">{formatMode(resp.mode)}</Chip>
        <Chip label="mtime">{formatMtime(resp.mtime)}</Chip>
        <FooterPill tone="default">streamed</FooterPill>
      </div>
    </div>
  )
}

/* ---------------- fs::mkdir ---------------- */

export function FsMkdirView({ input, output }: ViewProps) {
  const req = fsMkdirRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsMkdirResponseSchema, output)
  if (!resp) return null
  const created = resp.created
  const sandboxed = isSandboxTarget(req.data.target)
  // Sandbox-target responses default `already_existed` (serde fill-in,
  // not a signal) — collapse to the plain created/exists wording there.
  const verb = created
    ? '+ created '
    : sandboxed
      ? '· exists '
      : resp.already_existed
        ? '· already exists '
        : '· not created '

  return (
    <div className="shui-card">
      <div className="shui-slab">
        <div className="shui-line">
          <span className={created ? 't-accent' : 't-faint'}>{verb}</span>
          <span>{displayPath(req.data.path, resp.path)}</span>
        </div>
        <div className="shui-row">
          <Chip label="mode">{req.data.mode ?? '0755'}</Chip>
          {req.data.parents ? <Chip label="parents">true</Chip> : null}
          <TargetChip target={req.data.target} />
        </div>
      </div>
    </div>
  )
}

/* ---------------- fs::rm ---------------- */

export function FsRmView({ input, output }: ViewProps) {
  const req = fsRmRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsRmResponseSchema, output)
  if (!resp) return null
  const removed = resp.removed
  const sandboxed = isSandboxTarget(req.data.target)
  // `was_present` is a host-only signal; sandbox-target responses default
  // it (serde fill-in) — collapse to removed/not-removed wording there.
  const wasAbsent = !sandboxed && !removed && !resp.was_present
  const verb = removed
    ? '− removed '
    : wasAbsent
      ? '· was not present '
      : '· not removed '
  const tone = removed ? 't-warn' : wasAbsent ? 't-ghost' : 't-faint'
  const recursive = req.data.recursive === true

  return (
    <div className="shui-card">
      <div className="shui-slab">
        <div className="shui-line">
          <span className={tone}>{verb}</span>
          <span>{displayPath(req.data.path, resp.path)}</span>
        </div>
        {recursive || sandboxed ? (
          <div className="shui-row">
            {recursive ? (
              <Chip label="recursive" className="warn">
                true
              </Chip>
            ) : null}
            <TargetChip target={req.data.target} />
          </div>
        ) : null}
      </div>
    </div>
  )
}

/* ---------------- fs::mv ---------------- */

export function FsMvView({ input, output }: ViewProps) {
  const req = fsMvRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsMvResponseSchema, output)
  if (!resp) return null
  const moved = resp.moved
  /* Sandbox-target responses default `overwrote` — a serde fill-in, not
     a signal; only surface the warn pill for host moves. */
  const showOverwrote = resp.overwrote && !isSandboxTarget(req.data.target)
  const hasChips =
    isSandboxTarget(req.data.target) || !!req.data.overwrite || showOverwrote

  return (
    <div className="shui-card">
      <div className="shui-slab">
        <div className="shui-baseline-row">
          <span className={moved ? 't-accent' : 't-faint'}>
            {moved ? 'mv' : '·'}
          </span>
          <span>{displayPath(req.data.src, resp.src)}</span>
          <span className="t-ghost">→</span>
          <span>{displayPath(req.data.dst, resp.dst)}</span>
        </div>
        {hasChips ? (
          <div className="shui-row">
            <TargetChip target={req.data.target} />
            {req.data.overwrite ? <Chip label="overwrite">true</Chip> : null}
            {showOverwrote ? (
              <FooterPill tone="warn">overwrote existing</FooterPill>
            ) : null}
          </div>
        ) : null}
      </div>
    </div>
  )
}

/* ---------------- fs::chmod ---------------- */

export function FsChmodView({ input, output }: ViewProps) {
  const req = fsChmodRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsChmodResponseSchema, output)
  if (!resp) return null
  const ownership =
    typeof req.data.uid === 'number' || typeof req.data.gid === 'number'
      ? `${req.data.uid ?? '_'}:${req.data.gid ?? '_'}`
      : null

  return (
    <div className="shui-card">
      <div className="shui-slab">
        <div className="shui-baseline-row">
          <span className="t-faint">chmod</span>
          <span>{displayPath(req.data.path, resp.path)}</span>
          <span className="t-ghost">→</span>
          <span className="num">{req.data.mode}</span>
          <span className="t-faint">({formatMode(req.data.mode)})</span>
        </div>
        <div className="shui-row">
          {ownership ? <Chip label="own">{ownership}</Chip> : null}
          {req.data.recursive ? <Chip label="recursive">true</Chip> : null}
          <Chip label="changed">{resp.entries_changed}</Chip>
          <TargetChip target={req.data.target} />
        </div>
      </div>
    </div>
  )
}
