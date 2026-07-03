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
import { useCallback, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { fetchFileIndex, fuzzyFilterFiles } from '@/lib/file-index'
import { $createFileMentionNode } from './FileMentionNode'
import { FlipMenu } from './FlipMenu'

class FileMentionOption extends MenuOption {
  path: string
  constructor(path: string) {
    super(path)
    this.path = path
  }
}

interface FileMentionsPluginProps {
  /** When set, this ref is flipped to true while the typeahead is visible
      so a sibling SubmitOnEnter plugin can skip its Enter handler. */
  menuOpenRef?: React.MutableRefObject<boolean>
  /** Jail anchor for the file index; the menu is inert without one. */
  workingDir: string
}

/**
 * `#` typeahead over the working directory's files. The index is fetched
 * lazily on first open (and re-used through file-index's TTL cache), so a
 * conversation that never mentions files never walks the repo. `minLength: 1`
 * keeps markdown headings (`# foo` — the space ends the match) from flashing
 * the menu.
 */
export function FileMentionsPlugin({
  menuOpenRef,
  workingDir,
}: FileMentionsPluginProps) {
  const [query, setQuery] = useState<string | null>(null)
  const [index, setIndex] = useState<string[]>([])
  /* Guards a slow fetch resolving after the working dir changed. */
  const workingDirRef = useRef(workingDir)
  workingDirRef.current = workingDir

  const triggerFn = useBasicTypeaheadTriggerMatch('#', { minLength: 1 })

  const options = useMemo(
    () =>
      fuzzyFilterFiles(index, query ?? '').map(
        (path) => new FileMentionOption(path),
      ),
    [index, query],
  )

  const loadIndex = useCallback(() => {
    const dir = workingDir
    fetchFileIndex(dir)
      .then((paths) => {
        if (workingDirRef.current === dir) setIndex(paths)
      })
      .catch(() => {
        /* Worker away or dir invalid: the menu simply stays empty. */
      })
  }, [workingDir])

  const onSelectOption = useCallback(
    (
      option: FileMentionOption,
      textNodeContainingQuery: TextNode | null,
      closeMenu: () => void,
    ) => {
      if (textNodeContainingQuery) {
        const mention = $createFileMentionNode(option.path)
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
    <LexicalTypeaheadMenuPlugin<FileMentionOption>
      options={options}
      onQueryChange={setQuery}
      onSelectOption={onSelectOption}
      onOpen={() => {
        if (menuOpenRef) menuOpenRef.current = true
        loadIndex()
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
            header="files"
            options={options}
            selectedIndex={props.selectedIndex}
            selectOptionAndCleanUp={props.selectOptionAndCleanUp}
            setHighlightedIndex={props.setHighlightedIndex}
            getOptionKey={(opt) => opt.path}
            renderOption={(opt) => (
              <>
                <span
                  aria-hidden="true"
                  className="text-accent leading-none w-3 flex justify-center shrink-0"
                >
                  <svg
                    width="9"
                    height="11"
                    viewBox="0 0 10 12"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1"
                    aria-hidden="true"
                  >
                    <path d="M1 1H6L9 4V11H1V1Z" />
                    <path d="M6 1V4H9" />
                  </svg>
                </span>
                <span className="min-w-0 font-mono text-[13px] text-ink truncate">
                  {opt.path}
                </span>
              </>
            )}
          />,
          anchorElementRef.current,
        )
      }}
    />
  )
}
