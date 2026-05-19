import {
  LexicalTypeaheadMenuPlugin,
  MenuOption,
} from '@lexical/react/LexicalTypeaheadMenuPlugin'
import {
  $createTextNode,
  COMMAND_PRIORITY_NORMAL,
  type LexicalEditor,
  type TextNode,
} from 'lexical'
import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { fuzzyFilterSlash, type SlashCommand } from '@/lib/slash-commands'
import { cn } from '@/lib/utils'

class SlashCommandOption extends MenuOption {
  entry: SlashCommand
  constructor(entry: SlashCommand) {
    super(entry.command)
    this.entry = entry
  }
}

interface SlashCommandsPluginProps {
  /** True while the menu is open so SubmitOnEnter yields Enter to the typeahead. */
  menuOpenRef?: React.RefObject<boolean>
}

interface FlipMenuProps {
  anchorEl: HTMLElement
  options: SlashCommandOption[]
  selectedIndex: number | null
  selectOptionAndCleanUp: (option: SlashCommandOption) => void
  setHighlightedIndex: (index: number) => void
}

function FlipMenu({
  anchorEl,
  options,
  selectedIndex,
  selectOptionAndCleanUp,
  setHighlightedIndex,
}: FlipMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null)

  // biome-ignore lint/correctness/useExhaustiveDependencies: options.length is a trigger, not read in the effect body.
  useLayoutEffect(() => {
    const adjust = () => {
      const menu = menuRef.current
      if (!menu) return
      const viewportTop = anchorEl.offsetTop - window.scrollY
      const menuHeight = menu.getBoundingClientRect().height
      const anchorHeight = anchorEl.offsetHeight
      const SAFE = 12
      const next =
        viewportTop + menuHeight > window.innerHeight - SAFE
          ? `translateY(-${menuHeight + anchorHeight + 8}px)`
          : ''
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
        commands
      </div>
      <div role="listbox" className="divide-y divide-rule-2">
        {options.map((opt, i) => {
          const active = i === selectedIndex
          return (
            <div
              key={opt.entry.command}
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
                className="text-accent font-semibold leading-none w-3 text-center shrink-0"
              >
                /
              </span>
              <div className="min-w-0 flex flex-col">
                <span className="font-mono text-[13px] text-ink truncate">
                  {opt.entry.command}
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

// Anchored at column 0 so `/` mid-sentence ("either/or") doesn't fire.
const SLASH_PATTERN = /^\/(\w*)$/

function slashTriggerFn(text: string, _editor: LexicalEditor) {
  const match = text.match(SLASH_PATTERN)
  if (!match) return null
  return {
    leadOffset: 0,
    matchingString: match[1] ?? '',
    replaceableString: match[0],
  }
}

export function SlashCommandsPlugin({
  menuOpenRef,
}: SlashCommandsPluginProps = {}) {
  const [query, setQuery] = useState<string | null>(null)

  const options = useMemo(
    () =>
      fuzzyFilterSlash(query ?? '').map(
        (entry) => new SlashCommandOption(entry),
      ),
    [query],
  )

  const onSelectOption = useCallback(
    (
      option: SlashCommandOption,
      textNodeContainingQuery: TextNode | null,
      closeMenu: () => void,
    ) => {
      if (textNodeContainingQuery) {
        const replacement = $createTextNode(`${option.entry.command} `)
        textNodeContainingQuery.replace(replacement)
        replacement.selectEnd()
      }
      closeMenu()
    },
    [],
  )

  return (
    <LexicalTypeaheadMenuPlugin<SlashCommandOption>
      options={options}
      onQueryChange={setQuery}
      onSelectOption={onSelectOption}
      onOpen={() => {
        if (menuOpenRef) menuOpenRef.current = true
      }}
      onClose={() => {
        if (menuOpenRef) menuOpenRef.current = false
      }}
      triggerFn={slashTriggerFn}
      // NORMAL so typeahead consumes Enter before LOW-priority SubmitOnEnter.
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
