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

export function isHtmlPath(path: string): boolean {
  const lower = path.toLowerCase()
  return lower.endsWith('.html') || lower.endsWith('.htm')
}

const SCRIPT_TAG = /<script\b/i
// A resource the page pulls in (not a link it navigates to), by its URL.
const RESOURCE_REF = /<(?:script|link|img|source|video|audio|iframe|embed)\b[^>]*?\b(?:src|href)\s*=\s*["']?([^"'\s>]+)/gi
const REMOTE_REF = /^(?:[a-z][a-z0-9+.-]*:|\/\/|#)/i

/** What the HTML preview cannot show of this document, as a sentence, or
    null when it renders whole. The preview is a sandboxed `srcdoc` frame:
    scripts do not run, and a relative or root path has no folder to
    resolve against, so only inline markup, inline styles and remote
    resources appear. */
export function htmlPreviewLimits(contents: string): string | null {
  const scripts = SCRIPT_TAG.test(contents)
  let local = false
  for (const match of contents.matchAll(RESOURCE_REF)) {
    if (!REMOTE_REF.test(match[1] ?? '')) {
      local = true
      break
    }
  }
  if (scripts && local) return "Static preview: scripts don't run and linked local files don't load, so parts of the page may be missing."
  if (scripts) return "Static preview: scripts don't run, so anything they draw is missing."
  if (local) return "Static preview: linked local files, like stylesheets and images, don't load."
  return null
}

export function richPreviewNode(path: string, contents: string): React.ReactNode | null {
  const lower = path.toLowerCase()
  if (isHtmlPath(lower)) {
    const limits = htmlPreviewLimits(contents)
    return (
      <div className="shui-html-preview">
        {limits !== null ? (
          <p className="shui-html-preview-note" role="note">
            {limits}
          </p>
        ) : null}
        <iframe className="shui-rich-preview" title={`preview ${path}`} sandbox="" srcDoc={contents} />
      </div>
    )
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
