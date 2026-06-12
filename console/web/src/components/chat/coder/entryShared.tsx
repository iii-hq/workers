/**
 * Entry rendering shared by `coder::tree` and `coder::list-folder` — both
 * surface the same `EntryKind` + `non_accessible` wire vocabulary, so the
 * icon pick and the "locked" marker live here once.
 */
import {
  File,
  FileText,
  Folder,
  Link as LinkIcon,
  TriangleAlert,
} from 'lucide-react'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
import type { EntryKind } from './parsers'

/** "Visible but locked" marker for non_accessible entries (C211 on use). */
export function LockedBadge() {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="inline-flex items-center gap-1 text-warn cursor-help text-[10px] uppercase tracking-[0.06em]">
          <TriangleAlert aria-hidden className="w-3 h-3" />
          locked
        </span>
      </TooltipTrigger>
      <TooltipContent>
        matches non_accessible_globs — listed, but coder::* ops on it return
        C211
      </TooltipContent>
    </Tooltip>
  )
}

/** Mirror FsLsView's icon picks: symlink → link, dir → folder, known
    text/code extensions → FileText, everything else (incl. "other") → File. */
export function iconForEntry(kind: EntryKind, name: string) {
  if (kind === 'symlink') return LinkIcon
  if (kind === 'dir') return Folder
  const lower = name.toLowerCase()
  if (/\.(md|txt|json|yml|yaml|toml|csv|log)$/.test(lower)) return FileText
  if (/\.(js|jsx|ts|tsx|py|rs|go|rb|sh|bash)$/.test(lower)) return FileText
  return File
}
