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
import { useCallback, useMemo, useState } from 'react'
import { createPortal } from 'react-dom'
import { type FunctionEntry, fuzzyFilter } from '@/lib/functions'
import { FlipMenu } from './FlipMenu'
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
  functionEntries?: FunctionEntry[]
}

export function MentionsPlugin({
  menuOpenRef,
  functionEntries = [],
}: MentionsPluginProps = {}) {
  const [query, setQuery] = useState<string | null>(null)

  const triggerFn = useBasicTypeaheadTriggerMatch('@', { minLength: 0 })

  const options = useMemo(
    () =>
      fuzzyFilter(functionEntries, query ?? '').map(
        (entry) => new FunctionMentionOption(entry),
      ),
    [functionEntries, query],
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
            header="functions"
            options={options}
            selectedIndex={props.selectedIndex}
            selectOptionAndCleanUp={props.selectOptionAndCleanUp}
            setHighlightedIndex={props.setHighlightedIndex}
            getOptionKey={(opt) => opt.entry.id}
            renderOption={(opt) => (
              <>
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
              </>
            )}
          />,
          anchorElementRef.current,
        )
      }}
    />
  )
}
