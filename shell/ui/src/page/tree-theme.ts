/* Console-token theming for every @pierre/trees FileTree the page mounts
   (files tab, git changes tree). The tree renders into shadow DOM, so
   console classes can't reach it; the `--trees-*-override` custom
   properties inherit through the shadow boundary. */

export const TREE_THEME: React.CSSProperties = {
  '--trees-bg-override': 'transparent',
  '--trees-bg-muted-override': 'var(--color-surface-hover)',
  '--trees-fg-override': 'var(--color-ink)',
  '--trees-fg-muted-override': 'var(--color-ink-faint)',
  '--trees-accent-override': 'var(--color-accent)',
  '--trees-border-color-override': 'var(--color-rule-2)',
  '--trees-border-radius-override': '2px',
} as React.CSSProperties
