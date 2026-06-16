/**
 * `coder::list-folder` — paginated flat listing of one folder, the
 * follow-up call for `coder::tree` per_folder_limit stubs. Wire notes
 * (list_folder.rs): entries carry basenames only (full path = response
 * `path` + "/" + name), `page_size` echoes the EFFECTIVE size after
 * default fill / cap clamp (may differ from the request — show what was
 * actually used), and `non_accessible` is a plain bool, always
 * serialized, unlike tree's omit-when-false.
 */
import { formatBytes, formatMtime } from '@/components/chat/sandbox/format'
import { Chip, FooterPill } from '@/components/chat/sandbox/terminal/Terminal'
import { iconForEntry, LockedBadge } from './entryShared'
import {
  type EntryKind,
  joinEntryPath,
  listFolderRequestSchema,
  listFolderResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

interface ListFolderViewProps {
  input: unknown
  output?: unknown
  running?: boolean
}

export function ListFolderView({
  input,
  output,
  running,
}: ListFolderViewProps) {
  const req = safeParseRequest(listFolderRequestSchema, input)
  if (!req) return null
  const resp =
    output != null ? safeParseResponse(listFolderResponseSchema, output) : null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <Chip label="path">{resp?.path ?? req.path ?? '.'}</Chip>
        {resp ? (
          <>
            <Chip label="page">
              {pageLabel(resp.page, resp.page_size, resp.total)}
            </Chip>
            {/* Effective size — the worker may clamp or default-fill it. */}
            <Chip label="size">{resp.page_size}</Chip>
            <Chip label="total">{resp.total}</Chip>
          </>
        ) : (
          <>
            {req.page != null ? <Chip label="page">{req.page}</Chip> : null}
            {req.page_size != null ? (
              <Chip label="size">{req.page_size}</Chip>
            ) : null}
          </>
        )}
        {running ? (
          <span className="font-mono text-[11px] text-ink-ghost animate-pulse">
            · listing…
          </span>
        ) : null}
      </div>

      {resp ? (
        resp.entries.length === 0 ? (
          <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
            {resp.total === 0
              ? '· directory is empty'
              : '· no entries on this page'}
          </div>
        ) : (
          <EntriesTable path={resp.path} entries={resp.entries} />
        )
      ) : (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
          {running ? '· listing…' : '· no listing data'}
        </div>
      )}

      {resp?.has_more ? (
        <div className="bg-paper-2 border-t border-rule-2 px-3 py-1.5">
          <FooterPill tone="warn">more pages available</FooterPill>
        </div>
      ) : null}
    </div>
  )
}

interface EntriesTableProps {
  /** Canonical folder path — the prefix for every entry basename. */
  path: string
  entries: {
    name: string
    kind: EntryKind
    size: number
    mtime: number
    non_accessible: boolean
  }[]
}

function EntriesTable({ path, entries }: EntriesTableProps) {
  return (
    <table className="w-full font-mono text-[12px] text-ink">
      <tbody>
        {entries.map((e) => {
          const Icon = iconForEntry(e.kind, e.name)
          return (
            <tr key={e.name} className="border-b border-rule-2 last:border-b-0">
              <td className="pl-3 pr-1.5 py-1.5 w-5">
                <Icon aria-hidden className="w-3.5 h-3.5 text-ink-faint" />
              </td>
              <td
                className="px-1.5 py-1.5 text-ink"
                title={joinEntryPath(path, e.name)}
              >
                <span className="inline-flex items-center gap-1.5">
                  <span>{e.kind === 'dir' ? `${e.name}/` : e.name}</span>
                  {e.non_accessible ? <LockedBadge /> : null}
                </span>
              </td>
              <td className="px-2 py-1.5 text-ink-faint tabular-nums text-right">
                {e.kind === 'dir' ? '—' : formatBytes(e.size)}
              </td>
              <td className="pr-3 pl-2 py-1.5 text-ink-faint">
                {formatMtime(e.mtime)}
              </td>
            </tr>
          )
        })}
      </tbody>
    </table>
  )
}

/**
 * "page/totalPages" chip text. Both `total` and the EFFECTIVE
 * `page_size` come from the response, so the page count reflects what
 * the worker actually used (the requested size may have been clamped).
 */
export function pageLabel(
  page: number,
  pageSize: number,
  total: number,
): string {
  if (pageSize <= 0) return String(page)
  const totalPages = Math.max(1, Math.ceil(total / pageSize))
  return `${page}/${totalPages}`
}
