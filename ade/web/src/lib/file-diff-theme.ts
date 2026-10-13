import { registerCustomCSSVariableTheme } from '@pierre/diffs'

export const FILE_DIFF_THEME = 'iii-console-diff'

// Strengthen secondary ink and literals against the nested red/green word
// highlights, while retaining the console's syntax roles and theme tokens.
const secondary =
  'color-mix(in srgb, var(--color-ink-faint) 60%, var(--color-ink))'
const literal = 'color-mix(in srgb, var(--color-accent) 60%, var(--color-ink))'

registerCustomCSSVariableTheme(FILE_DIFF_THEME, {
  foreground: 'var(--color-ink)',
  background: 'var(--color-bg)',
  'token-comment': secondary,
  'token-string': secondary,
  'token-string-expression': secondary,
  'token-punctuation': secondary,
  'token-constant': literal,
  'token-link': literal,
  'token-keyword': 'var(--color-ink)',
  'token-parameter': 'var(--color-ink)',
  'token-function': 'var(--color-ink)',
  'token-inserted': 'var(--color-ink)',
  'token-deleted': 'var(--color-ink)',
  'token-changed': 'var(--color-ink)',
})
