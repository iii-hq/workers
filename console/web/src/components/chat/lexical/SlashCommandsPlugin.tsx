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
import { useCallback, useMemo, useState } from 'react'
import { createPortal } from 'react-dom'
import { fuzzyFilterSlash, type SlashCommand } from '@/lib/slash-commands'
import { FlipMenu } from './FlipMenu'

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
            header="commands"
            options={options}
            selectedIndex={props.selectedIndex}
            selectOptionAndCleanUp={props.selectOptionAndCleanUp}
            setHighlightedIndex={props.setHighlightedIndex}
            getOptionKey={(opt) => opt.entry.command}
            renderOption={(opt) => (
              <>
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
              </>
            )}
          />,
          anchorElementRef.current,
        )
      }}
    />
  )
}
