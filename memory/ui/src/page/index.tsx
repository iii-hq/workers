import { useCallback, useState } from 'react'
import { Button, type Host, StatusDot, StatusPanel } from '@iii-dev/console-ui'
import { BankRail } from './BankRail'
import { AlertCircle, Brain, RefreshCw } from './icons'
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
import { MemoriesPanel } from './MemoriesPanel'
import { MemoryGraph } from './MemoryGraph'
import { ModeToggle } from './ModeToggle'
import { RecallPanel } from './RecallPanel'
import { RulesPanel } from './RulesPanel'
import { useMemoryLive } from './useMemoryLive'

/**
 * The memory worker's visible-and-editable surface (`#/ext/memory`): banks in
 * a left rail, the selected bank's memories (pin/edit/tombstone in place), a
 * schematic graph, its always-injected markdown rules, and a turn-preview dry
 * run. Everything re-reads live off `memory::item-changed` /
 * `memory::bank-changed` (poll fallback while bindings are unavailable).
 * Memory that acts visibly, not magically — watch a memory appear the moment
 * it's learned.
 *
 * No presence gate: the host only mounts this page while the memory worker's
 * script is loaded, which already tracks worker connectedness via trigger GC.
 */

type Panel = 'memories' | 'graph' | 'rules' | 'recall'

// Team review decision: rules are the most important surface — what goes
// in the system prompt — so they lead the tab order.
const PANEL_OPTIONS: { value: Panel; label: string }[] = [
  { value: 'rules', label: 'rules' },
  { value: 'memories', label: 'memories' },
  { value: 'graph', label: 'graph' },
  { value: 'recall', label: 'preview' },
]

export function MemoryPage({ host }: { host: Host }) {
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

  const [panel, setPanel] = useState<Panel>('rules')
  const [busy, setBusy] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)

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

  const bankLabel = loading ? '...' : `${banks.length} banks`

  return (
    <div className="mem-page" aria-label="memory">
      <header className="mem-header">
        <div>
          <h1 className="mem-h1">memory</h1>
          <p className="mem-header-sub">{bankLabel}</p>
        </div>
        <div className="mem-row">
          <span
            className="mem-live"
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
            className="mem-gap1"
            title="re-read every bank from disk — picks up hand-edited rules/memories files (live events already keep this page current otherwise)"
          >
            <RefreshCw
              size={14}
              className={loading || busy ? 'mem-spin' : undefined}
              aria-hidden
            />
            reload from disk
          </Button>
        </div>
      </header>

      <div className="mem-body">
        <BankRail
          banks={banks}
          selected={selected}
          onSelect={setSelected}
          onCreate={(name) => act(() => createBank(host, name))}
          creating={busy}
        />
        <div className="mem-content">
          {error ? (
            <StatusPanel
              variant="alert"
              icon={<AlertCircle size={18} />}
              headline="failed to load memory"
              detail={error}
            />
          ) : !selected ? (
            <div className="mem-empty-banks">
              <Brain size={24} style={{ color: 'var(--color-ink-ghost)' }} aria-hidden />
              <p className="mem-hint mem-center">
                no banks yet — create one on the left, or just chat: the default
                bank materializes when the first memory is saved
              </p>
            </div>
          ) : (
            <>
              <div className="mem-spread">
                <ModeToggle<Panel>
                  value={panel}
                  onChange={setPanel}
                  options={PANEL_OPTIONS}
                  aria-label="memory panel"
                />
                {actionError ? (
                  <span className="mem-danger mem-truncate">{actionError}</span>
                ) : null}
              </div>
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
                  onSave={(text) => act(() => saveMemory(host, selected, text, false))}
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
            </>
          )}
        </div>
      </div>
    </div>
  )
}
