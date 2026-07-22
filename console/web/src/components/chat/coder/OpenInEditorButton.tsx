import { ChevronDown, SquareArrowOutUpRight } from 'lucide-react'
import { useState } from 'react'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
import { copyTextToClipboard } from '@/lib/clipboard'
import {
  EDITORS,
  type EditorId,
  editorById,
  getPreferredEditor,
  setPreferredEditor,
} from '@/lib/editor-links'

/**
 * Split "open in editor" affordance for coder file-change rows: the anchor
 * opens the preferred editor via its URL scheme (one click), the chevron
 * menu switches editors (persisting the choice) or copies the path — the
 * fallback when the browser isn't on the machine that has the files.
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
  const [preferred, setPreferred] = useState<EditorId>(getPreferredEditor)
  const [copied, setCopied] = useState(false)
  if (!path.startsWith('/')) return null

  const editor = editorById(preferred)

  const openWith = (id: EditorId) => {
    setPreferred(id)
    setPreferredEditor(id)
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
    <span className="inline-flex items-center gap-0.5 shrink-0">
      <a
        href={editor.buildUrl(path, line)}
        draggable={false}
        className="text-ink-ghost hover:text-ink transition-colors"
        aria-label={`open in ${editor.label}`}
        title={`open in ${editor.label}`}
      >
        <SquareArrowOutUpRight size={12} aria-hidden />
      </a>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className="text-ink-ghost hover:text-ink transition-colors"
            aria-label="editor options"
            title="editor options"
          >
            <ChevronDown size={10} aria-hidden />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          {EDITORS.map((e) => (
            <DropdownMenuItem key={e.id} onSelect={() => openWith(e.id)}>
              open in {e.label}
              {e.id === preferred ? (
                <span className="text-ink-ghost">· default</span>
              ) : null}
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
    </span>
  )
}
