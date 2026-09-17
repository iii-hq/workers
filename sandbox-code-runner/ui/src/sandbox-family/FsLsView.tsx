/**
 * `sandbox::fs::ls` — the directory listing table. The console version
 * used lucide icons; worker assets don't bundle an icon set, so entries
 * carry a small text glyph instead (`▸` dir, `↪` symlink, `·` file).
 */

import { Table, TableBody, TableCell, TableRow } from '@iii-dev/console-ui'
import { File, Folder, Link2 } from 'lucide-react'
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

function EntryGlyph({ entry }: { entry: FsEntry }) {
  const Icon = entry.is_symlink ? Link2 : entry.is_dir ? Folder : File
  return <Icon size={16} aria-hidden />
}

function FsEntriesTable({ entries }: { entries: FsEntry[] }) {
  return (
    <Table density="compact" inset>
      <TableBody>
        {entries.map((e) => (
          <TableRow key={`${e.name}:${e.size}:${e.mtime}`}>
            <TableCell className="glyph">
              <EntryGlyph entry={e} />
            </TableCell>
            <TableCell>{e.name}</TableCell>
            <TableCell className="faint num right">{e.is_dir ? '—' : formatBytes(e.size)}</TableCell>
            <TableCell className="faint num">{`${e.is_dir ? 'd' : '-'}${formatMode(e.mode)}`}</TableCell>
            <TableCell className="faint">{formatMtime(e.mtime)}</TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}
