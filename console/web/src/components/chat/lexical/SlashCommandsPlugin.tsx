import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext'
import {
  LexicalTypeaheadMenuPlugin,
  MenuOption,
} from '@lexical/react/LexicalTypeaheadMenuPlugin'
import {
  $createTextNode,
  $getNodeByKey,
  $isTextNode,
  COMMAND_PRIORITY_NORMAL,
  type LexicalEditor,
  type TextNode,
} from 'lexical'
import { useCallback, useMemo, useState } from 'react'
import { createPortal } from 'react-dom'
import { getPrompt } from '@/lib/prompts'
import {
  fuzzyFilterSlash,
  loadPromptSlashCommands,
  SLASH_COMMANDS,
  type SlashCommand,
} from '@/lib/slash-commands'
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
// `-` is allowed because prompt names are kebab-case (`/blog-writer`).
const SLASH_PATTERN = /^\/([\w-]*)$/

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
  const [editor] = useLexicalComposerContext()
  const [query, setQuery] = useState<string | null>(null)
  const [promptCommands, setPromptCommands] = useState<SlashCommand[]>([])

  // The prompt store changes rarely; re-read each time the menu opens so a
  // save in the prompts library shows up without a reload.
  const refreshPromptCommands = useCallback(() => {
    void loadPromptSlashCommands().then(setPromptCommands)
  }, [])

  const options = useMemo(
    () =>
      fuzzyFilterSlash(query ?? '', [...SLASH_COMMANDS, ...promptCommands]).map(
        (entry) => new SlashCommandOption(entry),
      ),
    [query, promptCommands],
  )

  const onSelectOption = useCallback(
    (
      option: SlashCommandOption,
      textNodeContainingQuery: TextNode | null,
      closeMenu: () => void,
    ) => {
      const { entry } = option
      if (!textNodeContainingQuery) {
        closeMenu()
        return
      }
      if (!entry.promptName) {
        const replacement = $createTextNode(`${entry.command} `)
        textNodeContainingQuery.replace(replacement)
        replacement.selectEnd()
        closeMenu()
        return
      }
      // Prompt entry: inject the prompt BODY into the message (claude-code
      // style context injection). The body is fetched async, so drop a
      // placeholder now and swap it once the read lands.
      const placeholder = $createTextNode(`${entry.command} `)
      textNodeContainingQuery.replace(placeholder)
      placeholder.selectEnd()
      const key = placeholder.getKey()
      const name = entry.promptName
      closeMenu()
      void getPrompt(name).then((detail) => {
        editor.update(() => {
          const node = $getNodeByKey(key)
          if (!node || !detail) return
          // Swap only while the node still holds the untouched placeholder.
          // If the user kept typing into it while the read was pending, leave
          // their text alone rather than clobbering it.
          if (
            !$isTextNode(node) ||
            node.getTextContent() !== `${entry.command} `
          )
            return
          const body = $createTextNode(`${detail.body.trim()}\n`)
          node.replace(body)
          body.selectEnd()
        })
      })
    },
    [editor],
  )

  return (
    <LexicalTypeaheadMenuPlugin<SlashCommandOption>
      options={options}
      onQueryChange={setQuery}
      onSelectOption={onSelectOption}
      onOpen={() => {
        if (menuOpenRef) menuOpenRef.current = true
        refreshPromptCommands()
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
