import { SquareArrowOutUpRight } from 'lucide-react'
import { useState } from 'react'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
import { copyTextToClipboard } from '@/lib/clipboard'
import { EDITORS, type EditorId, editorById } from '@/lib/editor-links'

/**
 * "open in editor" affordance for coder file-change rows: the console's
 * own shell explorer first (`#/ext/shell/open/<encoded-path>[:line]` —
 * the shell worker serves both the `coder::*` surface that produced this
 * row and that page, so the route always resolves), then external
 * editors (cursor / vs code / zed) via their URL schemes, plus "copy
 * path" — the fallback when the browser isn't on the machine that has
 * the files. Renders nothing for non-absolute paths (result resolution
 * failed — `coder` results are jail-resolved absolute on success).
 */
export function OpenInEditorButton({
  path,
  line,
}: {
  path: string
  line?: number
}) {
  const [copied, setCopied] = useState(false)
  if (!path.startsWith('/')) return null

  const openInShell = () => {
    const anchor =
      typeof line === 'number' && Number.isInteger(line) && line > 0
        ? `:${line}`
        : ''
    window.location.hash = `#/ext/shell/open/${encodeURIComponent(path)}${anchor}`
  }

  const openWith = (id: EditorId) => {
    window.location.href = editorById(id).buildUrl(path, line)
  }

  const copyPath = () => {
    void copyTextToClipboard(path).then((ok) => {
      if (!ok) return
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1200)
    })
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="inline-flex items-center shrink-0 cursor-pointer text-ink-ghost hover:text-ink transition-colors"
          aria-label="open file"
          title="open file"
        >
          <SquareArrowOutUpRight size={12} aria-hidden />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        <DropdownMenuItem onSelect={openInShell}>
          open in shell
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        {EDITORS.map((e) => (
          <DropdownMenuItem key={e.id} onSelect={() => openWith(e.id)}>
            open in {e.label}
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onSelect={(event) => {
            // Keep the menu open so the "copied" flip is visible.
            event.preventDefault()
            copyPath()
          }}
        >
          {copied ? 'copied' : 'copy path'}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
