import { useEffect, useRef } from 'react'

export interface DeckFrameProps {
  html: string
  index: number
  interactive?: boolean
  selectedBlockId?: string | null
  onSelect?: (blockId: string | null, index: number) => void
  className?: string
  title?: string
}

export function DeckFrame({
  html,
  index,
  interactive = false,
  selectedBlockId,
  onSelect,
  className,
  title,
}: DeckFrameProps) {
  const frame = useRef<HTMLIFrameElement>(null)
  const post = (message: Record<string, unknown>) => frame.current?.contentWindow?.postMessage(message, '*')

  useEffect(() => {
    post({ type: 'slides:goto', index })
  }, [index])

  useEffect(() => {
    post({ type: 'slides:highlight', blockId: selectedBlockId ?? null })
  }, [selectedBlockId])

  useEffect(() => {
    if (!interactive) return
    const onMessage = (event: MessageEvent) => {
      if (event.source !== frame.current?.contentWindow) return
      const data = event.data as { type?: string; blockId?: string | null; index?: number }
      if (data?.type === 'slides:select')
        onSelect?.(data.blockId ?? null, typeof data.index === 'number' ? data.index : index)
    }
    window.addEventListener('message', onMessage)
    return () => window.removeEventListener('message', onMessage)
  }, [interactive, onSelect, index])

  return (
    <div className={`sl-frame${className ? ` ${className}` : ''}${interactive ? '' : ' sl-frame-static'}`}>
      <iframe
        ref={frame}
        title={title ?? `Slide ${index + 1}`}
        srcDoc={html}
        sandbox="allow-scripts"
        tabIndex={-1}
        onLoad={() => {
          post({ type: 'slides:goto', index })
          post({ type: 'slides:highlight', blockId: selectedBlockId ?? null })
        }}
      />
    </div>
  )
}
