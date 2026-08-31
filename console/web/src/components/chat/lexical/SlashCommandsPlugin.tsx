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
import { useCallback, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { listSkills } from '@/lib/backend/directory-prompts'
import { getIiiClient } from '@/lib/iii-client'
import {
  fuzzyFilterSlash,
  mergeSlashEntries,
  SLASH_COMMANDS,
  type SlashCommand,
  setDynamicSlashEntries,
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

// Anchored at column 0 so `/` mid-sentence ("either/or") doesn't fire. The
// query is either a plain built-in slug or the `/skill:<id>` form (ids are
// `/`-separated paths) — a bare second `/`
// ("/home/…") disarms the palette at once instead of shadowing a typed path.
const SLASH_PATTERN = /^\/(skill:[\w./-]*|[\w-]*)$/

export function slashTriggerFn(text: string, _editor: LexicalEditor) {
  const match = text.match(SLASH_PATTERN)
  if (!match) return null
  const query = match[1] ?? ''
  if (
    !query.startsWith('skill:') &&
    !SLASH_COMMANDS.some((command) =>
      command.command.slice(1).startsWith(query),
    )
  )
    return null
  return {
    leadOffset: 0,
    matchingString: query,
    replaceableString: match[0],
  }
}

export function SlashCommandsPlugin({
  menuOpenRef,
}: SlashCommandsPluginProps = {}) {
  const [query, setQuery] = useState<string | null>(null)

  /* Directory-backed skills, refetched on EVERY open and published to the
     module registry that gates submit-time expansion. A failed or absent
     directory worker degrades to built-ins only. */
  const [dynamic, setDynamic] = useState<SlashCommand[] | null>(null)
  const loadingRef = useRef(false)
  const loadDynamic = useCallback(async () => {
    if (loadingRef.current) return
    loadingRef.current = true
    try {
      const client = await getIiiClient()
      const skills = await listSkills(client).catch(() => [])
      const entries = skills.map((s) => ({
        command: `/skill:${s.id}`,
        description: s.description || s.title,
      }))
      setDynamic(entries)
      setDynamicSlashEntries(entries)
    } catch {
      /* No client: keep whatever list the last open produced. */
    } finally {
      loadingRef.current = false
    }
  }, [])

  const options = useMemo(
    () =>
      fuzzyFilterSlash(query ?? '', mergeSlashEntries(dynamic ?? [])).map(
        (entry) => new SlashCommandOption(entry),
      ),
    [query, dynamic],
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
        void loadDynamic()
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
