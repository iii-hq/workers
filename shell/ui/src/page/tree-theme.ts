/* The shell explorer and Codex both use @pierre/trees. Keep the shared
   wrapper aligned with Codex's review-file tree: compact rows, neutral
   filenames, sticky folders, and glyph-based git state. */

function statusMask(mark: string): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 20 20" fill="none"><rect x="3.333" y="3.333" width="13.334" height="13.334" rx="3.333" stroke="currentColor" stroke-width="1.333"/>${mark}</svg>`
  return `url("data:image/svg+xml,${encodeURIComponent(svg)}")`
}

const ADDED_MASK = statusMask(
  '<path d="M10 6.418C10.367 6.418 10.665 6.716 10.665 7.083V9.335H12.916C13.283 9.335 13.581 9.633 13.581 10C13.581 10.367 13.283 10.665 12.916 10.665H10.665V12.916C10.665 13.283 10.367 13.581 10 13.581C9.633 13.581 9.335 13.283 9.335 12.916V10.665H7.083C6.716 10.665 6.418 10.367 6.418 10C6.418 9.633 6.716 9.335 7.083 9.335H9.335V7.083C9.335 6.716 9.633 6.418 10 6.418Z" fill="currentColor"/>',
)
const DELETED_MASK = statusMask(
  '<path d="M12.916 9.335C13.283 9.335 13.581 9.633 13.581 10C13.581 10.367 13.283 10.665 12.916 10.665H7.083C6.716 10.665 6.418 10.367 6.418 10C6.418 9.633 6.716 9.335 7.083 9.335H12.916Z" fill="currentColor"/>',
)
const MODIFIED_MASK = statusMask(
  '<path d="M10 8.333C10.921 8.333 11.667 9.08 11.667 10C11.667 10.921 10.921 11.667 10 11.667C9.08 11.667 8.334 10.92 8.334 10C8.334 9.08 9.08 8.333 10 8.333Z" fill="currentColor"/>',
)

export const TREE_THEME: React.CSSProperties = {
  backgroundColor: 'var(--shui-explorer-bg, var(--color-panel-raised))',
  color: 'var(--color-ink)',
  width: '100%',
  '--shui-explorer-bg': 'var(--color-panel-raised)',
  '--shui-tree-file-muted': 'color-mix(in srgb, var(--color-ink-faint) 80%, var(--color-panel-raised))',
} as React.CSSProperties

export const TREE_UNSAFE_CSS = `
  :host {
    --trees-bg-override: var(--shui-explorer-bg, var(--color-panel-raised));
    --trees-bg-muted-override: var(--color-surface-hover);
    --trees-border-color-override: var(--color-edge);
    --trees-border-radius-override: 6px;
    --trees-fg-override: var(--color-ink);
    --trees-fg-muted-override: var(--color-ink-ghost);
    --trees-font-family-override: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    --trees-font-size-override: 13px;
    --trees-focus-ring-color-override: var(--color-rule-focus);
    --trees-git-added-color-override: light-dark(#199f43, #40c977);
    --trees-git-deleted-color-override: light-dark(#d52c36, #fa423e);
    --trees-git-ignored-color-override: var(--color-ink-ghost);
    --trees-git-lane-width-override: 20px;
    --trees-git-modified-color-override: light-dark(#d47628, #ff8549);
    --trees-git-renamed-color-override: light-dark(#d5a910, #ffd452);
    --trees-git-untracked-color-override: light-dark(#199f43, #40c977);
    --trees-indent-guide-bg-override: color-mix(in srgb, var(--color-ink) 10%, transparent);
    --trees-item-margin-x-override: 0px;
    --trees-item-padding-x-override: 6px;
    --trees-item-row-gap-override: 10px;
    --trees-level-gap-override: 0px;
    --trees-padding-inline-override: 0px;
    --trees-scrollbar-gutter-override: 0px;
    --trees-scrollbar-gutter-measured: 0px;
    --trees-selected-bg-override: var(--color-surface);
    --trees-selected-fg-override: var(--color-ink);
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

  [data-item-type='file'] {
    color: var(--shui-tree-file-muted, var(--color-ink-faint));
  }

  [data-item-type='file']:hover,
  [data-item-type='file'][aria-selected='true'] {
    color: var(--color-ink);
  }

  [data-item-type='file']:has([data-item-section='content']:empty) {
    display: none;
  }

  [data-item-git-status] > [data-item-section='content'] {
    color: inherit;
  }

  [data-item-git-status='added'] > [data-item-section='git'] > span,
  [data-item-git-status='deleted'] > [data-item-section='git'] > span,
  [data-item-git-status='modified'] > [data-item-section='git'] > span,
  [data-item-git-status='renamed'] > [data-item-section='git'] > span,
  [data-item-git-status='untracked'] > [data-item-section='git'] > span {
    font-size: 0;
  }

  [data-item-git-status='added'] > [data-item-section='git'] > span::before,
  [data-item-git-status='deleted'] > [data-item-section='git'] > span::before,
  [data-item-git-status='modified'] > [data-item-section='git'] > span::before,
  [data-item-git-status='renamed'] > [data-item-section='git'] > span::before,
  [data-item-git-status='untracked'] > [data-item-section='git'] > span::before {
    width: 20px;
    height: 20px;
    background-color: currentColor;
    content: '';
    mask: var(--shui-tree-git-status-icon) center / contain no-repeat;
  }

  [data-item-git-status='added'] > [data-item-section='git'] > span::before,
  [data-item-git-status='untracked'] > [data-item-section='git'] > span::before {
    --shui-tree-git-status-icon: ${ADDED_MASK};
  }

  [data-item-git-status='deleted'] > [data-item-section='git'] > span::before {
    --shui-tree-git-status-icon: ${DELETED_MASK};
  }

  [data-item-git-status='modified'] > [data-item-section='git'] > span::before,
  [data-item-git-status='renamed'] > [data-item-section='git'] > span::before {
    --shui-tree-git-status-icon: ${MODIFIED_MASK};
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
