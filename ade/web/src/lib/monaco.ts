/**
 * The console's single Monaco instance — loaded lazily (this module is only
 * ever `import()`ed, from `CodeEditor`) so the ~3 MiB editor chunk never
 * taxes the initial console load.
 *
 * Owns the two things every Monaco host must own exactly once:
 *
 * - the worker environment (Vite `?worker` bundles — no CDN, the embedded
 *   SPA must work fully offline), and
 * - the theme: one `iii-console` theme — Vesper in dark, min-light in
 *   light — re-defined on every `html[data-theme]` flip by a module-level
 *   observer. Monaco themes are global, so every editor in the console
 *   wears it.
 */

import * as monaco from 'monaco-editor'
// Specifiers per monaco's `exports` map (`monaco-editor/*` → `esm/vs/*.js`).
import editorWorker from 'monaco-editor/editor/editor.worker.js?worker'
import cssWorker from 'monaco-editor/language/css/css.worker.js?worker'
import htmlWorker from 'monaco-editor/language/html/html.worker.js?worker'
import jsonWorker from 'monaco-editor/language/json/json.worker.js?worker'
import tsWorker from 'monaco-editor/language/typescript/ts.worker.js?worker'

export const CONSOLE_THEME = 'iii-console'

/** Mirrors `--font-code` in index.css — used only if the token is unreadable. */
const FALLBACK_CODE =
  'Monaco, "SFMono-Regular", "SF Mono", Menlo, Consolas, "Liberation Mono", "Courier New", monospace'

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

export function codeFontFamily(): string {
  const raw = getComputedStyle(document.documentElement)
    .getPropertyValue('--font-code')
    .trim()
  return raw || FALLBACK_CODE
}

interface EditorPalette {
  base: 'vs' | 'vs-dark'
  bg: string
  fg: string
  muted: string
  comment: string
  string: string
  literal: string
  /** Types, tags, function-ish names: Vesper's peach, min-light's purple. */
  entity: string
  keyword: string
  delimiter: string
  operator: string
  tag: string
  attribute: string
  cssProperty: string
  cssValue: string
  jsonKey: string
  invalid: string
  warning: string
  /** Markdown bold. */
  strong: string
  lineNumber: string
  lineNumberActive: string
  lineBorder: string
  /** Neutral on purpose: selection never takes a hue (selection-conformance). */
  selection: string
  selectionInactive: string
  bracketMatch: string
  bracketMatchBorder: string
  widget: string
  widgetBorder: string
  listSelected: string
  listHover: string
  input: string
  scrollbar: string
  scrollbarHover: string
}

/* Ports of Rauno Freiberg's Vesper (dark) and miguelsolorio's min-light
   (light), the same pair the static code panes wear (`.code-theme-editor`
   in index.css). Monaco tokenizes with Monarch, not TextMate, so each
   theme's scopes are mapped onto Monarch token names. Comments take the
   panes' strengthened greys (≥ 4.5:1) rather than the originals' faint
   ones. */
const VESPER: EditorPalette = {
  base: 'vs-dark',
  bg: '#101010',
  fg: '#ffffff',
  muted: '#a0a0a0',
  comment: '#8b8b8b',
  string: '#99ffe4',
  literal: '#ffc799',
  entity: '#ffc799',
  keyword: '#a0a0a0',
  delimiter: '#a0a0a0',
  operator: '#a0a0a0',
  tag: '#ffc799',
  attribute: '#ffc799',
  cssProperty: '#ffffff',
  cssValue: '#a0a0a0',
  jsonKey: '#ffc799',
  invalid: '#ff8080',
  warning: '#ffc799',
  strong: '#ffffff',
  lineNumber: '#505050',
  lineNumberActive: '#a0a0a0',
  lineBorder: '#1c1c1c',
  selection: '#ffffff25',
  selectionInactive: '#ffffff12',
  bracketMatch: '#ffffff1a',
  bracketMatchBorder: '#00000000',
  widget: '#161616',
  widgetBorder: '#282828',
  listSelected: '#232323',
  listHover: '#282828',
  input: '#1c1c1c',
  scrollbar: '#34343480',
  scrollbarHover: '#343434',
}

const MIN_LIGHT: EditorPalette = {
  base: 'vs',
  bg: '#ffffff',
  fg: '#24292e',
  muted: '#757575',
  comment: '#8b949e',
  string: '#22863a',
  literal: '#1976d2',
  entity: '#6f42c1',
  keyword: '#d32f2f',
  delimiter: '#24292e',
  operator: '#d32f2f',
  tag: '#22863a',
  attribute: '#6f42c1',
  cssProperty: '#1976d2',
  cssValue: '#1976d2',
  jsonKey: '#d32f2f',
  invalid: '#cd3131',
  warning: '#cd9731',
  strong: '#6f42c1',
  lineNumber: '#cccccc',
  lineNumberActive: '#757575',
  lineBorder: '#f2f2f2',
  selection: '#e0e0e0',
  selectionInactive: '#eeeeee',
  bracketMatch: '#e7f3ff',
  bracketMatchBorder: '#c8e1ff',
  widget: '#f3f3f3',
  widgetBorder: '#e9e9e9',
  listSelected: '#e0e0e0',
  listHover: '#eeeeee',
  input: '#ffffff',
  scrollbar: '#0000001a',
  scrollbarHover: '#00000033',
}

/**
 * (Re)define + apply the editor theme for the CURRENT `data-theme`:
 * Vesper in dark, min-light in light.
 */
function syncTheme(): void {
  const p =
    document.documentElement.dataset.theme === 'dark' ? VESPER : MIN_LIGHT
  const t = (hex: string) => hex.slice(1)
  monaco.editor.defineTheme(CONSOLE_THEME, {
    base: p.base,
    inherit: false,
    rules: [
      { token: '', foreground: t(p.fg) },
      { token: 'comment', foreground: t(p.comment), fontStyle: 'italic' },
      { token: 'string', foreground: t(p.string) },
      { token: 'string.escape', foreground: t(p.muted) },
      {
        token: 'string.link',
        foreground: t(p.literal),
        fontStyle: 'underline',
      },
      { token: 'regexp', foreground: t(p.string) },
      { token: 'number', foreground: t(p.literal) },
      { token: 'constant', foreground: t(p.literal) },
      { token: 'predefined', foreground: t(p.literal) },
      { token: 'keyword', foreground: t(p.keyword) },
      { token: 'type', foreground: t(p.entity) },
      { token: 'annotation', foreground: t(p.entity) },
      { token: 'identifier', foreground: t(p.fg) },
      { token: 'variable', foreground: t(p.fg) },
      { token: 'delimiter', foreground: t(p.delimiter) },
      { token: 'operator', foreground: t(p.operator) },
      { token: 'metatag', foreground: t(p.muted) },
      { token: 'tag', foreground: t(p.tag) },
      { token: 'attribute.name', foreground: t(p.attribute) },
      { token: 'attribute.value', foreground: t(p.string) },
      { token: 'invalid', foreground: t(p.invalid) },
      // CSS: selectors read as tags, properties and values per theme.
      { token: 'tag.css', foreground: t(p.tag) },
      { token: 'attribute.name.css', foreground: t(p.cssProperty) },
      { token: 'attribute.value.css', foreground: t(p.cssValue) },
      { token: 'attribute.value.number.css', foreground: t(p.literal) },
      { token: 'attribute.value.unit.css', foreground: t(p.literal) },
      { token: 'attribute.value.hex.css', foreground: t(p.fg) },
      { token: 'tag.scss', foreground: t(p.tag) },
      { token: 'attribute.name.scss', foreground: t(p.cssProperty) },
      { token: 'attribute.value.scss', foreground: t(p.cssValue) },
      // JSON / YAML keys, and their literals.
      { token: 'string.key.json', foreground: t(p.jsonKey) },
      { token: 'string.value.json', foreground: t(p.string) },
      { token: 'keyword.json', foreground: t(p.literal) },
      { token: 'type.yaml', foreground: t(p.jsonKey) },
      // Markdown.
      { token: 'keyword.md', foreground: t(p.entity), fontStyle: 'bold' },
      { token: 'strong', foreground: t(p.strong), fontStyle: 'bold' },
      { token: 'emphasis', foreground: t(p.fg), fontStyle: 'italic' },
    ],
    colors: {
      'editor.background': p.bg,
      'editor.foreground': p.fg,
      'editorCursor.foreground': p.fg,
      'editor.selectionBackground': p.selection,
      'editor.inactiveSelectionBackground': p.selectionInactive,
      'editor.selectionHighlightBackground': p.selectionInactive,
      'editor.lineHighlightBackground': '#00000000',
      'editor.lineHighlightBorder': p.lineBorder,
      'editorLineNumber.foreground': p.lineNumber,
      'editorLineNumber.activeForeground': p.lineNumberActive,
      'editorBracketHighlight.foreground1': p.muted,
      'editorBracketHighlight.foreground2': p.muted,
      'editorBracketHighlight.foreground3': p.muted,
      'editorBracketHighlight.foreground4': p.muted,
      'editorBracketHighlight.foreground5': p.muted,
      'editorBracketHighlight.foreground6': p.muted,
      'editorBracketHighlight.unexpectedBracket.foreground': p.invalid,
      'editorBracketMatch.background': p.bracketMatch,
      'editorBracketMatch.border': p.bracketMatchBorder,
      'editorError.foreground': p.invalid,
      'editorWarning.foreground': p.warning,
      'editorWidget.background': p.widget,
      'editorWidget.border': p.widgetBorder,
      'editorWidget.foreground': p.fg,
      'editorHoverWidget.background': p.widget,
      'editorHoverWidget.border': p.widgetBorder,
      'editorSuggestWidget.background': p.widget,
      'editorSuggestWidget.border': p.widgetBorder,
      'editorSuggestWidget.foreground': p.fg,
      'editorSuggestWidget.selectedBackground': p.listSelected,
      'editorSuggestWidget.highlightForeground': p.literal,
      'list.hoverBackground': p.listHover,
      'input.background': p.input,
      'input.foreground': p.fg,
      'input.border': p.widgetBorder,
      'scrollbarSlider.background': p.scrollbar,
      'scrollbarSlider.hoverBackground': p.scrollbarHover,
      'scrollbarSlider.activeBackground': p.scrollbarHover,
      'editor.placeholder.foreground': p.comment,
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
