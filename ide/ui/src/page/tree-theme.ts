/* The shell explorer and Codex both use @pierre/trees. The wrapper is
   themed the way the reference sidebar reads: roomy 24px rows with the
   name 8px off its glyph, a 12px step per level with an indent guide,
   full-width hover and selection fills (the selected row wears a rail on
   its outer edge), file names tinted by their git state and a small dot
   in the lane at the right for a changed file or a folder holding one. */

export const TREE_THEME: React.CSSProperties = {
  backgroundColor: 'var(--shui-explorer-bg, var(--color-panel-raised))',
  color: 'var(--color-ink)',
  width: '100%',
  // The sidebar's canvas: the tree sits on the same black as the pane around it.
  '--shui-explorer-bg': 'var(--color-bg)',
} as React.CSSProperties

/* The six glyph tones the design system allows beside a label, by icon
   token: the same map `.shui-file-type-icon` applies to the tabs and
   breadcrumbs, so one file wears one colour everywhere. Anything unlisted
   stays faint. yml sits in purple, not rose: a red glyph beside a name
   reads as a deleted file. */
const ICON_TONES: Record<string, readonly string[]> = {
  // Token names stay literal: the build checks every `--color-*` it finds.
  '--color-glyph-blue': ['biome', 'c', 'cpp', 'docker', 'python', 'typescript', 'vscode', 'webpack'],
  '--color-glyph-purple': ['astro', 'database', 'vite', 'bootstrap', 'css', 'eslint', 'terraform', 'wasm', 'bun', 'yml'],
  '--color-glyph-teal': ['go', 'react', 'tailwind', 'mcp', 'prettier', 'table'],
  '--color-glyph-green': ['bash', 'markdown', 'svgo', 'vue'],
  '--color-glyph-amber': ['babel', 'browserslist', 'javascript', 'claude', 'html', 'json', 'rust', 'svg', 'swift', 'zig', 'zip'],
  '--color-glyph-rose': ['graphql', 'image', 'sass', 'npm', 'postcss', 'ruby', 'svelte'],
}
const ICON_TONE_VARS = Object.entries(ICON_TONES)
  .flatMap(([tone, tokens]) => tokens.map((token) => `    --trees-file-icon-color-${token}: var(${tone});`))
  .join('\n')

export const TREE_UNSAFE_CSS = `
  :host {
    /* Icons tint through the library's per-token variables (set above). */
    --trees-bg-override: var(--shui-explorer-bg, var(--color-panel-raised));
    --trees-bg-muted-override: var(--color-surface-hover);
    --trees-border-color-override: var(--color-edge);
    --trees-border-radius-override: 0px;
    --trees-fg-override: var(--color-ink);
    --trees-fg-muted-override: var(--color-ink-faint);
    --trees-font-family-override: var(--font-sans);
    --trees-font-size-override: 13px;
    --trees-focus-ring-color-override: var(--color-rule-focus);
    --trees-git-added-color-override: var(--color-ok);
    --trees-git-deleted-color-override: var(--color-alert);
    --trees-git-ignored-color-override: var(--color-ink-ghost);
    --trees-git-lane-width-override: 16px;
    --trees-git-modified-color-override: var(--color-warn);
    --trees-git-renamed-color-override: var(--color-ok);
    --trees-git-untracked-color-override: var(--color-ok);
    --trees-indent-guide-bg-override: color-mix(in srgb, var(--color-ink) 14%, transparent);
    --trees-item-margin-x-override: 0px;
    --trees-item-padding-x-override: 12px;
    --trees-item-row-gap-override: 8px;
    --trees-level-gap-override: 12px;
    --trees-padding-inline-override: 0px;
    --trees-scrollbar-gutter-override: 0px;
    --trees-scrollbar-gutter-measured: 0px;
    --trees-selected-bg-override: var(--color-surface-selected);
    --trees-selected-fg-override: var(--color-ink);
    --trees-selected-focused-border-color-override: transparent;
${ICON_TONE_VARS}
  }

  [data-file-tree-sticky-overlay-content='true'],
  [data-file-tree-sticky-row='true'] {
    background-color: var(--shui-explorer-bg, var(--color-panel-raised));
  }

  [data-file-tree-virtualized-scroll='true'] {
    scrollbar-gutter: auto;
  }

  [role='treeitem'],
  [role='treeitem'] * {
    cursor: pointer !important;
  }

  /* Files and folders share one ink: a dimmed file name reads as disabled,
     and the file is what the reader came to click. */
  [data-item-type='file'] {
    color: var(--color-ink);
  }

  /* The chevron is a folder's only glyph: quiet until the row is hovered
     or open, as in the reference. */
  [data-item-type='folder'] > [data-item-section='icon'] {
    color: var(--color-ink-faint);
  }

  /* The selected row runs edge to edge and wears a 2px rail on the
     sidebar's outer edge; keyboard focus draws the console's ring. */
  [data-type='item'][data-item-selected='true']::after {
    content: '';
    position: absolute;
    inset: 0 auto 0 0;
    width: 2px;
    background: color-mix(in srgb, var(--color-ink) 45%, transparent);
    pointer-events: none;
  }

  /* The row a context menu was opened on, and the rows a folder's actions
     reach, while the menu is open (FilesTab marks them): the neutral
     selected fill with its edge rail, never the accent. */
  [data-type='item'][data-shui-menu='target'] {
    background-color: var(--color-surface-selected);
    outline: 1px solid var(--color-edge);
    outline-offset: -1px;
  }

  [data-type='item'][data-shui-menu='scope'] {
    background-color: var(--color-surface-hover);
  }

  [data-item-type='file']:has([data-item-section='content']:empty) {
    display: none;
  }

  /* Git state: a small dot in the lane at the right, for a changed file
     (its own colour) and for a folder holding one (the modified tone, as
     the library marks it). The library's letter and 6px icon are replaced
     by the one dot so files and folders read alike. */
  [data-item-git-status] > [data-item-section='git'] > span,
  [data-item-contains-git-change='true'] > [data-item-section='git'] > span {
    font-size: 0;
  }

  [data-item-git-status] > [data-item-section='git'] > span > svg,
  [data-item-contains-git-change='true'] > [data-item-section='git'] > span > svg {
    display: none;
  }

  [data-item-git-status='added'] > [data-item-section='git'] > span::before,
  [data-item-git-status='deleted'] > [data-item-section='git'] > span::before,
  [data-item-git-status='modified'] > [data-item-section='git'] > span::before,
  [data-item-git-status='renamed'] > [data-item-section='git'] > span::before,
  [data-item-git-status='untracked'] > [data-item-section='git'] > span::before,
  [data-item-contains-git-change='true'] > [data-item-section='git'] > span::before {
    content: '';
    width: 7px;
    height: 7px;
    border-radius: 9999px;
    background-color: currentColor;
  }

  [data-item-contains-git-change='true'] > [data-item-section='git'] {
    opacity: 0.7;
  }

  [data-item-git-status='ignored'] > [data-item-section='content'] {
    color: var(--color-ink-faint);
  }

  [data-type='item'][data-item-focused='true']::before {
    outline-color: transparent;
  }

  [data-type='item']:focus-visible::before {
    outline-color: var(--color-rule-focus);
  }

  @container measure (height <= calc(1lh + 1px)) {
    [data-truncate-marker] {
      opacity: 0;
    }
  }
`
