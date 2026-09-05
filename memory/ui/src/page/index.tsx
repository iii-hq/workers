/**
 * The memory page (#/ext/memory): the standard page chrome (PageShell/
 * PageHeader from @iii-dev/console-ui) over the bank rail (navigation
 * column) and the selected bank's workspace — rules, memories, graph, and
 * turn preview behind one segmented control in the workspace header.
 * Everything re-reads live off `memory::item-changed` /
 * `memory::bank-changed` (poll fallback while bindings are unavailable,
 * see useMemoryLive). Memory that acts visibly, not magically — watch a
 * memory appear the moment it's learned.
 *
 * Layout adapts to the width the page HAS (a ResizeObserver on its own
 * body, not a viewport media query — the console can host it in panes of
 * any size). Under NARROW_BELOW px it becomes a drill-in flow: the bank
 * list fills the width, opening a bank swaps in its workspace with a ←
 * back button.
 *
 * Unsaved edits anywhere (rule editors, in-place memory edits, the
 * new-memory draft) count into one page-level guard; bank/panel/back
 * navigation asks before discarding them.
 *
 * No presence gate: the host only mounts this page while the memory worker's
 * script is loaded, which already tracks worker connectedness via trigger GC.
 */

import {
  Button,
  type Host,
  PageHeader,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  SegmentedControl,
  StatusDot,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import { BankRail } from './BankRail'
import { Brain, RefreshCw } from './icons'
import { MemoriesPanel } from './MemoriesPanel'
import { MemoryGraph } from './MemoryGraph'
import {
  createBank,
  deleteMemory,
  type MemoryItem,
  openConversation,
  pinMemory,
  reloadStore,
  saveMemory,
  setRule,
  updateMemory,
} from './memory-data'
import { RecallPanel } from './RecallPanel'
import { RulesPanel } from './RulesPanel'
import { useMemoryLive } from './useMemoryLive'
import { BackButton } from './widgets'

/** Container width (px) below which the page collapses to the drill-in
 * banks ⇄ workspace flow. */
const NARROW_BELOW = 800

type Panel = 'memories' | 'graph' | 'rules' | 'recall'

// Team review decision: rules are the most important surface — what goes
// in the system prompt — so they lead the tab order.
const PANEL_OPTIONS: { value: Panel; label: string }[] = [
  { value: 'rules', label: 'Rules' },
  { value: 'memories', label: 'Memories' },
  { value: 'graph', label: 'Graph' },
  { value: 'recall', label: 'Preview' },
]

const isPanel = (v: string | null): v is Panel =>
  v === 'rules' || v === 'memories' || v === 'graph' || v === 'recall'

function readStored(key: string): string | null {
  try {
    return window.localStorage.getItem(key)
  } catch {
    return null
  }
}

function writeStored(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value)
  } catch {
    /* private mode / quota — persistence is best-effort */
  }
}

/** Observe the page body's own width. Returns a callback ref to put on
 * the body plus whether it is currently narrower than `threshold` —
 * container-driven, so the same page adapts inside any pane the console
 * gives it. Measures synchronously on mount to avoid a wide-mode flash;
 * zero widths (display:none) are ignored so a hidden page keeps its last
 * real layout. */
function useContainerNarrow(
  threshold: number,
): [(node: HTMLDivElement | null) => void, boolean] {
  const [narrow, setNarrow] = useState(false)
  const observerRef = useRef<ResizeObserver | null>(null)
  const refCb = useCallback(
    (node: HTMLDivElement | null) => {
      observerRef.current?.disconnect()
      observerRef.current = null
      if (!node) return
      const width = node.getBoundingClientRect().width
      if (width > 0) setNarrow(width < threshold)
      const observer = new ResizeObserver((entries) => {
        const next = entries[0]?.contentRect.width
        if (typeof next === 'number' && next > 0) setNarrow(next < threshold)
      })
      observer.observe(node)
      observerRef.current = observer
    },
    [threshold],
  )
  return [refCb, narrow]
}

export function MemoryPage({
  host,
  panelSide = 'left',
  tabId = '',
  onRequestClose,
  panelContext,
  commands,
}: { host: Host } & Partial<PageRenderProps>) {
  const {
    banks,
    selected,
    setSelected,
    memories,
    total,
    offset,
    setOffset,
    pageSize,
    rules,
    includeSuperseded,
    setIncludeSuperseded,
    tag,
    setTag,
    tags,
    loading,
    error,
    live,
    refresh,
  } = useMemoryLive(host)

  const storageKey = `memory-ui:${tabId || 'page'}`
  const [panel, setPanelState] = useState<Panel>(() => {
    const v = readStored(`${storageKey}:panel`)
    return isPanel(v) ? v : 'rules'
  })
  const [busy, setBusy] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)

  const [rootRef, narrow] = useContainerNarrow(NARROW_BELOW)
  // Narrow drill-in position: the bank list until the user opens one.
  const [drilled, setDrilled] = useState(false)

  // Open editors report unsaved edits up as +1/−1 deltas (several can be
  // dirty at once); navigating away asks before discarding them.
  const dirtyCount = useRef(0)
  const reportDirty = useCallback((delta: number) => {
    dirtyCount.current += delta
  }, [])
  const confirmDiscard = () =>
    dirtyCount.current <= 0 || window.confirm('Discard unsaved changes?')

  // Resolves to whether the mutation landed, so children keep their
  // drafts when it did not (a failed save must not eat the input).
  const act = useCallback(
    async (action: () => Promise<void>): Promise<boolean> => {
      setBusy(true)
      setActionError(null)
      try {
        await action()
        refresh()
        return true
      } catch (err) {
        setActionError(err instanceof Error ? err.message : String(err))
        return false
      } finally {
        setBusy(false)
      }
    },
    [refresh],
  )

  const setPanel = (next: Panel) => {
    if (next === panel) return
    if (!confirmDiscard()) return
    setPanelState(next)
    writeStored(`${storageKey}:panel`, next)
  }

  const openBank = (bank: string) => {
    if (bank !== selected) {
      if (!confirmDiscard()) return
      setSelected(bank)
    }
    setDrilled(true)
  }
  const backToBanks = () => {
    if (!confirmDiscard()) return
    setDrilled(false)
  }

  // Palette rows for a bank/memory (see palette.ts) select it here once the
  // page is open. `panelContext.id` is monotonic, so a repeated identical
  // click still re-applies.
  const appliedContextRef = useRef(0)
  const shellRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!panelContext || panelContext.id === appliedContextRef.current) return
    appliedContextRef.current = panelContext.id
    const context = panelContext.context as {
      type?: string
      bank?: string
    } | null
    if (!context || typeof context.bank !== 'string') return
    openBank(context.bank)
    if (context.type === 'memory') setPanel('memories')
  }, [panelContext, openBank, setPanel])

  // The page's primary verbs, for the palette and for the keyboard while
  // this pane has the focus.
  useEffect(
    () =>
      commands?.register([
        {
          id: 'reload',
          title: 'Reload from disk',
          detail: 'Re-read every bank — picks up hand-edited files',
          enabled: () => !loading && !busy,
          run: () => void act(() => reloadStore(host)),
        },
        {
          id: 'new-bank',
          title: 'New bank',
          detail: 'Create a named memory scope',
          run: () => {
            if (narrow && drilled) backToBanks()
            window.requestAnimationFrame(() => {
              shellRef.current
                ?.querySelector<HTMLElement>('[data-mem-new-bank-input]')
                ?.focus()
            })
          },
        },
      ]),
    [commands, host, act, loading, busy, narrow, drilled, backToBanks],
  )

  const bankInfo = selected
    ? (banks.find((b) => b.name === selected) ?? null)
    : null

  // Narrow: one pane at a time — the bank list, or the opened workspace.
  const showRail = !narrow || !drilled || selected === null
  const showDoc = !narrow || (drilled && selected !== null)

  return (
    <PageShell ref={shellRef} className="mem-ui-shell">
      <PageHeader
        icon={<Brain size={16} />}
        title="Memory"
        description="Banks of rules and memories, injected per turn"
        actions={
          <>
            <span
              className="mem-ui-live"
              title={
                live
                  ? 'updates arrive on the memory trigger types'
                  : 'live bindings unavailable; refreshing on a timer'
              }
            >
              <StatusDot tone={live ? 'accent' : 'ink'} pulse={live} />
              {live ? 'live' : 'polling'}
            </span>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => void act(() => reloadStore(host))}
              disabled={loading || busy}
              className="mem-ui-gap1"
              title="re-read every bank from disk — picks up hand-edited rules/memories files (live events already keep this page current otherwise)"
            >
              <RefreshCw
                size={16}
                className={loading || busy ? 'mem-ui-spin' : undefined}
                aria-hidden
              />
              reload from disk
            </Button>
          </>
        }
        onClose={onRequestClose}
      />

      <div
        className={`mem-ui-body${narrow ? ' narrow' : ''}${panelSide === 'right' ? ' right' : ''}`}
        ref={rootRef}
      >
        {showRail ? (
          <PageSidebar
            label="bank list"
            side={panelSide}
            collapsible
            storageKey="memory:banks"
            defaultWidth={232}
            narrow={narrow}
            className="mem-ui-rail"
            header={
              <div className="mem-ui-col-head">
                <span className="label">Banks</span>
                <span className="spacer" />
                {loading && banks.length === 0 ? null : (
                  <span className="count">{banks.length}</span>
                )}
              </div>
            }
          >
            <BankRail
              banks={banks}
              selected={selected}
              loading={loading}
              onSelect={openBank}
              onCreate={(name) => act(() => createBank(host, name))}
              creating={busy}
              autofocusFirst={selected === null}
            />
          </PageSidebar>
        ) : null}

        {showDoc ? (
          <section className="mem-ui-doc" aria-label="bank workspace">
            {error ? (
              <div className="mem-ui-error-panel grow">
                <p>
                  memory could not be loaded.
                  <span className="detail">{error}</span>
                </p>
                <Button variant="ghost" size="sm" onClick={refresh}>
                  retry
                </Button>
              </div>
            ) : selected === null ? (
              <div className="mem-ui-hero">
                <Brain size={28} className="mem-ui-hero-icon" aria-hidden />
                <h2 className="mem-ui-hero-title">No banks yet</h2>
                <p className="mem-ui-hero-body">
                  create one on the left, or just chat: the default bank
                  materializes when the first memory is saved.
                </p>
              </div>
            ) : (
              <>
                <header className="mem-ui-doc-head">
                  {narrow ? (
                    <BackButton onClick={backToBanks} label="back to banks" />
                  ) : null}
                  <div className="mem-ui-doc-identity">
                    <span className="mem-ui-doc-name" title={selected}>
                      <span className="txt">{selected}</span>
                    </span>
                    {bankInfo ? (
                      <span className="mem-ui-doc-crumb">
                        {bankInfo.memories} memories · {bankInfo.pinned} pinned
                        · {bankInfo.rules} rules
                      </span>
                    ) : null}
                  </div>
                  <SegmentedControl<Panel>
                    value={panel}
                    onChange={setPanel}
                    options={PANEL_OPTIONS}
                    aria-label="memory panel"
                  />
                </header>

                {actionError ? (
                  <div className="mem-ui-banner alert" role="alert">
                    <span>
                      the last action failed.
                      <span className="detail">{actionError}</span>
                    </span>
                    <button
                      type="button"
                      className="mem-ui-linkish quiet"
                      onClick={() => setActionError(null)}
                    >
                      dismiss
                    </button>
                  </div>
                ) : null}

                <div className="mem-ui-doc-scroll">
                  {panel === 'memories' ? (
                    <MemoriesPanel
                      key={selected}
                      host={host}
                      bank={selected}
                      memories={memories}
                      total={total}
                      offset={offset}
                      pageSize={pageSize}
                      onOffsetChange={setOffset}
                      includeSuperseded={includeSuperseded}
                      onToggleSuperseded={setIncludeSuperseded}
                      tag={tag}
                      onTagChange={setTag}
                      tags={tags}
                      onOpenChat={openConversation}
                      onSave={(text) =>
                        act(() => saveMemory(host, selected, text, false))
                      }
                      onPin={(memory: MemoryItem) =>
                        void act(() =>
                          pinMemory(host, selected, memory.id, !memory.pinned),
                        )
                      }
                      onEdit={(memory, text) =>
                        act(() => updateMemory(host, selected, memory.id, text))
                      }
                      onDelete={(memory) =>
                        void act(() => deleteMemory(host, selected, memory.id))
                      }
                      busy={busy}
                      reportDirty={reportDirty}
                    />
                  ) : panel === 'graph' ? (
                    <MemoryGraph
                      memories={memories}
                      totalFacts={total}
                      onShowFacts={() => setPanel('memories')}
                      onPin={(memory: MemoryItem) =>
                        void act(() =>
                          pinMemory(host, selected, memory.id, !memory.pinned),
                        )
                      }
                      onDelete={(memory) =>
                        void act(() => deleteMemory(host, selected, memory.id))
                      }
                      busy={busy}
                    />
                  ) : panel === 'rules' ? (
                    <RulesPanel
                      rules={rules}
                      onSet={(name, content) =>
                        act(() => setRule(host, selected, name, content))
                      }
                      busy={busy}
                      reportDirty={reportDirty}
                    />
                  ) : (
                    <RecallPanel
                      key={selected}
                      host={host}
                      bank={selected}
                      memories={memories}
                      tags={tags}
                    />
                  )}
                </div>
              </>
            )}
          </section>
        ) : null}
      </div>
    </PageShell>
  )
}
