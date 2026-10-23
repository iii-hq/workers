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
import {
  MentionGlyph,
  mentionColor,
  mentionIcon,
} from '@/components/chat/mentions/appearance'
import type { FileSearchFn } from '@/lib/file-search'
import type { FunctionEntry } from '@/lib/functions'
import {
  mentionDetail,
  mentionKey,
  mentionName,
  paginateMentions,
  rankMentions,
} from '@/lib/mention-search'
import {
  buildGlobalMenu,
  buildScopedMenu,
  GLOBAL_PROVIDER_LIMIT,
  type MentionMenuEntry,
  type MentionMenuRow,
  PROVIDER_MIN_QUERY,
  parseScopedQuery,
  SCOPED_PROVIDER_ROWS,
  scopedText,
  showsSectionHeaders,
} from '@/lib/mentions/menu'
import {
  loadMentionProviders,
  useMentionProviders,
} from '@/lib/mentions/providers'
import { useMentionSearch } from '@/lib/mentions/search'
import type { MentionSearchContext } from '@/lib/mentions/types'
import { primeMentionView } from '@/lib/mentions/views'
import { $createFileMentionNode } from './FileMentionNode'
import { FlipMenu, type MenuSection } from './FlipMenu'
import { $createFunctionMentionNode } from './FunctionMentionNode'
import {
  EntityRow,
  FileGlyph,
  FunctionGlyph,
  MentionRow,
  MoreRow,
  ProviderMoreRow,
  ProviderRow,
} from './MentionRow'
import { useFileSearch } from './use-file-search'
import { $createWorkerMentionNode } from './WorkerMentionNode'

class MentionOption extends MenuOption {
  entry: MentionMenuEntry
  constructor(entry: MentionMenuEntry) {
    super(entry.key)
    this.entry = entry
  }
}

/* `@` after a start-of-line, whitespace or `(`, then anything that isn't
   whitespace, another `@` or a paren. Unlike Lexical's basic matcher this
   keeps `:`, `/`, `.` and `-` inside the query, because function ids
   (`shell::exec`) and paths (`src/a.ts`) are made of them. */
const AT_PATTERN = /(^|\s|\()(@([^\s@()]{0,200}))$/

/* `@<provider>:<text>` scopes the menu to one worker's mentions; the text
   may hold spaces (titles do). A second colon (`@kanban::ticket::get`) is a
   function id, never a scope. Whether a provider by that name exists is the
   plugin's call — this only recognises the shape. */
const SCOPED_PATTERN =
  /(^|\s|\()(@([a-z][a-z0-9-]{0,39}:(?!:)[^\n@()]{0,120}))$/

export function atTriggerFn(text: string, _editor: LexicalEditor) {
  const match = SCOPED_PATTERN.exec(text) ?? AT_PATTERN.exec(text)
  if (!match) return null
  return {
    leadOffset: match.index + match[1].length,
    matchingString: match[3],
    replaceableString: match[2],
  }
}

interface MentionsPluginProps {
  /** When set, true while the menu shows options, so the composer's Enter
      and arrow handlers yield those keys to the typeahead. */
  menuOpenRef?: React.MutableRefObject<boolean>
  functionEntries?: FunctionEntry[]
  /** Files under the conversation's working directory; absent = functions only. */
  searchFiles?: FileSearchFn
  /** The composer card the menu aligns to. */
  frameRef?: RefObject<HTMLElement | null>
  /** Where worker mention searches are asked from. */
  mentionContext?: MentionSearchContext
}

/**
 * The `@` typeahead.
 *
 * - `@text`: functions and files in one ranked list paged ten rows at a
 *   time (a "show more" row reveals the next page without closing the
 *   menu), plus the workers that define mentions — the ones whose name
 *   matches first, as sources to drill into, and from two characters on a
 *   group per worker with its best items.
 * - `@<worker>:text` (typed, or reached with Tab/Enter on a source): that
 *   worker's own search. Picking an item inserts `@<worker>(id="…")`.
 */
export function MentionsPlugin({
  menuOpenRef,
  functionEntries = [],
  searchFiles,
  frameRef,
  mentionContext,
}: MentionsPluginProps = {}) {
  const [query, setQuery] = useState<string | null>(null)
  const [page, setPage] = useState(0)
  const providers = useMentionProviders()
  const scoped = useMemo(
    () => (query === null ? null : parseScopedQuery(query, providers.byName)),
    [query, providers.byName],
  )
  // Files only belong to the global menu.
  const { files, loading: filesLoading } = useFileSearch(
    searchFiles,
    scoped ? null : query,
  )
  const providerResults = useMentionSearch({
    providers: scoped ? [scoped.provider] : providers.providers,
    query: scoped ? scoped.query : query,
    limit: scoped ? SCOPED_PROVIDER_ROWS : GLOBAL_PROVIDER_LIMIT,
    context: mentionContext,
    minQueryLength: scoped ? 0 : PROVIDER_MIN_QUERY,
  })
  /* Where the highlight should land after a page is revealed. */
  const revealAtRef = useRef<number | null>(null)
  const isOpen = query !== null

  // biome-ignore lint/correctness/useExhaustiveDependencies: a new query starts over at the first page.
  useEffect(() => {
    setPage(0)
  }, [query])

  // A menu opening is the moment a stale provider list should refresh.
  useEffect(() => {
    if (isOpen) void loadMentionProviders()
  }, [isOpen])

  const options = useMemo(() => {
    const entries = scoped
      ? buildScopedMenu(scoped, providerResults)
      : (() => {
          const ranked = rankMentions(query ?? '', functionEntries, files)
          const { visible, remaining } = paginateMentions(ranked, page)
          return buildGlobalMenu({
            query: query ?? '',
            providers: providers.providers,
            catalog: visible,
            catalogRemaining: remaining,
            catalogKey: mentionKey,
            results: providerResults,
          })
        })()
    return entries.map((entry) => new MentionOption(entry))
  }, [
    scoped,
    providerResults,
    query,
    functionEntries,
    files,
    page,
    providers.providers,
  ])

  const sectioned = useMemo(
    () => showsSectionHeaders(options.map((o) => o.entry)),
    [options],
  )

  /* The trigger fires on any `@query`, so the typeahead counts as open on
     text that matches nothing ("thanks @bob"). Only a menu that shows
     options may claim Enter, or the message could not be sent; a list that
     fills in after a search lands claims it without a keystroke. A scoped
     `@worker:` menu always claims it: Enter there means "pick", never
     "send the half-typed search". Written only while this menu is open, so
     it never clears another menu's claim. */
  const openRef = useRef(false)
  useEffect(() => {
    if (menuOpenRef && openRef.current) {
      menuOpenRef.current = options.length > 0 || scoped !== null
    }
  }, [options, scoped, menuOpenRef])

  /* The typeahead plugin wraps this callback in editor.update() and passes
     the TextNode holding "@<query>" (shouldSplitNodeWithQuery is true inside
     the plugin). An item becomes its pill plus a trailing space; a source
     (or "more …") rewrites the query to `@<worker>:` and keeps the menu, as
     the "show more" row keeps it to grow the list. */
  const onSelectOption = useCallback(
    (
      option: MentionOption,
      textNodeContainingQuery: TextNode | null,
      closeMenu: () => void,
    ) => {
      const { row } = option.entry
      if (row.kind === 'more') {
        revealAtRef.current = options.findIndex((o) => o.entry.row === row)
        setPage((current) => current + 1)
        return
      }
      if (row.kind === 'provider' || row.kind === 'provider-more') {
        if (textNodeContainingQuery) {
          const text = scopedText(
            row.provider,
            row.kind === 'provider-more' ? row.query : '',
          )
          textNodeContainingQuery.setTextContent(text)
          textNodeContainingQuery.select(text.length, text.length)
        }
        return
      }
      if (textNodeContainingQuery) {
        const mention =
          row.kind === 'entity'
            ? $createWorkerMentionNode(row.provider.name, row.item.id)
            : row.candidate.kind === 'function'
              ? $createFunctionMentionNode(row.candidate.id)
              : $createFileMentionNode(row.candidate.path)
        if (row.kind === 'entity') primeMentionView(row.provider.name, row.item)
        const trailing = $createTextNode(' ')
        textNodeContainingQuery.replace(mention)
        mention.insertAfter(trailing)
        trailing.selectEnd()
      }
      closeMenu()
    },
    [options],
  )

  const providerSearching = [...providerResults.values()].some(
    (result) => result.loading && result.items.length === 0,
  )
  const footer = scoped
    ? providerSearching
      ? `searching ${scoped.provider.label.toLowerCase()}…`
      : options.length === 0
        ? providerResults.get(scoped.provider.name)?.failed
          ? `${scoped.provider.name} did not answer`
          : `no ${scoped.provider.label.toLowerCase()} match`
        : undefined
    : filesLoading && files.length === 0 && searchFiles
      ? 'searching files…'
      : providerSearching && (query?.trim().length ?? 0) >= PROVIDER_MIN_QUERY
        ? 'searching mentions…'
        : undefined

  return (
    <LexicalTypeaheadMenuPlugin<MentionOption>
      options={options}
      onQueryChange={setQuery}
      onSelectOption={onSelectOption}
      onOpen={() => {
        openRef.current = true
        if (menuOpenRef)
          menuOpenRef.current = options.length > 0 || scoped !== null
      }}
      onClose={() => {
        openRef.current = false
        if (menuOpenRef) menuOpenRef.current = false
      }}
      triggerFn={atTriggerFn}
      /* Run the typeahead's KEY_ENTER_COMMAND (and arrows/tab/escape) at NORMAL
         so it consumes Enter before the composer's send, which listens at
         the front of LOW. */
      commandPriority={COMMAND_PRIORITY_NORMAL}
      menuRenderFn={(anchorElementRef, props) => {
        if (!anchorElementRef.current) return null
        // A scoped menu shows even while empty: it says what it is doing.
        if (options.length === 0 && !scoped) return null
        return createPortal(
          <MentionMenu
            anchorEl={anchorElementRef.current}
            frameEl={frameRef?.current ?? null}
            options={options}
            header={
              scoped
                ? `@${scoped.provider.name} · ${scoped.provider.label.toLowerCase()}`
                : undefined
            }
            sectioned={sectioned && !scoped}
            footer={footer}
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
  header?: string
  sectioned: boolean
  footer?: string
  revealAtRef: React.MutableRefObject<number | null>
  selectedIndex: number | null
  selectOptionAndCleanUp: (option: MentionOption) => void
  setHighlightedIndex: (index: number) => void
}

function MentionMenu({
  anchorEl,
  frameEl,
  options,
  header,
  sectioned,
  footer,
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
    if (at >= 0 && at < options.length) setHighlightedIndex(at)
  }, [options.length, setHighlightedIndex, revealAtRef])

  return (
    <FlipMenu
      anchorEl={anchorEl}
      frameEl={frameEl}
      header={header}
      footer={footer}
      options={options}
      selectedIndex={selectedIndex}
      selectOptionAndCleanUp={selectOptionAndCleanUp}
      setHighlightedIndex={setHighlightedIndex}
      getOptionKey={(opt) => opt.key}
      getOptionSection={sectioned ? (opt) => menuSection(opt.entry) : undefined}
      renderOption={(opt) => renderMentionRow(opt.entry.row)}
    />
  )
}

function menuSection(entry: MentionMenuEntry): MenuSection {
  const { section } = entry
  const provider = section.provider
  return {
    key: section.key,
    label: section.label,
    icon: provider ? (
      <MentionGlyph
        icon={mentionIcon(provider.icon)}
        color={mentionColor(provider.color)}
        className="size-4"
      />
    ) : undefined,
  }
}

export function renderMentionRow(row: MentionMenuRow) {
  switch (row.kind) {
    case 'more':
      return <MoreRow remaining={row.remaining} />
    case 'provider':
      return <ProviderRow provider={row.provider} />
    case 'provider-more':
      return <ProviderMoreRow provider={row.provider} />
    case 'entity':
      return <EntityRow provider={row.provider} item={row.item} />
    case 'candidate': {
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
  }
}
