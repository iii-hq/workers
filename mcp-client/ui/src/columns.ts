import type { KeyboardEvent } from 'react'

/** The marks of a selected row: `data-selected` is what the shared list's roving Tab stop and row
 * style read, `aria-current` what assistive tech reads. No listbox roles: a row may hold a button. */
export const currentRow = (selected: boolean) => ({
  'data-selected': selected || undefined,
  'aria-current': selected || undefined,
})

const FOCUSABLE =
  'a[href], button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), summary, [tabindex]:not([tabindex="-1"])'

/** Focus column `n` (`data-column`): a list's selected row, else its first; otherwise the column's
 * first focusable element. False when there is nothing to focus. */
export function focusColumn(root: Element | null, n: number): boolean {
  const column = root?.querySelector<HTMLElement>(`[data-column="${n}"]`)
  const target = column?.matches('[data-list-root]')
    ? (column.querySelector<HTMLElement>('[data-list-item][data-selected]') ??
      column.querySelector<HTMLElement>('[data-list-item]'))
    : column?.querySelector<HTMLElement>(FOCUSABLE)
  target?.focus()
  return Boolean(target)
}

/** A column list's own key handler: Left and Right on a row move focus to the neighbouring column,
 * mirrored when the sidebar sits on the right. A text field, or a row's own button, keeps its arrows. */
export function onColumnKeys(event: KeyboardEvent<HTMLElement>): void {
  if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return
  const target = event.target as HTMLElement
  if (target.closest('input, textarea, [contenteditable]') || !target.matches('[data-list-item]')) return
  const root = event.currentTarget.closest<HTMLElement>('.mcp-ui-root')
  const forward = event.key === (root?.dataset.side === 'right' ? 'ArrowLeft' : 'ArrowRight')
  if (focusColumn(root, Number(event.currentTarget.dataset.column) + (forward ? 1 : -1))) event.preventDefault()
}
