import { createContext, useContext, useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { ImageViewer } from '@/components/ui/ImageViewer'
import { type Theme, useTheme } from '@/hooks/use-theme'
import { renderMermaid } from '@/lib/mermaid'

export const MermaidStreamingContext = createContext(false)

type DiagramResult = {
  source: string
  theme: Theme
} & ({ url: string; error?: never } | { error: true; url?: never })

export function MermaidDiagram({ source }: { source: string }) {
  const streaming = useContext(MermaidStreamingContext)
  const [theme] = useTheme()
  const [result, setResult] = useState<DiagramResult | null>(null)
  const [fit, setFit] = useState(true)
  const [expanded, setExpanded] = useState(false)
  const [previewHeight, setPreviewHeight] = useState(320)
  const figureRef = useRef<HTMLElement>(null)
  const viewportRef = useRef<HTMLElement>(null)

  useEffect(() => {
    // Incomplete fences and syntax are expected during streaming. Do not load
    // Mermaid or show parse errors until the assistant finishes its message.
    if (streaming) return
    let active = true
    let url: string | undefined
    void renderMermaid(source, theme).then(
      (svg) => {
        if (!active) return
        // SVG as an image is inert: no scripts, event handlers, interactive
        // links or HTML injection into the chat DOM. Do not bind Mermaid events.
        url = URL.createObjectURL(new Blob([svg], { type: 'image/svg+xml' }))
        setResult({ source, theme, url })
      },
      () => {
        if (active) setResult({ source, theme, error: true })
      },
    )
    return () => {
      active = false
      if (url) URL.revokeObjectURL(url)
    }
  }, [source, theme, streaming])

  const current =
    !streaming && result?.source === source && result.theme === theme
      ? result
      : null
  useEffect(() => {
    if (!current?.url) return
    const figure = figureRef.current
    const viewport = viewportRef.current
    if (!figure || !viewport) return
    const transcript = figure.closest<HTMLElement>('[data-message-list]')
    const px = (value: string) => Number.parseFloat(value) || 0
    const measure = () => {
      const figureStyle = getComputedStyle(figure)
      const viewportStyle = getComputedStyle(viewport)
      const listStyle = transcript ? getComputedStyle(transcript) : null
      const available = transcript
        ? transcript.clientHeight -
          px(listStyle?.paddingTop ?? '') -
          px(listStyle?.paddingBottom ?? '')
        : window.innerHeight * 0.75
      const chrome = figure.offsetHeight - viewport.clientHeight
      const padding =
        px(viewportStyle.paddingTop) + px(viewportStyle.paddingBottom)
      const margin = px(figureStyle.marginTop) + px(figureStyle.marginBottom)
      // Fit BOTH axes, not just the width. A tall flowchart must not extend
      // beyond the transcript viewport underneath the composer.
      setPreviewHeight(
        Math.max(1, Math.min(560, available - chrome - padding - margin)),
      )
    }
    measure()
    const observer =
      typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure)
    if (transcript) observer?.observe(transcript)
    observer?.observe(figure)
    window.addEventListener('resize', measure)
    return () => {
      observer?.disconnect()
      window.removeEventListener('resize', measure)
    }
  }, [current?.url])

  const sourceBlock = (
    <pre className="overflow-x-auto p-4 font-code text-[12.5px] leading-[1.55] text-ink">
      <code className="language-mermaid">{source}</code>
    </pre>
  )

  return (
    <figure
      ref={figureRef}
      className="my-4 min-w-0 max-w-full rounded-md border border-rule-2 bg-bg"
      data-mermaid-diagram=""
    >
      <figcaption className="flex flex-wrap items-center justify-between gap-2 border-b border-rule-2 px-4 py-2 text-xs text-ink-faint">
        <span>Mermaid</span>
        {current?.url ? (
          <div className="flex items-center gap-1">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              aria-label="Fit entire diagram"
              aria-pressed={fit}
              onClick={() => {
                setFit(true)
                viewportRef.current?.scrollTo?.({ left: 0, top: 0 })
              }}
            >
              Fit
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              aria-label="Show diagram at actual size"
              aria-pressed={!fit}
              onClick={() => setFit(false)}
            >
              100%
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              aria-label="Expand diagram"
              onClick={() => setExpanded(true)}
            >
              Expand
            </Button>
          </div>
        ) : null}
      </figcaption>
      {current?.url ? (
        <>
          <ImageViewer
            open={expanded}
            onOpenChange={setExpanded}
            src={current.url}
            alt="Mermaid diagram"
            title="Mermaid diagram"
          />
          <section
            ref={viewportRef}
            style={{ maxHeight: previewHeight + 32 }}
            className="max-w-full overflow-auto p-4 focus-visible:outline focus-visible:outline-2 focus-visible:outline-rule-focus"
            aria-label="Mermaid diagram viewport"
            // biome-ignore lint/a11y/noNoninteractiveTabindex: Keyboard users must be able to scroll wide diagrams.
            tabIndex={0}
          >
            <img
              src={current.url}
              alt="Mermaid diagram"
              className="mx-auto block h-auto"
              style={{
                width: 'auto',
                maxWidth: fit ? '100%' : 'none',
                maxHeight: fit ? previewHeight : 'none',
                objectFit: 'contain',
              }}
            />
          </section>
          <details className="border-t border-rule-2">
            <summary className="cursor-pointer px-4 py-2 text-xs text-ink-faint">
              View source
            </summary>
            {sourceBlock}
          </details>
        </>
      ) : (
        <>
          <p className="px-4 pt-3 text-xs text-ink-faint" role="status">
            {streaming
              ? 'Diagram will render when the message is complete.'
              : current?.error
                ? 'Unable to render this diagram. Mermaid source is shown below.'
                : 'Rendering diagram…'}
          </p>
          {sourceBlock}
        </>
      )}
    </figure>
  )
}
