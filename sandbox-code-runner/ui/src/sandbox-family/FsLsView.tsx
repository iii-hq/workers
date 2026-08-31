/**
 * `sandbox::fs::ls` — the directory listing table. The console version
 * used lucide icons; worker assets don't bundle an icon set, so entries
 * carry a small text glyph instead (`▸` dir, `↪` symlink, `·` file).
 */

import { formatBytes, formatMode, formatMtime } from './format'
import { type FsEntry, fsLsRequestSchema, fsLsResponseSchema, safeParseResponse } from './parsers'
import { Chip, SandboxIdChip } from './shared'

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
    <div className="cr-fam-card">
      <div className="cr-fam-chips-row">
        <SandboxIdChip sandboxId={req.data.sandbox_id} />
        <Chip label="path">{req.data.path}</Chip>
        <Chip label="entries">{entries.length}</Chip>
      </div>
      {entries.length === 0 ? (
        <div className="cr-fam-note-ghost">· directory is empty</div>
      ) : (
        <FsEntriesTable entries={entries} />
      )}
    </div>
  )
}

function entryGlyph(e: FsEntry): string {
  if (e.is_symlink) return '↪'
  if (e.is_dir) return '▸'
  return '·'
}

/** Directory-listing table — both wires speak `FsEntry`. */
function FsEntriesTable({ entries }: { entries: FsEntry[] }) {
  return (
    <table className="cr-fam-table plain">
      <tbody>
        {entries.map((e) => (
          <tr key={`${e.name}:${e.size}:${e.mtime}`}>
            <td className="glyph" aria-hidden>
              {entryGlyph(e)}
            </td>
            <td>{e.name}</td>
            <td className="faint num right">{e.is_dir ? '—' : formatBytes(e.size)}</td>
            <td className="faint num">{`${e.is_dir ? 'd' : '-'}${formatMode(e.mode)}`}</td>
            <td className="faint">{formatMtime(e.mtime)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}
