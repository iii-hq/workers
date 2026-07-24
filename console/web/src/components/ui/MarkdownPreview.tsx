import { Markdown } from '@/lib/markdown'
import { cn } from '@/lib/utils'

interface MarkdownPreviewProps {
  markdown: string
  className?: string
}

/**
 * Rendered-markdown pane: `Markdown` with the standard `bg-bg` chrome the
 * function-trigger cards use for skill/prompt/README bodies. The shared
 * counterpart to `CodeEditor` — edit raw markdown on one side, preview it
 * through this on the other — and the component worker UI should reach
 * for instead of re-wrapping `Markdown` per worker.
 */
export function MarkdownPreview({ markdown, className }: MarkdownPreviewProps) {
  return (
    <div className={cn('bg-bg px-3 py-2', className)}>
      <Markdown>{markdown}</Markdown>
    </div>
  )
}
