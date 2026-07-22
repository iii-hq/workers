/**
 * Deep links from chat file-change cards into a local editor, URL-scheme
 * based (`cursor://file/...`) — the link opens on the machine running the
 * BROWSER. When that isn't where the files live (LAN browsing), the
 * dropdown's "copy path" is the fallback; there is deliberately no
 * backend involvement (spec: docs/superpowers/specs/
 * 2026-07-22-console-chat-copy-open-editor-design.md).
 */

export type EditorId = 'cursor' | 'vscode' | 'zed'

export interface Editor {
  id: EditorId
  label: string
  buildUrl: (path: string, line?: number) => string
}

/** `<scheme>://file<encoded-abs-path>[:<line>]` — the shape all three share. */
function fileUrl(scheme: string, path: string, line?: number): string {
  const anchor =
    typeof line === 'number' && Number.isInteger(line) && line > 0
      ? `:${line}`
      : ''
  // encodeURI leaves URL delimiters (#, ?) raw — legal filename bytes that
  // would truncate the path into a fragment/query — so encode per segment,
  // keeping only the slashes literal.
  const encoded = path.split('/').map(encodeURIComponent).join('/')
  return `${scheme}://file${encoded}${anchor}`
}

export const EDITORS: readonly Editor[] = [
  {
    id: 'cursor',
    label: 'cursor',
    buildUrl: (p, l) => fileUrl('cursor', p, l),
  },
  {
    id: 'vscode',
    label: 'vs code',
    buildUrl: (p, l) => fileUrl('vscode', p, l),
  },
  { id: 'zed', label: 'zed', buildUrl: (p, l) => fileUrl('zed', p, l) },
]

export function editorById(id: EditorId): Editor {
  // EDITORS covers every EditorId; the fallback keeps the type non-optional.
  return EDITORS.find((e) => e.id === id) ?? EDITORS[0]
}
