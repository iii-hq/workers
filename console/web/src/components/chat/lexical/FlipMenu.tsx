import type { MenuOption } from '@lexical/react/LexicalTypeaheadMenuPlugin'
import { type ReactNode, useLayoutEffect, useRef } from 'react'
import { cn } from '@/lib/utils'

/**
 * Shared typeahead dropdown chrome for the composer's `/`, `@`, and `#`
 * menus: the bordered listbox, the section header, row selection styling,
 * and the flip-up positioning fix.
 *
 * Lexical positions the anchor div just below the caret. When the composer
 * sits near the viewport bottom the menu would hang off the page (and grow
 * body height, scrolling the page) because Lexical's flip-up branch only
 * triggers when there's room above WITHIN the editor's root element — which
 * our short composer never satisfies. We layer a `transform: translateY(...)`
 * on top of Lexical's `style.top` so Lexical's repositioning keeps working
 * while we override the placement when needed.
 */

interface FlipMenuProps<T extends MenuOption> {
  anchorEl: HTMLElement
  /** Uppercased section label, e.g. "commands" / "functions" / "files". */
  header: string
  options: T[]
  selectedIndex: number | null
  selectOptionAndCleanUp: (option: T) => void
  setHighlightedIndex: (index: number) => void
  getOptionKey: (option: T) => string
  /** Inner row content (glyph + labels); the row shell is shared. */
  renderOption: (option: T) => ReactNode
}

export function FlipMenu<T extends MenuOption>({
  anchorEl,
  header,
  options,
  selectedIndex,
  selectOptionAndCleanUp,
  setHighlightedIndex,
  getOptionKey,
  renderOption,
}: FlipMenuProps<T>) {
  const menuRef = useRef<HTMLDivElement>(null)

  /* Re-run on options.length so the menu repositions when the option count
     changes (the menu's height shifts and the flip-up threshold may cross). */
  // biome-ignore lint/correctness/useExhaustiveDependencies: options.length is a trigger, not read in the effect body.
  useLayoutEffect(() => {
    const adjust = () => {
      const menu = menuRef.current
      if (!menu) return
      /* IMPORTANT: read layout (offsetTop/offsetHeight), not visual
         (getBoundingClientRect). The anchor's bounding rect reflects the
         transform we apply here, so using it would make the overflow check
         flip true/false on every MutationObserver fire and lock the UI. */
      const viewportTop = anchorEl.offsetTop - window.scrollY
      const menuHeight = menu.getBoundingClientRect().height
      const anchorHeight = anchorEl.offsetHeight
      const SAFE = 12
      const next =
        viewportTop + menuHeight > window.innerHeight - SAFE
          ? `translateY(-${menuHeight + anchorHeight + 8}px)`
          : ''
      /* Guard against the MutationObserver → setter → MutationObserver loop. */
      if (anchorEl.style.transform !== next) anchorEl.style.transform = next
    }
    adjust()
    const mo = new MutationObserver(adjust)
    mo.observe(anchorEl, { attributes: true, attributeFilter: ['style'] })
    const ro = new ResizeObserver(adjust)
    if (menuRef.current) ro.observe(menuRef.current)
    window.addEventListener('resize', adjust)
    document.addEventListener('scroll', adjust, true)
    return () => {
      mo.disconnect()
      ro.disconnect()
      window.removeEventListener('resize', adjust)
      document.removeEventListener('scroll', adjust, true)
      anchorEl.style.transform = ''
    }
  }, [anchorEl, options.length])

  return (
    <div
      ref={menuRef}
      className="border border-rule bg-bg w-[300px] max-h-[280px] overflow-y-auto shadow-none"
    >
      <div className="px-3 py-1.5 border-b border-rule-2 bg-panel font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint">
        {header}
      </div>
      {/* div+role over ul/li because keyboard navigation is driven by Lexical's
          typeahead controller (arrow keys, enter, escape), not by Tab. The
          listbox/option roles preserve the screen-reader semantics without
          dragging in semantic <ul>/<li> structure that Biome's a11y rules
          (correctly) treat as non-interactive. */}
      <div role="listbox" className="divide-y divide-rule-2">
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
                'flex items-center gap-2 px-3 py-2 cursor-pointer transition-colors',
                active
                  ? 'bg-panel border-l-2 border-l-accent pl-[10px]'
                  : 'border-l-2 border-l-transparent hover:bg-paper-2',
              )}
            >
              {renderOption(opt)}
            </div>
          )
        })}
      </div>
    </div>
  )
}
