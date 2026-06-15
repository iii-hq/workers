import { File, FileText, Folder, Link as LinkIcon } from 'lucide-react'
import { formatBytes, formatMode, formatMtime } from './format'
import {
  type FsEntry,
  fsLsRequestSchema,
  fsLsResponseSchema,
  safeParseResponse,
} from './parsers'
import { Chip } from './terminal/Terminal'

interface FsLsViewProps {
  input: unknown
  output: unknown
}

export function FsLsView({ input, output }: FsLsViewProps) {
  const req = fsLsRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsLsResponseSchema, output)
  if (!resp) return null
  const entries = resp.entries

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <Chip label="path">{req.data.path}</Chip>
        <Chip label="entries">{entries.length}</Chip>
      </div>
      {entries.length === 0 ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
          · directory is empty
        </div>
      ) : (
        <FsEntriesTable entries={entries} />
      )}
    </div>
  )
}

/** Directory-listing table. Shared with the shell module's `fs::ls`
    renderer — both wires speak `FsEntry`. */
export function FsEntriesTable({ entries }: { entries: FsEntry[] }) {
  return (
    <table className="w-full font-mono text-[12px] text-ink">
      <tbody>
        {entries.map((e) => {
          const Icon = e.is_symlink
            ? LinkIcon
            : e.is_dir
              ? Folder
              : iconForFile(e.name)
          return (
            <tr
              key={`${e.name}:${e.size}:${e.mtime}`}
              className="border-b border-rule-2 last:border-b-0"
            >
              <td className="pl-3 pr-1.5 py-1.5 w-5">
                <Icon aria-hidden className="w-3.5 h-3.5 text-ink-faint" />
              </td>
              <td className="px-1.5 py-1.5 text-ink">{e.name}</td>
              <td className="px-2 py-1.5 text-ink-faint tabular-nums text-right">
                {e.is_dir ? '—' : formatBytes(e.size)}
              </td>
              <td className="px-2 py-1.5 text-ink-faint tabular-nums">
                {`${e.is_dir ? 'd' : '-'}${formatMode(e.mode)}`}
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

function iconForFile(name: string) {
  const lower = name.toLowerCase()
  if (/\.(md|txt|json|yml|yaml|toml|csv|log)$/.test(lower)) return FileText
  if (/\.(js|jsx|ts|tsx|py|rs|go|rb|sh|bash)$/.test(lower)) return FileText
  return File
}
