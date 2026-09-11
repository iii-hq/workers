import type { MenuOption } from '@lexical/react/LexicalTypeaheadMenuPlugin'
import { type ReactNode, useLayoutEffect, useRef } from 'react'
import { cn } from '@/lib/utils'

/**
 * Shared typeahead dropdown chrome for the composer's `/`, `@`, and `#`
 * menus: the listbox, the optional section header, the one-line rows with
 * their selection styling, and the placement.
 *
 * Lexical positions its anchor div just below the caret and re-sets its
 * `style.top/left` on every scroll, resize and edit. Rather than fight
 * that, we layer a `transform` on the anchor so Lexical's repositioning
 * keeps working while we decide where the menu really goes:
 *
 * - With a `frameEl` (the composer card) the menu takes the card's width
 *   and left edge and hangs just above it — or below it when the card
 *   sits near the top of the viewport — so a long list never covers the
 *   text being typed and always reads as part of the composer.
 * - Without one it stays caret-anchored and only flips above the caret
 *   when it would otherwise hang off the bottom of the page (Lexical's own
 *   flip needs room WITHIN the editor root, which a short composer never
 *   has).
 */

interface FlipMenuProps<T extends MenuOption> {
  anchorEl: HTMLElement
  /** The composer card: the menu takes its width and left edge. */
  frameEl?: HTMLElement | null
  /** Uppercased section label, e.g. "commands" / "files". Omit for none. */
  header?: string
  /** A quiet status line under the rows ("searching files…"). */
  footer?: ReactNode
  options: T[]
  selectedIndex: number | null
  selectOptionAndCleanUp: (option: T) => void
  setHighlightedIndex: (index: number) => void
  getOptionKey: (option: T) => string
  /** Inner row content (glyph + labels); the row shell is shared. */
  renderOption: (option: T) => ReactNode
}

const GAP = 8
const SAFE = 12

export function FlipMenu<T extends MenuOption>({
  anchorEl,
  frameEl,
  header,
  footer,
  options,
  selectedIndex,
  selectOptionAndCleanUp,
  setHighlightedIndex,
  getOptionKey,
  renderOption,
}: FlipMenuProps<T>) {
  const menuRef = useRef<HTMLDivElement>(null)

  /* Re-run on options.length so the menu repositions when the option count
     changes (the menu's height shifts and the flip threshold may cross). */
  // biome-ignore lint/correctness/useExhaustiveDependencies: options.length is a trigger, not read in the effect body.
  useLayoutEffect(() => {
    const adjust = () => {
      const menu = menuRef.current
      if (!menu) return
      /* IMPORTANT: read the anchor's layout (offsetTop/offsetLeft), not its
         visual rect. The bounding rect reflects the transform we apply
         here, so using it would make the placement flip on every
         MutationObserver fire and lock the UI. */
      const anchorTop = anchorEl.offsetTop - window.scrollY
      const anchorLeft = anchorEl.offsetLeft - window.scrollX
      const anchorHeight = anchorEl.offsetHeight
      let next = ''
      if (frameEl) {
        const frame = frameEl.getBoundingClientRect()
        const width = `${Math.round(frame.width)}px`
        if (menu.style.width !== width) menu.style.width = width
        const menuHeight = menu.offsetHeight
        const above = frame.top - GAP - menuHeight
        const top = above >= SAFE ? above : frame.bottom + GAP
        const dx = Math.round(frame.left - anchorLeft)
        const dy = Math.round(top - anchorTop)
        next = `translate(${dx}px, ${dy}px)`
      } else {
        const menuHeight = menu.getBoundingClientRect().height
        next =
          anchorTop + menuHeight > window.innerHeight - SAFE
            ? `translateY(-${menuHeight + anchorHeight + GAP}px)`
            : ''
      }
      /* Guard against the MutationObserver → setter → MutationObserver loop. */
      if (anchorEl.style.transform !== next) anchorEl.style.transform = next
    }
    adjust()
    const mo = new MutationObserver(adjust)
    mo.observe(anchorEl, { attributes: true, attributeFilter: ['style'] })
    const ro = new ResizeObserver(adjust)
    if (menuRef.current) ro.observe(menuRef.current)
    if (frameEl) ro.observe(frameEl)
    window.addEventListener('resize', adjust)
    document.addEventListener('scroll', adjust, true)
    return () => {
      mo.disconnect()
      ro.disconnect()
      window.removeEventListener('resize', adjust)
      document.removeEventListener('scroll', adjust, true)
      anchorEl.style.transform = ''
    }
  }, [anchorEl, frameEl, options.length])

  return (
    <div
      ref={menuRef}
      className="flex w-[300px] max-h-[min(50vh,352px)] flex-col overflow-hidden rounded-md bg-panel-raised shadow-floating"
    >
      {header ? (
        <div className="shrink-0 bg-paper-2 px-3 py-1 font-mono text-[10px] uppercase tracking-[0.18em] text-ink-faint">
          {header}
        </div>
      ) : null}
      {/* div+role over ul/li because keyboard navigation is driven by Lexical's
          typeahead controller (arrow keys, enter, escape), not by Tab. The
          listbox/option roles preserve the screen-reader semantics without
          dragging in semantic <ul>/<li> structure that Biome's a11y rules
          (correctly) treat as non-interactive. */}
      <div role="listbox" className="min-h-0 overflow-y-auto py-1">
        {options.map((opt, i) => {
          const active = i === selectedIndex
          return (
            <div
              key={getOptionKey(opt)}
              role="option"
              tabIndex={-1}
              aria-selected={active}
              ref={(el) => opt.setRefElement(el)}
              onMouseEnter={() => setHighlightedIndex(i)}
              onMouseDown={(e) => {
                e.preventDefault()
                selectOptionAndCleanUp(opt)
              }}
              className={cn(
                'mx-1 flex h-7 cursor-pointer items-center gap-2 rounded-sm px-2 transition-colors',
                active ? 'bg-surface-selected' : 'hover:bg-surface-hover',
              )}
            >
              {renderOption(opt)}
            </div>
          )
        })}
      </div>
      {footer ? (
        <div className="shrink-0 border-t border-rule-2 px-3 py-1 font-mono text-[10px] text-ink-ghost">
          {footer}
        </div>
      ) : null}
    </div>
  )
}
