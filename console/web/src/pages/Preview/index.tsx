import { useEffect, useRef, useState } from 'react'
import { EmptyState } from '@/components/ui/EmptyState'
import { PageBody, PageHeader, PageShell } from '@/components/ui/PageChrome'
import { getIiiClient } from '@/lib/iii-client'
import type { PanelSide } from '@/types/injectable-ui'

interface PreviewPaneProps {
  /** Host path of the HTML file to render. */
  path: string
  panelSide?: PanelSide
  onRequestClose?: () => void
}

interface ReadResponse {
  content?: string
  is_utf8?: boolean
  more_lines?: boolean
}

type State =
  | { phase: 'loading' }
  | { phase: 'ready'; html: string; truncated: boolean }
  | { phase: 'error'; message: string }

const MAX_PREVIEW_BYTES = 2_000_000

function isHtmlPath(path: string): boolean {
  const lower = path.toLowerCase()
  return (
    lower.endsWith('.html') || lower.endsWith('.htm') || lower.endsWith('.svg')
  )
}

/**
 * Renders an HTML (or SVG) file the agent wrote, in a sandboxed iframe beside
 * the chat. The file is read through `coder::read-file`; nothing is stored in
 * the polled console config. The iframe carries an empty `sandbox`, so page
 * scripts never run and it cannot reach the console around it.
 */
export function PreviewPane({
  path,
  panelSide = 'left',
  onRequestClose,
}: PreviewPaneProps) {
  const [state, setState] = useState<State>({ phase: 'loading' })
  const seqRef = useRef(0)

  useEffect(() => {
    const seq = ++seqRef.current
    setState({ phase: 'loading' })
    getIiiClient()
      .then((client) =>
        client.trigger<ReadResponse>('coder::read-file', {
          path,
          max_output_bytes: MAX_PREVIEW_BYTES,
        }),
      )
      .then((out) => {
        if (seqRef.current !== seq) return
        if (out.is_utf8 === false) {
          setState({ phase: 'error', message: 'not a text file' })
          return
        }
        setState({
          phase: 'ready',
          html: out.content ?? '',
          truncated: out.more_lines === true,
        })
      })
      .catch((err: unknown) => {
        if (seqRef.current !== seq) return
        setState({
          phase: 'error',
          message: err instanceof Error ? err.message : String(err),
        })
      })
  }, [path])

  const name = path.split('/').pop() || 'preview'
  const html = isHtmlPath(path)
    ? state.phase === 'ready'
      ? state.html
      : ''
    : `<pre style="white-space:pre-wrap;font:13px/1.5 ui-monospace,monospace;padding:16px;margin:0">${
        state.phase === 'ready' ? escapeHtml(state.html) : ''
      }</pre>`

  return (
    <PageShell aria-label={`preview ${name}`}>
      <PageHeader
        title={name}
        description={
          state.phase === 'ready' && state.truncated
            ? `${path} (truncated)`
            : path
        }
        onClose={onRequestClose}
      />
      <PageBody side={panelSide} className="min-h-0 p-0">
        {state.phase === 'loading' ? (
          <div className="flex flex-1 items-center justify-center text-sm text-ink-faint">
            loading {name}…
          </div>
        ) : state.phase === 'error' ? (
          <div className="flex flex-1 items-center justify-center p-6">
            <EmptyState
              title="Preview unavailable"
              description={state.message}
            />
          </div>
        ) : (
          <iframe
            title={`preview ${name}`}
            sandbox=""
            srcDoc={html}
            className="size-full min-h-0 flex-1 border-0 bg-white"
          />
        )}
      </PageBody>
    </PageShell>
  )
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}
