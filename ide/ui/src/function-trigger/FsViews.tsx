/* The single-outcome shell::fs::* views: ls, stat, read, mkdir, rm, mv,
   chmod. Grep/sed (match lists) live in FsSearchViews.tsx; write (with
   its batch table) in FsWriteView.tsx. Each card opens with the console's
   MetaRow (the request's facts) and says what happened in one line. */

import { Badge, Chip, EmptyState, MetaRow, Table, TableBody, TableCell, TableRow } from '@iii-dev/console-ui'
import { formatBytes } from '@iii-dev/console-ui/format'
import { File, FileText, Folder, Link as LinkIcon } from 'lucide-react'
import { formatMode, truncateMiddle } from '../lib/format'
import { formatEpochMs } from './format'
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
import { displayPath, isSandboxTarget, items, kv, targetItem } from './shared'

interface ViewProps {
  input: unknown
  output: unknown
}

/** Unix-second mtimes (0 = unknown) as `3m ago`. */
const mtime = (secs: number) => formatEpochMs(secs * 1000)

/* ---------------- fs::ls ---------------- */

export function FsLsView({ input, output }: ViewProps) {
  const req = fsLsRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsLsResponseSchema, output)
  if (!resp) return null
  const entries = resp.entries

  return (
    <div className="shui-card">
      <MetaRow items={items(kv('path', req.data.path), targetItem(req.data.target), kv('entries', entries.length))} />
      {entries.length === 0 ? (
        <div className="shui-card-body">
          <EmptyState title="Empty directory" description="There is nothing to list here." />
        </div>
      ) : (
        <FsEntriesTable entries={entries} />
      )}
    </div>
  )
}

/** Directory-listing table shared by the ls view. */
export function FsEntriesTable({ entries }: { entries: FsEntry[] }) {
  return (
    <Table density="compact">
      <TableBody>
        {entries.map((e) => {
          const Icon = e.is_symlink
            ? LinkIcon
            : e.is_dir
              ? Folder
              : iconForFile(e.name)
          return (
            <TableRow key={`${e.name}:${e.size}:${e.mtime}`}>
              <TableCell className="icon">
                <Icon aria-hidden className="shui-glyph t-faint" />
              </TableCell>
              <TableCell className="shui-path t-ink">{e.name}</TableCell>
              <TableCell className="t-faint num r">
                {e.is_dir ? '—' : formatBytes(e.size)}
              </TableCell>
              <TableCell className="t-faint num">
                {`${e.is_dir ? 'd' : '-'}${formatMode(e.mode)}`}
              </TableCell>
              <TableCell className="t-faint">{mtime(e.mtime)}</TableCell>
            </TableRow>
          )
        })}
      </TableBody>
    </Table>
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
      <MetaRow
        items={items(
          kv('size', e.is_dir ? '—' : formatBytes(e.size)),
          kv('mode', `${e.is_dir ? 'd' : '-'}${formatMode(e.mode)}`),
          kv('mtime', mtime(e.mtime)),
          targetItem(req.data.target),
        )}
      >
        {e.is_dir ? <Badge>dir</Badge> : null}
        {e.is_symlink ? <Badge variant="warn">symlink</Badge> : null}
      </MetaRow>
      <div className="shui-card-body shui-line">
        <span className="t-faint">stat </span>
        <span className="shui-path">{req.data.path}</span>
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
      <MetaRow
        items={items(
          kv('file', req.data.path),
          targetItem(req.data.target),
          kv('size', formatBytes(resp.size)),
          kv('mode', formatMode(resp.mode)),
          kv('mtime', mtime(resp.mtime)),
        )}
      >
        <Badge>streamed</Badge>
      </MetaRow>
      <div className="shui-card-body shui-row">
        <span className="t-faint">content streamed via channel</span>
        <code className="shui-inline-code">{truncateMiddle(resp.content.channel_id, 18)}</code>
        <span className="t-ghost">({resp.content.direction ?? 'read'})</span>
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
      <MetaRow
        items={items(
          kv('mode', req.data.mode ?? '0755'),
          req.data.parents ? kv('parents', 'true') : null,
          targetItem(req.data.target),
        )}
      />
      <div className="shui-card-body shui-line">
        <span className={created ? 't-accent' : 't-faint'}>{verb}</span>
        <span className="shui-path">{displayPath(req.data.path, resp.path)}</span>
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
      {recursive || sandboxed ? (
        <MetaRow items={items(targetItem(req.data.target))}>
          {recursive ? <Chip tone="warning">recursive</Chip> : null}
        </MetaRow>
      ) : null}
      <div className="shui-card-body shui-line">
        <span className={tone}>{verb}</span>
        <span className="shui-path">{displayPath(req.data.path, resp.path)}</span>
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
      {hasChips ? (
        <MetaRow items={items(targetItem(req.data.target), req.data.overwrite ? kv('overwrite', 'true') : null)}>
          {showOverwrote ? <Badge variant="warn">overwrote existing</Badge> : null}
        </MetaRow>
      ) : null}
      <div className="shui-card-body shui-baseline-row">
        <span className={moved ? 't-accent' : 't-faint'}>{moved ? 'mv' : '·'}</span>
        <span className="shui-path">{displayPath(req.data.src, resp.src)}</span>
        <span className="t-ghost">→</span>
        <span className="shui-path">{displayPath(req.data.dst, resp.dst)}</span>
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
      <MetaRow
        items={items(
          ownership ? kv('own', ownership) : null,
          req.data.recursive ? kv('recursive', 'true') : null,
          kv('changed', resp.entries_changed),
          targetItem(req.data.target),
        )}
      />
      <div className="shui-card-body shui-baseline-row">
        <span className="t-faint">chmod</span>
        <span className="shui-path">{displayPath(req.data.path, resp.path)}</span>
        <span className="t-ghost">→</span>
        <span className="num">{req.data.mode}</span>
        <span className="t-faint">({formatMode(req.data.mode)})</span>
      </div>
    </div>
  )
}
