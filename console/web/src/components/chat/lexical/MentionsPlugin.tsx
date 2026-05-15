import {
  LexicalTypeaheadMenuPlugin,
  MenuOption,
  useBasicTypeaheadTriggerMatch,
} from '@lexical/react/LexicalTypeaheadMenuPlugin'
import {
  $createTextNode,
  COMMAND_PRIORITY_NORMAL,
  type TextNode,
} from 'lexical'
import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { type FunctionEntry, fuzzyFilter } from '@/lib/functions'
import { cn } from '@/lib/utils'
import { $createFunctionMentionNode } from './FunctionMentionNode'

class FunctionMentionOption extends MenuOption {
  entry: FunctionEntry
  constructor(entry: FunctionEntry) {
    super(entry.id)
    this.entry = entry
  }
}

interface MentionsPluginProps {
  /** When set, this ref is flipped to true while the typeahead is visible
      so a sibling SubmitOnEnter plugin can skip its Enter handler. */
  menuOpenRef?: React.MutableRefObject<boolean>
}

interface FlipMenuProps {
  anchorEl: HTMLElement
  options: FunctionMentionOption[]
  selectedIndex: number | null
  selectOptionAndCleanUp: (option: FunctionMentionOption) => void
  setHighlightedIndex: (index: number) => void
}

/* Lexical positions the anchor div just below the caret. When the composer sits
   near the viewport bottom the menu would hang off the page (and grow body
   height, scrolling the page) because Lexical's flip-up branch only triggers
   when there's room above WITHIN the editor's root element — which our short
   composer never satisfies. We layer a `transform: translateY(...)` on top of
   Lexical's `style.top` so Lexical's repositioning keeps working while we
   override the placement when needed. */
function FlipMenu({
  anchorEl,
  options,
  selectedIndex,
  selectOptionAndCleanUp,
  setHighlightedIndex,
}: FlipMenuProps) {
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
        functions
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
              key={opt.entry.id}
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
              <span
                aria-hidden="true"
                className="text-accent font-semibold italic leading-none w-3 text-center shrink-0"
              >
                ƒ
              </span>
              <div className="min-w-0 flex flex-col">
                <span className="font-mono text-[13px] text-ink truncate">
                  {opt.entry.id}
                </span>
                <span className="font-mono text-[11px] text-ink-faint truncate lowercase">
                  {opt.entry.description}
                </span>
              </div>
            </div>
          )
        })}
      </div>
    </div>
  )
}

export function MentionsPlugin({ menuOpenRef }: MentionsPluginProps = {}) {
  const [query, setQuery] = useState<string | null>(null)

  const triggerFn = useBasicTypeaheadTriggerMatch('@', { minLength: 0 })

  const options = useMemo(
    () =>
      fuzzyFilter(query ?? '').map((entry) => new FunctionMentionOption(entry)),
    [query],
  )

  /* The typeahead plugin wraps this callback in editor.update() and passes us
     the TextNode currently holding "@<query>" (since shouldSplitNodeWithQuery
     is true inside the plugin). We just replace it with our mention node and
     append a trailing space so the caret lands cleanly after the pill. */
  const onSelectOption = useCallback(
    (
      option: FunctionMentionOption,
      textNodeContainingQuery: TextNode | null,
      closeMenu: () => void,
    ) => {
      if (textNodeContainingQuery) {
        const mention = $createFunctionMentionNode(option.entry.id)
        const trailing = $createTextNode(' ')
        textNodeContainingQuery.replace(mention)
        mention.insertAfter(trailing)
        trailing.selectEnd()
      }
      closeMenu()
    },
    [],
  )

  return (
    <LexicalTypeaheadMenuPlugin<FunctionMentionOption>
      options={options}
      onQueryChange={setQuery}
      onSelectOption={onSelectOption}
      onOpen={() => {
        if (menuOpenRef) menuOpenRef.current = true
      }}
      onClose={() => {
        if (menuOpenRef) menuOpenRef.current = false
      }}
      triggerFn={triggerFn}
      /* Run the typeahead's KEY_ENTER_COMMAND (and arrows/tab/escape) at NORMAL
         so it consumes Enter before our SubmitOnEnter handler at LOW. */
      commandPriority={COMMAND_PRIORITY_NORMAL}
      menuRenderFn={(anchorElementRef, props) => {
        if (!anchorElementRef.current || options.length === 0) return null
        return createPortal(
          <FlipMenu
            anchorEl={anchorElementRef.current}
            options={options}
            selectedIndex={props.selectedIndex}
            selectOptionAndCleanUp={props.selectOptionAndCleanUp}
            setHighlightedIndex={props.setHighlightedIndex}
          />,
          anchorElementRef.current,
        )
      }}
    />
  )
}
