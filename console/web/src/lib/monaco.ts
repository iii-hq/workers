/**
 * The console's single Monaco instance — loaded lazily (this module is only
 * ever `import()`ed, from `CodeEditor`) so the ~3 MiB editor chunk never
 * taxes the initial console load.
 *
 * Owns the two things every Monaco host must own exactly once:
 *
 * - the worker environment (Vite `?worker` bundles — no CDN, the embedded
 *   SPA must work fully offline), and
 * - the theme: one `iii-console` theme whose colors are resolved from the
 *   live design tokens (`--color-ink`, …) so Monaco follows
 *   `html[data-theme]` exactly like every token-styled surface. Monaco only
 *   accepts concrete hex colors, so tokens are normalized through a 1×1
 *   canvas (which also flattens `lab()`/`color-mix()` values) and the theme
 *   is re-defined on every theme flip by a module-level observer.
 */

import * as monaco from 'monaco-editor'
// Specifiers per monaco's `exports` map (`monaco-editor/*` → `esm/vs/*.js`).
import editorWorker from 'monaco-editor/editor/editor.worker.js?worker'
import cssWorker from 'monaco-editor/language/css/css.worker.js?worker'
import htmlWorker from 'monaco-editor/language/html/html.worker.js?worker'
import jsonWorker from 'monaco-editor/language/json/json.worker.js?worker'
import tsWorker from 'monaco-editor/language/typescript/ts.worker.js?worker'

export const CONSOLE_THEME = 'iii-console'

/** Mirrors `--font-mono` in index.css — used only if the token is unreadable. */
const FALLBACK_MONO =
  '"Geist Mono", "Chivo Mono", ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace'

self.MonacoEnvironment = {
  getWorker(_workerId: string, label: string) {
    switch (label) {
      case 'json':
        return new jsonWorker()
      case 'css':
      case 'scss':
      case 'less':
        return new cssWorker()
      case 'html':
      case 'handlebars':
      case 'razor':
        return new htmlWorker()
      case 'typescript':
      case 'javascript':
        return new tsWorker()
      default:
        return new editorWorker()
    }
  },
}

/**
 * Normalize any CSS color (`lab()`, `color-mix()`, hex, …) to `#rrggbb`.
 * `base` is composited under translucent values (the border tokens are
 * white-alpha in dark mode) so the readback is the color as actually seen,
 * not the un-premultiplied channel values.
 */
function toHex(css: string, fallback: string, base?: string): string {
  const canvas = document.createElement('canvas')
  canvas.width = 1
  canvas.height = 1
  const ctx = canvas.getContext('2d', { willReadFrequently: true })
  if (!ctx) return fallback
  if (base) {
    ctx.fillStyle = base
    ctx.fillRect(0, 0, 1, 1)
  }
  ctx.fillStyle = fallback
  ctx.fillStyle = css // invalid values keep the fallback
  ctx.fillRect(0, 0, 1, 1)
  const [r, g, b] = ctx.getImageData(0, 0, 1, 1).data
  return `#${[r, g, b].map((v) => v.toString(16).padStart(2, '0')).join('')}`
}

export function monoFontFamily(): string {
  const raw = getComputedStyle(document.documentElement)
    .getPropertyValue('--font-mono')
    .trim()
  return raw || FALLBACK_MONO
}

/**
 * (Re)define + apply the token-derived theme for the CURRENT `data-theme`.
 * Token rules mirror `syntaxTheme` (lib/syntax.tsx): monochrome ink with the
 * single accent hit reserved for literals — keys strongest, strings faint,
 * punctuation ghost.
 */
function syncTheme(): void {
  const root = document.documentElement
  const style = getComputedStyle(root)
  const dark = root.dataset.theme === 'dark'
  const bg = toHex(
    style.getPropertyValue('--color-bg').trim() ||
      (dark ? '#0a0a0a' : '#f2f0ed'),
    dark ? '#0a0a0a' : '#f2f0ed',
  )
  const token = (name: string, fallback: string) =>
    toHex(style.getPropertyValue(name).trim() || fallback, fallback, bg)

  const ink = token('--color-ink', dark ? '#ededed' : '#0a0a0a')
  const inkFaint = token('--color-ink-faint', dark ? '#a6a6a6' : '#6b6865')
  const inkGhost = token('--color-ink-ghost', dark ? '#6f6f6f' : '#a3a09c')
  const accent = token('--color-accent', dark ? '#28a8f7' : '#b8420f')
  const panel = token('--color-panel', dark ? '#111111' : '#fafafa')
  // rule is transparent in the no-lines system; composited over bg it
  // resolves to the background, so Monaco widget "borders" disappear too.
  const rule = token('--color-rule', dark ? '#0a0a0a' : '#f2f0ed')

  const t = (hex: string) => hex.slice(1)
  monaco.editor.defineTheme(CONSOLE_THEME, {
    base: dark ? 'vs-dark' : 'vs',
    inherit: false,
    rules: [
      { token: '', foreground: t(ink) },
      { token: 'comment', foreground: t(inkGhost), fontStyle: 'italic' },
      { token: 'string', foreground: t(inkFaint) },
      // JSON object keys carry the strongest ink, like Prism's `property`.
      { token: 'string.key.json', foreground: t(ink) },
      { token: 'string.value.json', foreground: t(inkFaint) },
      { token: 'number', foreground: t(accent), fontStyle: 'italic' },
      { token: 'keyword', foreground: t(accent), fontStyle: 'italic' },
      { token: 'constant', foreground: t(accent), fontStyle: 'italic' },
      { token: 'delimiter', foreground: t(inkGhost) },
      { token: 'operator', foreground: t(inkGhost) },
      { token: 'operators', foreground: t(inkGhost) },
      { token: 'tag', foreground: t(ink) },
      { token: 'attribute.name', foreground: t(inkFaint) },
      { token: 'type', foreground: t(ink) },
      { token: 'variable', foreground: t(ink) },
      { token: 'identifier', foreground: t(ink) },
      { token: 'strong', foreground: t(ink), fontStyle: 'bold' },
      { token: 'emphasis', foreground: t(ink), fontStyle: 'italic' },
    ],
    colors: {
      'editor.background': bg,
      'editor.foreground': ink,
      'editorCursor.foreground': ink,
      'editor.selectionBackground': `${accent}40`,
      'editor.inactiveSelectionBackground': `${accent}26`,
      'editor.lineHighlightBackground': '#00000000',
      'editorLineNumber.foreground': inkGhost,
      'editorLineNumber.activeForeground': inkFaint,
      'editorBracketMatch.background': `${accent}26`,
      'editorBracketMatch.border': `${accent}00`,
      'editorWidget.background': panel,
      'editorWidget.border': rule,
      'editorWidget.foreground': ink,
      'editorHoverWidget.background': panel,
      'editorHoverWidget.border': rule,
      'editorSuggestWidget.background': panel,
      'editorSuggestWidget.border': rule,
      'editorSuggestWidget.foreground': ink,
      'editorSuggestWidget.selectedBackground': `${accent}26`,
      'editorSuggestWidget.highlightForeground': accent,
      'input.background': bg,
      'input.foreground': ink,
      'input.border': rule,
      'scrollbarSlider.background': `${inkGhost}55`,
      'scrollbarSlider.hoverBackground': `${inkGhost}77`,
      'scrollbarSlider.activeBackground': `${inkGhost}99`,
      'editor.placeholder.foreground': inkGhost,
      focusBorder: '#00000000',
    },
  })
  monaco.editor.setTheme(CONSOLE_THEME)
}

syncTheme()
new MutationObserver(syncTheme).observe(document.documentElement, {
  attributes: true,
  attributeFilter: ['data-theme'],
})

export { monaco }
