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
import {
  type RefObject,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { createPortal } from 'react-dom'
import { listSkills } from '@/lib/backend/directory-prompts'
import { getIiiClient } from '@/lib/iii-client'
import {
  fuzzyFilterSlash,
  mergeSlashEntries,
  type SlashCommand,
  setDynamicSlashEntries,
  slashCommandLabel,
} from '@/lib/slash-commands'
import { FlipMenu } from './FlipMenu'
import { MentionRow } from './MentionRow'
import { $createSlashCommandNode } from './SlashCommandNode'

class SlashCommandOption extends MenuOption {
  entry: SlashCommand
  constructor(entry: SlashCommand) {
    super(entry.command)
    this.entry = entry
  }
}

interface SlashCommandsPluginProps {
  /** True while the menu shows options so SubmitOnEnter yields Enter to the typeahead. */
  menuOpenRef?: React.RefObject<boolean>
  /** The composer card the menu aligns to. */
  frameRef?: RefObject<HTMLElement | null>
}

/* `/` after a start-of-line, whitespace or `(` — the same boundary as `@`
   — so a command can be picked mid-sentence while "either/or" never fires.
   The query is a plain slug or the `/skill:<id>` form (ids are
   `/`-separated paths); a bare second `/` ("/home/…") disarms the palette
   at once instead of shadowing a typed path. */
const SLASH_PATTERN = /(^|\s|\()(\/(skill:[\w./-]*|[\w-]*))$/

export function slashTriggerFn(text: string, _editor: LexicalEditor) {
  const match = SLASH_PATTERN.exec(text)
  if (!match) return null
  return {
    leadOffset: match.index + match[1].length,
    matchingString: match[2].slice(1),
    replaceableString: match[2],
  }
}

export function SlashCommandsPlugin({
  menuOpenRef,
  frameRef,
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

  /* The trigger fires on any `/slug`, so the typeahead counts as open on
     text that matches nothing ("hello /foo"). Only a menu that actually
     shows options may claim Enter — otherwise the message could never be
     sent — so the flag follows the option count, not the trigger. A list
     that fills in after the fetch lands flips it on without a keystroke. */
  const openRef = useRef(false)
  useEffect(() => {
    if (menuOpenRef) menuOpenRef.current = openRef.current && options.length > 0
  }, [options, menuOpenRef])

  /* The typeahead plugin wraps this callback in editor.update() and passes us
     the TextNode currently holding "/<query>". We replace it with the command
     pill and append a trailing space so the caret lands cleanly after it. */
  const onSelectOption = useCallback(
    (
      option: SlashCommandOption,
      textNodeContainingQuery: TextNode | null,
      closeMenu: () => void,
    ) => {
      if (textNodeContainingQuery) {
        const command = $createSlashCommandNode(option.entry.command)
        const trailing = $createTextNode(' ')
        textNodeContainingQuery.replace(command)
        command.insertAfter(trailing)
        trailing.selectEnd()
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
        openRef.current = true
        if (menuOpenRef) menuOpenRef.current = options.length > 0
        void loadDynamic()
      }}
      onClose={() => {
        openRef.current = false
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
            frameEl={frameRef?.current ?? null}
            header="commands"
            options={options}
            selectedIndex={props.selectedIndex}
            selectOptionAndCleanUp={props.selectOptionAndCleanUp}
            setHighlightedIndex={props.setHighlightedIndex}
            getOptionKey={(opt) => opt.entry.command}
            renderOption={(opt) => (
              <MentionRow
                icon={<span className="font-semibold leading-none">/</span>}
                name={slashCommandLabel(opt.entry.command)}
                detail={opt.entry.description}
              />
            )}
          />,
          anchorElementRef.current,
        )
      }}
    />
  )
}
