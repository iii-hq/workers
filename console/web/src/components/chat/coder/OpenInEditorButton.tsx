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
 * "open in editor" affordance for coder file-change rows: one button that
 * always opens a menu of editors (cursor / vs code / zed) — each entry
 * launches that editor via its URL scheme — plus "copy path", the fallback
 * when the browser isn't on the machine that has the files. No editor is
 * privileged; the menu is shown every time so the choice stays explicit.
 * Renders nothing for non-absolute paths (result resolution failed —
 * `coder` results are jail-resolved absolute on success).
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
          className="inline-flex items-center shrink-0 text-ink-ghost hover:text-ink transition-colors"
          aria-label="open in editor"
          title="open in editor"
        >
          <SquareArrowOutUpRight size={12} aria-hidden />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
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
