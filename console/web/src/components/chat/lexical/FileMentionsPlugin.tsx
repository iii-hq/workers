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
  useState,
} from 'react'
import { createPortal } from 'react-dom'
import type { FileSearchFn } from '@/lib/file-search'
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
import { FileGlyph, MentionRow, MoreRow } from './MentionRow'
import { useFileSearch } from './use-file-search'

type FileRowModel =
  | { kind: 'candidate'; candidate: MentionCandidate }
  | { kind: 'more'; remaining: number }

class FileMentionOption extends MenuOption {
  row: FileRowModel
  constructor(row: FileRowModel) {
    super(row.kind === 'more' ? 'more' : mentionKey(row.candidate))
    this.row = row
  }
}

/* `#` needs at least one character so markdown headings (`# foo` — the
   space ends the match) never flash the menu; the rest mirrors `@`. */
const HASH_PATTERN = /(^|\s|\()(#([^\s#()]{1,200}))$/

export function hashTriggerFn(text: string, _editor: LexicalEditor) {
  const match = HASH_PATTERN.exec(text)
  if (!match) return null
  return {
    leadOffset: match.index + match[1].length,
    matchingString: match[3],
    replaceableString: match[2],
  }
}

interface FileMentionsPluginProps {
  /** When set, this ref is flipped to true while the typeahead is visible
      so a sibling SubmitOnEnter plugin can skip its Enter handler. */
  menuOpenRef?: React.MutableRefObject<boolean>
  /** Files under the conversation's working directory. */
  searchFiles: FileSearchFn
  /** The composer card the menu aligns to. */
  frameRef?: RefObject<HTMLElement | null>
}

/**
 * `#` typeahead over the working directory's files and folders only — the
 * files half of the `@` menu, for people who reach for the old prefix.
 * Same worker search, same rows, same paging.
 */
export function FileMentionsPlugin({
  menuOpenRef,
  searchFiles,
  frameRef,
}: FileMentionsPluginProps) {
  const [query, setQuery] = useState<string | null>(null)
  const [page, setPage] = useState(0)
  const { files, loading } = useFileSearch(searchFiles, query)

  // biome-ignore lint/correctness/useExhaustiveDependencies: a new query starts over at the first page.
  useEffect(() => {
    setPage(0)
  }, [query])

  const options = useMemo(() => {
    const ranked = rankMentions(query ?? '', [], files)
    const { visible, remaining } = paginateMentions(ranked, page)
    const rows = visible.map(
      (candidate) => new FileMentionOption({ kind: 'candidate', candidate }),
    )
    if (remaining > 0) {
      rows.push(new FileMentionOption({ kind: 'more', remaining }))
    }
    return rows
  }, [files, query, page])

  const onSelectOption = useCallback(
    (
      option: FileMentionOption,
      textNodeContainingQuery: TextNode | null,
      closeMenu: () => void,
    ) => {
      if (option.row.kind === 'more') {
        setPage((current) => current + 1)
        return
      }
      if (textNodeContainingQuery && option.row.candidate.kind === 'file') {
        const mention = $createFileMentionNode(option.row.candidate.path)
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
      }}
      onClose={() => {
        if (menuOpenRef) menuOpenRef.current = false
      }}
      triggerFn={hashTriggerFn}
      /* Run the typeahead's KEY_ENTER_COMMAND (and arrows/tab/escape) at NORMAL
         so it consumes Enter before our SubmitOnEnter handler at LOW. */
      commandPriority={COMMAND_PRIORITY_NORMAL}
      menuRenderFn={(anchorElementRef, props) => {
        if (!anchorElementRef.current || options.length === 0) return null
        return createPortal(
          <FlipMenu
            anchorEl={anchorElementRef.current}
            frameEl={frameRef?.current ?? null}
            header="files & folders"
            footer={
              loading && files.length === 0 ? 'searching files…' : undefined
            }
            options={options}
            selectedIndex={props.selectedIndex}
            selectOptionAndCleanUp={props.selectOptionAndCleanUp}
            setHighlightedIndex={props.setHighlightedIndex}
            getOptionKey={(opt) => opt.key}
            renderOption={(opt) =>
              opt.row.kind === 'more' ? (
                <MoreRow remaining={opt.row.remaining} />
              ) : (
                <MentionRow
                  icon={
                    <FileGlyph
                      path={
                        opt.row.candidate.kind === 'file'
                          ? opt.row.candidate.path
                          : ''
                      }
                    />
                  }
                  name={
                    mentionName(opt.row.candidate) +
                    (opt.row.candidate.kind === 'file' &&
                    opt.row.candidate.isDir
                      ? '/'
                      : '')
                  }
                  detail={mentionDetail(opt.row.candidate)}
                />
              )
            }
          />,
          anchorElementRef.current,
        )
      }}
    />
  )
}
