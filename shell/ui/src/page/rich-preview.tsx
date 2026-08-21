import { Markdown } from '@iii-dev/console-ui'
import type React from 'react'
import { ImagePreview } from './image-preview'

export function isRichPreviewPath(path: string): boolean {
  const lower = path.toLowerCase()
  return (
    lower.endsWith('.html') ||
    lower.endsWith('.htm') ||
    lower.endsWith('.svg') ||
    lower.endsWith('.md') ||
    lower.endsWith('.markdown')
  )
}

export function richPreviewNode(path: string, contents: string): React.ReactNode | null {
  const lower = path.toLowerCase()
  if (lower.endsWith('.html') || lower.endsWith('.htm')) {
    return <iframe className="shui-rich-preview" title={`preview ${path}`} sandbox="" srcDoc={contents} />
  }
  if (lower.endsWith('.svg')) {
    const src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(contents)}`
    return <ImagePreview src={src} name={path} inline />
  }
  if (lower.endsWith('.md') || lower.endsWith('.markdown')) {
    return (
      <article className="shui-markdown-preview">
        <Markdown>{contents}</Markdown>
      </article>
    )
  }
  return null
}
