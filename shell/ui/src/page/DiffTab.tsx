/* One diff as a tab: a file against one source (staged, unstaged, a turn,
   a revision, a recorded change) in the console's FileDiff. The header
   carries the same breadcrumbs the editor shows, a chip naming the
   source, the +/- totals, the display options and the verbs that fit
   the source: stage / unstage / discard for the index, revert for a
   turn, a revision picker for compare, and "open the file" everywhere.
   Read-only: editing happens in the file tab. */

import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  FileDiff,
  IconButton,
  Selector,
} from '@iii-dev/console-ui'
import {
  Check,
  CircleAlert,
  FileStack,
  FileSymlink,
  FileX,
  Minus,
  MoreHorizontal,
  Plus,
  RefreshCw,
  Space,
  Undo2,
  WholeWord,
  WrapText,
} from 'lucide-react'
import { type ReactNode, useEffect, useMemo, useRef, useState } from 'react'
import { Breadcrumbs } from './Breadcrumbs'
import type { DiffContents } from './diff-load'
import { type DiffSource, diffSourceLabel, diffSourceSides } from './diff-source'
import { diffLines, diffTotals } from './diff'
import { firstChangedLine, gutterLineFromPath, resolveEditorLine } from './open-line'
import { PaneNotice } from './PaneNotice'
import { type WholeFileChange, wholeFileChange, wholeFileLabel } from './review-split'
import type { CompareRefs } from './use-compare-refs'

export interface DiffOptions {
  diffStyle: 'unified' | 'split'
  wordWrap: boolean
  wordDiffs: boolean
  hideWhitespace: boolean
  expandUnchanged: boolean
}

export const DEFAULT_DIFF_OPTIONS: DiffOptions = {
  diffStyle: 'unified',
  wordWrap: true,
  wordDiffs: true,
  hideWhitespace: false,
  expandUnchanged: false,
}

export type DiffTabState =
  | { phase: 'loading' }
  | { phase: 'error'; message: string }
  | { phase: 'ready'; contents: DiffContents }

/** Verbs the page offers for a source; each is optional so a tab in a
    folder without Git simply shows fewer buttons. */
export interface DiffTabActions {
  openFile?: (path: string, line?: number) => void
  stage?: () => void
  unstage?: () => void
  discard?: () => void
  revert?: () => void
  changeRef?: (ref: string) => void
}

interface DiffTabProps {
  rootLabel: string
  path: string
  source: DiffSource
  /** The turn's title when the source is a turn. */
  sourceTitle?: string
  state: DiffTabState
  options: DiffOptions
  onOptionsChange: (next: DiffOptions) => void
  onReload: () => void
  onRevealDir: (dir: string) => void
  actions: DiffTabActions
  compareRefs?: CompareRefs
  busy?: boolean
}

function normalizedForWhitespace(text: string): string {
  return text
    .split('\n')
    .map((line) => line.trim())
    .join('\n')
}

export function DiffTab({
  rootLabel,
  path,
  source,
  sourceTitle,
  state,
  options,
  onOptionsChange,
  onReload,
  onRevealDir,
  actions,
  compareRefs,
  busy = false,
}: DiffTabProps) {
  const [menuOpen, setMenuOpen] = useState(false)
  const bodyRef = useRef<HTMLDivElement>(null)
  const contents = state.phase === 'ready' ? state.contents : null
  const ops = useMemo(
    () =>
      contents && !contents.binary && !contents.noBaseline
        ? diffLines(
            options.hideWhitespace ? normalizedForWhitespace(contents.oldContents) : contents.oldContents,
            options.hideWhitespace ? normalizedForWhitespace(contents.newContents) : contents.newContents,
          )
        : null,
    [contents, options.hideWhitespace],
  )
  const totals = useMemo(() => (ops ? diffTotals(ops) : null), [ops])
  const wholeFile: WholeFileChange | null =
    contents && ops ? wholeFileChange(contents.oldContents, contents.newContents) : null
  const sides = diffSourceSides(source, sourceTitle)
  const label = diffSourceLabel(source, sourceTitle)

  // A fresh pair scrolls back to the top; a reload of the same pair stays.
  const scrollKey = `${path} ${label}`
  // biome-ignore lint/correctness/useExhaustiveDependencies: the key is the pair identity
  useEffect(() => {
    bodyRef.current?.scrollTo({ top: 0 })
  }, [scrollKey])

  const openFromGutter = (event: React.MouseEvent<HTMLElement>) => {
    if (!actions.openFile || !ops) return
    const target = gutterLineFromPath(event.nativeEvent.composedPath())
    if (target === null) return
    event.preventDefault()
    actions.openFile(path, resolveEditorLine(ops, target))
  }

  const canOpen = actions.openFile !== undefined && contents !== null && !contents.binary
  return (
    <div className="shui-main-pane shui-diff-tab" data-source={source.type}>
      <div className="shui-editor-head">
        <Breadcrumbs path={path} rootLabel={rootLabel} onSelectDir={onRevealDir} />
        <span className="shui-diff-chip" title={`${sides.old} to ${sides.new}`}>
          {label}
        </span>
        {totals ? (
          <span className="shui-diff-stats">
            <span className="add">+{totals.add}</span>
            <span className="del">-{totals.del}</span>
          </span>
        ) : null}
        <span className="spacer" />
        {source.type === 'compare' && compareRefs && actions.changeRef ? (
          <span className="shui-compare-picker">
            <Selector
              aria-label="Compare with revision"
              value={source.ref}
              groups={compareRefs.groups}
              loading={compareRefs.loading}
              error={compareRefs.error}
              onChange={actions.changeRef}
              onCreate={(query) => actions.changeRef?.(query.trim())}
              createOptionLabel={(query) => `Use revision "${query}"`}
              searchPlaceholder="Branch, tag, commit"
              emptyMessage="no matching revision"
              placeholder={label}
            />
          </span>
        ) : null}
        {actions.stage ? (
          <IconButton label="Stage changes" disabled={busy} onClick={actions.stage}>
            <Plus aria-hidden />
          </IconButton>
        ) : null}
        {actions.unstage ? (
          <IconButton label="Unstage changes" disabled={busy} onClick={actions.unstage}>
            <Minus aria-hidden />
          </IconButton>
        ) : null}
        {actions.discard ? (
          <IconButton label="Discard changes" disabled={busy} onClick={actions.discard}>
            <Undo2 aria-hidden />
          </IconButton>
        ) : null}
        {actions.revert ? (
          <IconButton label="Revert this file to before the turn" disabled={busy} onClick={actions.revert}>
            <Undo2 aria-hidden />
          </IconButton>
        ) : null}
        {canOpen ? (
          <IconButton
            label="Open the file (or click a line number)"
            onClick={() => actions.openFile?.(path, ops ? firstChangedLine(ops) : undefined)}
          >
            <FileSymlink aria-hidden />
          </IconButton>
        ) : null}
        <IconButton label="Reload" onClick={onReload} disabled={state.phase === 'loading'}>
          <RefreshCw aria-hidden />
        </IconButton>
        <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
          <DropdownMenuTrigger asChild>
            <IconButton label="Diff options" className={menuOpen ? 'active' : undefined}>
              <MoreHorizontal aria-hidden />
            </IconButton>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="shui-review-menu-content">
            <OptionRow
              label="Split view"
              icon={<FileStack />}
              checked={options.diffStyle === 'split'}
              onChange={(split) => onOptionsChange({ ...options, diffStyle: split ? 'split' : 'unified' })}
            />
            <OptionRow
              label="Word wrap"
              icon={<WrapText />}
              checked={options.wordWrap}
              onChange={(wordWrap) => onOptionsChange({ ...options, wordWrap })}
            />
            <DropdownMenuSeparator />
            <OptionRow
              label="Word diffs"
              icon={<WholeWord />}
              checked={options.wordDiffs}
              onChange={(wordDiffs) => onOptionsChange({ ...options, wordDiffs })}
            />
            <OptionRow
              label="Hide whitespace"
              icon={<Space />}
              checked={options.hideWhitespace}
              onChange={(hideWhitespace) => onOptionsChange({ ...options, hideWhitespace })}
            />
            <OptionRow
              label="Show the whole file"
              icon={<FileStack />}
              checked={options.expandUnchanged}
              onChange={(expandUnchanged) => onOptionsChange({ ...options, expandUnchanged })}
            />
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: gutter clicks inside the diff open the editor at that line */}
      {/* biome-ignore lint/a11y/useKeyWithClickEvents: the keyboard path is the "open the file" button in the header */}
      <div ref={bodyRef} className="shui-editor-body shui-diff-body" data-keybindings-standdown="" onClick={openFromGutter}>
        {state.phase === 'loading' ? (
          <div className="shui-side-note">loading diff</div>
        ) : state.phase === 'error' ? (
          <PaneNotice
            Icon={CircleAlert}
            tone="warn"
            title="This diff could not be loaded"
            path={path}
            detail={state.message}
            actions={
              <Button type="button" variant="ghost" size="sm" onClick={onReload}>
                <RefreshCw aria-hidden="true" />
                Try again
              </Button>
            }
          />
        ) : contents === null ? null : (
          <>
            {contents.note ? <div className="shui-review-message">{contents.note}</div> : null}
            {contents.noBaseline || contents.binary ? null : totals && totals.add === 0 && totals.del === 0 ? (
              <div className="shui-review-message">
                no line changes between {sides.old} and {sides.new}
              </div>
            ) : options.diffStyle === 'split' && wholeFile !== null ? (
              <WholeFileSplit change={wholeFile} lines={wholeFile === 'deleted' ? (totals?.del ?? 0) : (totals?.add ?? 0)}>
                <FileDiff
                  key="whole-file"
                  oldFile={{ name: path, contents: contents.oldContents }}
                  newFile={{ name: path, contents: contents.newContents }}
                  diffStyle="unified"
                  overflow={options.wordWrap ? 'wrap' : 'scroll'}
                  lineDiffType="none"
                  ignoreWhitespace={options.hideWhitespace}
                  expandUnchanged={options.expandUnchanged}
                  disableFileHeader
                  className="shui-review-diff"
                />
              </WholeFileSplit>
            ) : (
              <FileDiff
                key={options.diffStyle}
                oldFile={{ name: path, contents: contents.oldContents }}
                newFile={{ name: path, contents: contents.newContents }}
                diffStyle={options.diffStyle}
                overflow={options.wordWrap ? 'wrap' : 'scroll'}
                lineDiffType={options.wordDiffs ? 'word-alt' : 'none'}
                ignoreWhitespace={options.hideWhitespace}
                expandUnchanged={options.expandUnchanged}
                disableFileHeader
                className="shui-review-diff"
              />
            )}
          </>
        )}
      </div>
    </div>
  )
}

function OptionRow({
  label,
  icon,
  checked,
  onChange,
}: {
  label: string
  icon: ReactNode
  checked: boolean
  onChange: (checked: boolean) => void
}) {
  return (
    <DropdownMenuItem
      className="shui-review-option"
      role="menuitemcheckbox"
      aria-checked={checked}
      onSelect={(event) => {
        event.preventDefault()
        onChange(!checked)
      }}
    >
      <span className="menu-icon" aria-hidden>
        {icon}
      </span>
      <span>{label}</span>
      <span className="check" aria-hidden>
        {checked ? <Check /> : null}
      </span>
    </DropdownMenuItem>
  )
}

/** Split layout for a file that exists on one side only: the diff in that
    column, a placeholder in the other, so split never collapses to unified. */
function WholeFileSplit({ change, lines, children }: { change: WholeFileChange; lines: number; children: ReactNode }) {
  const label = wholeFileLabel(change, lines)
  const placeholder = (
    <div className="shui-whole-file-side">
      <div className="shui-whole-file-placeholder" role="status">
        <FileX aria-hidden />
        <span className="shui-whole-file-title">{label.title}</span>
        <span className="shui-whole-file-detail">{label.detail}</span>
      </div>
    </div>
  )
  return (
    <div className="shui-whole-file-split" data-change={change}>
      {change === 'deleted' ? children : placeholder}
      {change === 'deleted' ? placeholder : children}
    </div>
  )
}
