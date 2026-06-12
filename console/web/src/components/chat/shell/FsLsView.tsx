import { FsEntriesTable } from '@/components/chat/sandbox/FsLsView'
import { Chip } from '@/components/chat/sandbox/terminal/Terminal'
import {
  fsLsRequestSchema,
  fsLsResponseSchema,
  safeParseResponse,
} from './parsers'
import { TargetChip } from './shared'

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
        <TargetChip target={req.data.target} />
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
