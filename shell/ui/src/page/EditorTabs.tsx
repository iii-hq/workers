/* The editor tab strip: file tabs and diff tabs side by side with VS
   Code's semantics and chrome. A file tab shows its type icon and the
   name coloured by Git status; a diff tab shows the same name with a
   small chip naming what it compares (Staged, Changes, a turn, a
   revision), so two diffs of one file read apart at a glance. Preview
   tabs are italic, a dirty file shows a dot where the close button sits,
   middle-click closes, the wheel scrolls the strip, and a right-click
   menu carries the usual close verbs plus copy path, reveal and compare.
   The terminal, when docked in the editor area, is one more tab. */

import { GitCompareArrows, SquareTerminal, X } from 'lucide-react'
import { useCallback, useEffect, useRef } from 'react'
import { anchorFromEvent, type ContextMenuItem, useContextMenu } from './ContextMenu'
import { diffSourceLabel } from './diff-source'
import { FileTypeIcon } from './file-type-icon'
import type { GitFileStatus } from './git'
import { HoverTip } from './HoverTip'
import { basename } from './paths'
import type { OpenTab, TabsState } from './tabs'

export interface EditorTabsTerminal {
  title: string
  active: boolean
  onActivate: () => void
  onClose: () => void
}

interface EditorTabsProps {
  tabs: TabsState
  /** File paths with unsaved edits. */
  dirtyPaths: ReadonlySet<string>
  /** File paths whose file is gone from disk (`missing-files.ts`). */
  missingPaths?: ReadonlySet<string>
  /** False while the main pane shows the terminal: no tab reads selected. */
  tabVisible: boolean
  gitStatus: ReadonlyMap<string, GitFileStatus>
  /** Turn titles by id, for diff tabs of a turn. */
  turnTitles: ReadonlyMap<string, string>
  terminal: EditorTabsTerminal | null
  onActivate: (id: string) => void
  onClose: (id: string) => void
  onPin: (id: string) => void
  onCloseOthers: (id: string) => void
  onCloseRight: (id: string) => void
  onCloseSaved: () => void
  onCloseAll: () => void
  onReveal: (path: string) => void
  onCopyPath: (path: string, absolute: boolean) => void
  onCompare: (path: string) => void
  onOpenFile: (path: string) => void
}

export function EditorTabs({
  tabs,
  dirtyPaths,
  missingPaths,
  tabVisible,
  gitStatus,
  turnTitles,
  terminal,
  onActivate,
  onClose,
  onPin,
  onCloseOthers,
  onCloseRight,
  onCloseSaved,
  onCloseAll,
  onReveal,
  onCopyPath,
  onCompare,
  onOpenFile,
}: EditorTabsProps) {
  const stripRef = useRef<HTMLDivElement>(null)
  const menu = useContextMenu()

  useEffect(() => {
    if (!tabVisible || tabs.active === null) return
    const strip = stripRef.current
    const el = strip?.querySelector<HTMLElement>(`[data-tab-id="${CSS.escape(tabs.active)}"]`)
    el?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  }, [tabs.active, tabVisible])

  const itemsFor = useCallback(
    (tab: OpenTab): ContextMenuItem[] => {
      const index = tabs.tabs.findIndex((t) => t.id === tab.id)
      const hasRight = index !== -1 && index < tabs.tabs.length - 1
      const path = tab.target.path
      return [
        { id: 'close', label: 'Close', onSelect: () => onClose(tab.id) },
        { id: 'close-others', label: 'Close others', disabled: tabs.tabs.length < 2, onSelect: () => onCloseOthers(tab.id) },
        { id: 'close-right', label: 'Close to the right', disabled: !hasRight, onSelect: () => onCloseRight(tab.id) },
        { id: 'close-saved', label: 'Close saved', onSelect: onCloseSaved },
        { id: 'close-all', label: 'Close all', onSelect: onCloseAll },
        { type: 'separator', id: 's1' },
        ...(!tab.pinned ? [{ id: 'keep', label: 'Keep open', onSelect: () => onPin(tab.id) } satisfies ContextMenuItem] : []),
        ...(tab.target.kind === 'diff'
          ? [{ id: 'open-file', label: 'Open the file', onSelect: () => onOpenFile(path) } satisfies ContextMenuItem]
          : []),
        { id: 'copy-path', label: 'Copy path', onSelect: () => onCopyPath(path, true) },
        { id: 'copy-rel', label: 'Copy relative path', onSelect: () => onCopyPath(path, false) },
        { type: 'separator', id: 's2' },
        { id: 'reveal', label: 'Reveal in explorer', onSelect: () => onReveal(path) },
        { id: 'compare', label: 'Compare with', onSelect: () => onCompare(path) },
      ]
    },
    [tabs.tabs, onClose, onCloseOthers, onCloseRight, onCloseSaved, onCloseAll, onPin, onCopyPath, onReveal, onCompare, onOpenFile],
  )

  return (
    <div
      ref={stripRef}
      className="shui-editor-tabs"
      role="tablist"
      onWheel={(event) => {
        const strip = stripRef.current
        if (!strip || event.deltaX !== 0 || event.deltaY === 0) return
        strip.scrollLeft += event.deltaY
      }}
    >
      {tabs.tabs.map((tab) => {
        const active = tabVisible && tab.id === tabs.active
        const path = tab.target.path
        const isFile = tab.target.kind === 'file'
        const dirty = isFile && dirtyPaths.has(path)
        const missing = isFile && (missingPaths?.has(path) ?? false)
        const status = gitStatus.get(path)
        const name = basename(path)
        const chip =
          tab.target.kind === 'diff'
            ? diffSourceLabel(
                tab.target.source,
                tab.target.source.type === 'turn' ? turnTitles.get(tab.target.source.turnId) : undefined,
              )
            : null
        return (
          // biome-ignore lint/a11y/noStaticElementInteractions: the wrapper relays right/middle clicks for its tab button
          <div
            key={tab.id}
            data-tab-id={tab.id}
            data-status={status}
            data-kind={tab.target.kind}
            className={`shui-etab${active ? ' active' : ''}${tab.pinned ? '' : ' preview'}${dirty ? ' dirty' : ''}${missing ? ' missing' : ''}`}
            onContextMenu={(event) => {
              event.preventDefault()
              menu.open(anchorFromEvent(event), itemsFor(tab))
            }}
            onAuxClick={(event) => {
              if (event.button === 1) {
                event.preventDefault()
                onClose(tab.id)
              }
            }}
          >
            <button
              type="button"
              className="open"
              role="tab"
              aria-selected={active}
              title={missing ? `${path} (not found on disk)` : chip ? `${path} (${chip})` : path}
              onClick={() => onActivate(tab.id)}
              onDoubleClick={() => onPin(tab.id)}
            >
              {isFile ? (
                <FileTypeIcon path={path} className="shui-etab-file-icon" />
              ) : (
                <GitCompareArrows aria-hidden className="shui-etab-icon diff" />
              )}
              <span className="label">{name}</span>
              {chip ? <span className="shui-etab-chip">{chip}</span> : null}
            </button>
            <HoverTip label={dirty ? `Close ${name} (unsaved changes)` : `Close ${name}`}>
              <button
                type="button"
                className="close"
                aria-label={`close ${name}`}
                onClick={(event) => {
                  event.stopPropagation()
                  onClose(tab.id)
                }}
              >
                {dirty ? <span className="shui-dirty" aria-hidden /> : null}
                <X aria-hidden className="shui-x-icon" />
              </button>
            </HoverTip>
          </div>
        )
      })}
      {terminal ? (
        <div className={`shui-etab terminal${terminal.active ? ' active' : ''}`}>
          <button type="button" className="open" role="tab" aria-selected={terminal.active} onClick={terminal.onActivate}>
            <SquareTerminal aria-hidden className="shui-etab-icon" />
            <span className="label">{terminal.title}</span>
          </button>
          <HoverTip label="Close terminal">
            <button type="button" className="close" aria-label="Close terminal" onClick={terminal.onClose}>
              <X aria-hidden className="shui-x-icon" />
            </button>
          </HoverTip>
        </div>
      ) : null}
      {menu.element}
    </div>
  )
}
