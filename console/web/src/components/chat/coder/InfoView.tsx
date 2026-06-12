/**
 * `coder::info` — config/discovery panel: allowed roots (primary marked),
 * byte budgets, list/search/tree defaults, and the two glob lists.
 * Reference data, kept compact — this is the call to suggest whenever
 * another coder call rejects a path.
 */
import { formatBytes } from '@/components/chat/sandbox/format'
import { Chip, FooterPill } from '@/components/chat/sandbox/terminal/Terminal'
import {
  type InfoResponse,
  infoRequestSchema,
  infoResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

interface InfoViewProps {
  input: unknown
  output?: unknown
  running?: boolean
  preview?: boolean
}

/** Row in a key→value config grid; labels are the exact wire field
    names so humans can correlate with C213 budget errors verbatim. */
interface ConfigRow {
  label: string
  value: string
  note?: string
}

/** `"1.0 MiB (1,048,576 B)"` — humanized plus the exact wire value,
    since errors compare against exact byte counts. */
function formatBytesWithRaw(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '—'
  if (bytes < 1024) return `${bytes} B`
  return `${formatBytes(bytes)} (${bytes.toLocaleString('en-US')} B)`
}

/** Byte budgets — exceeding any of these surfaces as C213. */
function budgetRows(info: InfoResponse): ConfigRow[] {
  return [
    {
      label: 'batch_read_budget_bytes',
      value: formatBytesWithRaw(info.batch_read_budget_bytes),
      note: 'per paths[] batch read',
    },
    {
      label: 'max_output_bytes',
      value: formatBytesWithRaw(info.max_output_bytes),
      note: 'single-path full-read context',
    },
    {
      label: 'max_read_bytes',
      value: formatBytesWithRaw(info.max_read_bytes),
      note: 'per-file IO ceiling + search scan',
    },
    {
      label: 'max_write_bytes',
      value: formatBytesWithRaw(info.max_write_bytes),
      note: 'per single file write',
    },
    {
      label: 'search_response_budget_bytes',
      value: formatBytesWithRaw(info.search_response_budget_bytes),
      note: 'per search response',
    },
  ]
}

/** Defaults applied when a request omits the knob, plus hard caps. */
function limitRows(info: InfoResponse): ConfigRow[] {
  return [
    {
      label: 'list_default_page_size',
      value: String(info.list_default_page_size),
      note: 'list-folder page_size default',
    },
    {
      label: 'list_max_page_size',
      value: String(info.list_max_page_size),
      note: 'page_size hard cap',
    },
    {
      label: 'search_default_max_matches',
      value: String(info.search_default_max_matches),
      note: 'search max_matches default',
    },
    {
      label: 'search_default_max_line_bytes',
      value: formatBytesWithRaw(info.search_default_max_line_bytes),
      note: 'per matched line',
    },
    {
      label: 'tree_default_depth',
      value: String(info.tree_default_depth),
      note: 'tree max_depth default',
    },
    {
      label: 'tree_per_folder_limit',
      value: String(info.tree_per_folder_limit),
      note: 'entries per tree folder',
    },
  ]
}

/** Allowed roots in config order; index 0 is where relative paths resolve. */
function RootsList({ basePaths }: { basePaths: string[] }) {
  return (
    <div className="border-b border-rule-2 last:border-b-0">
      <div className="px-3 pt-2 pb-1 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
        allowed roots
      </div>
      {basePaths.length === 0 ? (
        <div className="px-3 pb-2 font-mono text-[12px] text-ink-ghost">
          · none
        </div>
      ) : (
        <div className="px-3 pb-2 flex flex-col gap-1">
          {basePaths.map((root, i) => (
            <div
              key={root}
              className="font-mono text-[12px] text-ink flex flex-wrap items-center gap-1.5"
            >
              <span className="break-all">{root}</span>
              {i === 0 ? <FooterPill tone="accent">primary</FooterPill> : null}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

function ConfigSection({ title, rows }: { title: string; rows: ConfigRow[] }) {
  return (
    <div className="border-b border-rule-2 last:border-b-0">
      <div className="px-3 pt-2 pb-1 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
        {title}
      </div>
      <table className="w-full font-mono text-[12px] text-ink mb-1">
        <tbody>
          {rows.map((row) => (
            <tr key={row.label}>
              <td className="px-3 py-0.5 text-ink-faint whitespace-nowrap align-top">
                {row.label}
              </td>
              <td className="px-3 py-0.5 tabular-nums break-all">
                {row.value}
                {row.note ? (
                  <span className="text-ink-ghost"> · {row.note}</span>
                ) : null}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

/** Glob list with its semantic note — the two lists mean different
    things (access protection vs noise filter), never conflate them. */
function GlobList({
  title,
  note,
  globs,
  warn,
}: {
  title: string
  note: string
  globs: string[]
  warn?: boolean
}) {
  return (
    <div className="border-b border-rule-2 last:border-b-0 px-3 py-2 flex flex-col gap-1.5">
      <div className="font-mono text-[11px] text-ink-faint">
        <span className="uppercase tracking-[0.06em]">{title}</span>
        <span className="text-ink-ghost"> · {note}</span>
      </div>
      {globs.length === 0 ? (
        <span className="font-mono text-[12px] text-ink-ghost">· none</span>
      ) : (
        <div className="flex flex-wrap gap-1.5">
          {globs.map((glob) => (
            <Chip
              key={glob}
              className={warn ? 'border-warn text-warn' : undefined}
            >
              {glob}
            </Chip>
          ))}
        </div>
      )}
    </div>
  )
}

export function InfoView({ input, output, running, preview }: InfoViewProps) {
  // Request is `{}` — trivially satisfiable, only bail on non-object junk.
  const req = safeParseRequest(infoRequestSchema, input)
  if (!req) return null
  const resp =
    output != null && !preview
      ? safeParseResponse(infoResponseSchema, output)
      : null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <span className="font-mono text-[12.5px] text-ink">
          <span className="text-ink-faint">coder </span>
          <span>info</span>
        </span>
        {resp ? <Chip label="version">{resp.version}</Chip> : null}
        {resp ? <Chip label="roots">{resp.base_paths.length}</Chip> : null}
        {running ? (
          <span className="font-mono text-[11px] text-ink-ghost animate-pulse">
            · querying…
          </span>
        ) : null}
      </div>

      {resp ? (
        <>
          <RootsList basePaths={resp.base_paths} />
          <ConfigSection title="byte budgets" rows={budgetRows(resp)} />
          <ConfigSection title="defaults & caps" rows={limitRows(resp)} />
          <GlobList
            title="non_accessible_globs"
            note="access-protected — ops return C211"
            globs={resp.non_accessible_globs}
            warn
          />
          <GlobList
            title="default_exclude_globs"
            note="noise filter only — bypass with use_default_excludes: false"
            globs={resp.default_exclude_globs}
          />
        </>
      ) : running ? null : (
        <div className="px-3 py-2 font-mono text-[12px] text-ink-ghost">
          · no config reported
        </div>
      )}
    </div>
  )
}
