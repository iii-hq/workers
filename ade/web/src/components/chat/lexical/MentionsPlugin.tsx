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
import type { FileSearchFn } from '@/lib/file-search'
import type { FunctionEntry } from '@/lib/functions'
import {
  type MentionCandidate,
  mentionDetail,
  mentionKey,
  mentionName,
  paginateMentions,
  rankMentions,
} from '@/lib/mention-search'
import { $createFileMentionNode } from './FileMentionNode'
import { FlipMenu } from './FlipMenu'
import { $createFunctionMentionNode } from './FunctionMentionNode'
import { FileGlyph, FunctionGlyph, MentionRow, MoreRow } from './MentionRow'
import { useFileSearch } from './use-file-search'

type MentionRowModel =
  | { kind: 'candidate'; candidate: MentionCandidate }
  | { kind: 'more'; remaining: number }

class MentionOption extends MenuOption {
  row: MentionRowModel
  constructor(row: MentionRowModel) {
    super(row.kind === 'more' ? 'more' : mentionKey(row.candidate))
    this.row = row
  }
}

/* `@` after a start-of-line, whitespace or `(`, then anything that isn't
   whitespace, another `@` or a paren. Unlike Lexical's basic matcher this
   keeps `:`, `/`, `.` and `-` inside the query, because function ids
   (`shell::exec`) and paths (`src/a.ts`) are made of them. */
const AT_PATTERN = /(^|\s|\()(@([^\s@()]{0,200}))$/

export function atTriggerFn(text: string, _editor: LexicalEditor) {
  const match = AT_PATTERN.exec(text)
  if (!match) return null
  return {
    leadOffset: match.index + match[1].length,
    matchingString: match[3],
    replaceableString: match[2],
  }
}

interface MentionsPluginProps {
  /** When set, this ref is flipped to true while the typeahead is visible
      so a sibling SubmitOnEnter plugin can skip its Enter handler. */
  menuOpenRef?: React.MutableRefObject<boolean>
  functionEntries?: FunctionEntry[]
  /** Files under the conversation's working directory; absent = functions only. */
  searchFiles?: FileSearchFn
  /** The composer card the menu aligns to. */
  frameRef?: RefObject<HTMLElement | null>
}

/**
 * `@` typeahead over functions AND files together, one ranked list paged
 * ten rows at a time (a "show more" row reveals the next page without
 * closing the menu). Functions come from the catalog; files from the
 * worker's quick-open search under the working directory, fetched as the
 * query settles.
 */
export function MentionsPlugin({
  menuOpenRef,
  functionEntries = [],
  searchFiles,
  frameRef,
}: MentionsPluginProps = {}) {
  const [query, setQuery] = useState<string | null>(null)
  const [page, setPage] = useState(0)
  const { files, loading } = useFileSearch(searchFiles, query)
  /* Where the highlight should land after a page is revealed. */
  const revealAtRef = useRef<number | null>(null)

  // biome-ignore lint/correctness/useExhaustiveDependencies: a new query starts over at the first page.
  useEffect(() => {
    setPage(0)
  }, [query])

  const options = useMemo(() => {
    const ranked = rankMentions(query ?? '', functionEntries, files)
    const { visible, remaining } = paginateMentions(ranked, page)
    const rows = visible.map(
      (candidate) => new MentionOption({ kind: 'candidate', candidate }),
    )
    if (remaining > 0) rows.push(new MentionOption({ kind: 'more', remaining }))
    return rows
  }, [functionEntries, files, query, page])

  /* The typeahead plugin wraps this callback in editor.update() and passes us
     the TextNode currently holding "@<query>" (since shouldSplitNodeWithQuery
     is true inside the plugin). We replace it with the mention node and
     append a trailing space so the caret lands cleanly after the pill. The
     "more" row is the exception: it grows the list and keeps the menu. */
  const onSelectOption = useCallback(
    (
      option: MentionOption,
      textNodeContainingQuery: TextNode | null,
      closeMenu: () => void,
    ) => {
      if (option.row.kind === 'more') {
        revealAtRef.current = options.length - 1
        setPage((current) => current + 1)
        return
      }
      if (textNodeContainingQuery) {
        const { candidate } = option.row
        const mention =
          candidate.kind === 'function'
            ? $createFunctionMentionNode(candidate.id)
            : $createFileMentionNode(candidate.path)
        const trailing = $createTextNode(' ')
        textNodeContainingQuery.replace(mention)
        mention.insertAfter(trailing)
        trailing.selectEnd()
      }
      closeMenu()
    },
    [options.length],
  )

  return (
    <LexicalTypeaheadMenuPlugin<MentionOption>
      options={options}
      onQueryChange={setQuery}
      onSelectOption={onSelectOption}
      onOpen={() => {
        if (menuOpenRef) menuOpenRef.current = true
      }}
      onClose={() => {
        if (menuOpenRef) menuOpenRef.current = false
      }}
      triggerFn={atTriggerFn}
      /* Run the typeahead's KEY_ENTER_COMMAND (and arrows/tab/escape) at NORMAL
         so it consumes Enter before our SubmitOnEnter handler at LOW. */
      commandPriority={COMMAND_PRIORITY_NORMAL}
      menuRenderFn={(anchorElementRef, props) => {
        if (!anchorElementRef.current || options.length === 0) return null
        return createPortal(
          <MentionMenu
            anchorEl={anchorElementRef.current}
            frameEl={frameRef?.current ?? null}
            options={options}
            loading={loading && files.length === 0 && Boolean(searchFiles)}
            revealAtRef={revealAtRef}
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

interface MentionMenuProps {
  anchorEl: HTMLElement
  frameEl: HTMLElement | null
  options: MentionOption[]
  loading: boolean
  revealAtRef: React.MutableRefObject<number | null>
  selectedIndex: number | null
  selectOptionAndCleanUp: (option: MentionOption) => void
  setHighlightedIndex: (index: number) => void
}

function MentionMenu({
  anchorEl,
  frameEl,
  options,
  loading,
  revealAtRef,
  selectedIndex,
  selectOptionAndCleanUp,
  setHighlightedIndex,
}: MentionMenuProps) {
  /* After "show more" the highlight moves onto the first revealed row, so
     arrowing on continues down the list instead of jumping back up. */
  useEffect(() => {
    const at = revealAtRef.current
    if (at === null) return
    revealAtRef.current = null
    if (at < options.length) setHighlightedIndex(at)
  }, [options.length, setHighlightedIndex, revealAtRef])

  return (
    <FlipMenu
      anchorEl={anchorEl}
      frameEl={frameEl}
      footer={loading ? 'searching files…' : undefined}
      options={options}
      selectedIndex={selectedIndex}
      selectOptionAndCleanUp={selectOptionAndCleanUp}
      setHighlightedIndex={setHighlightedIndex}
      getOptionKey={(opt) => opt.key}
      renderOption={(opt) => renderMentionRow(opt.row)}
    />
  )
}

export function renderMentionRow(row: MentionRowModel) {
  if (row.kind === 'more') return <MoreRow remaining={row.remaining} />
  const { candidate } = row
  if (candidate.kind === 'function') {
    return (
      <MentionRow
        icon={<FunctionGlyph />}
        name={candidate.id}
        detail={candidate.description}
      />
    )
  }
  return (
    <MentionRow
      icon={<FileGlyph path={candidate.path} />}
      name={mentionName(candidate) + (candidate.isDir ? '/' : '')}
      detail={mentionDetail(candidate)}
    />
  )
}
