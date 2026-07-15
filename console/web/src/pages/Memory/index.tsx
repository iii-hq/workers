import { AlertCircle, Brain, RefreshCw } from 'lucide-react'
import { useCallback, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { ModeToggle } from '@/components/ui/ModeToggle'
import { StatusDot } from '@/components/ui/StatusDot'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { isMemoryAvailable, useMemoryStatus } from '@/hooks/use-memory-status'
import { useConversationsCtx } from '@/lib/conversations-context'
import {
  createBank,
  deleteFact,
  type MemoryFact,
  pinFact,
  saveFact,
  setBlock,
  updateFact,
} from '@/lib/memory'
import { cn } from '@/lib/utils'
import { BankRail } from './components/BankRail'
import { BlocksPanel } from './components/BlocksPanel'
import { FactsPanel } from './components/FactsPanel'
import { MemoryGraph } from './components/MemoryGraph'
import { RecallPanel } from './components/RecallPanel'
import { useMemoryLive } from './hooks/useMemoryLive'

/**
 * The memory worker's visible-and-editable surface: banks in a left rail,
 * the selected bank's facts (pin/edit/tombstone in place), its
 * always-injected markdown blocks, and a recall dry-run. Everything
 * re-reads live off `memory::item-changed` / `memory::bank-changed`
 * (poll fallback while bindings are unavailable). Memory that acts
 * visibly, not magically — watch a fact appear the moment it's learned.
 */

type Panel = 'facts' | 'graph' | 'blocks' | 'recall'

const PANEL_OPTIONS: { value: Panel; label: string }[] = [
  { value: 'facts', label: 'facts' },
  { value: 'graph', label: 'graph' },
  { value: 'blocks', label: 'blocks' },
  { value: 'recall', label: 'recall' },
]

export function Memory() {
  const { backend } = useConversationsCtx()
  const status = useMemoryStatus(backend.id === 'real')
  const available = isMemoryAvailable(status)

  const {
    banks,
    selected,
    setSelected,
    facts,
    total,
    offset,
    setOffset,
    pageSize,
    blocks,
    includeSuperseded,
    setIncludeSuperseded,
    loading,
    error,
    live,
    refresh,
  } = useMemoryLive(available)

  const [panel, setPanel] = useState<Panel>('facts')
  const [busy, setBusy] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)

  const act = useCallback(
    async (action: () => Promise<void>) => {
      setBusy(true)
      setActionError(null)
      try {
        await action()
        refresh()
      } catch (err) {
        setActionError(err instanceof Error ? err.message : String(err))
      } finally {
        setBusy(false)
      }
    },
    [refresh],
  )

  const bankLabel = loading ? '...' : `${banks.length} banks`

  return (
    <main
      className="flex-1 flex flex-col min-h-0 overflow-hidden"
      aria-label="memory"
    >
      <header className="shrink-0 px-4 sm:px-6 lg:px-8 py-4 border-b border-rule flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="font-mono text-[16px] font-semibold tracking-[-0.01em] text-ink lowercase">
            memory
          </h1>
          <p className="font-mono text-[12px] text-ink-faint mt-0.5 lowercase">
            {available ? bankLabel : 'worker not connected'}
          </p>
        </div>
        {available ? (
          <div className="flex items-center gap-3">
            <span
              className="flex items-center gap-1.5 font-mono text-[11px] lowercase text-ink-faint"
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
              onClick={refresh}
              disabled={loading}
              className="gap-1.5"
            >
              <RefreshCw
                className={cn('w-3.5 h-3.5', loading && 'animate-spin')}
                aria-hidden
              />
              refresh
            </Button>
          </div>
        ) : null}
      </header>

      {!available ? (
        <div className="flex-1 overflow-auto px-4 sm:px-6 lg:px-8 py-4">
          {status.loading ? (
            <p className="font-mono text-[12px] lowercase text-ink-ghost">
              checking for the memory worker...
            </p>
          ) : (
            <StatusPanel
              variant="info"
              icon={<AlertCircle className="w-full h-full" />}
              headline="memory worker not installed"
              detail="this page needs the optional memory worker. run: iii worker add memory"
            />
          )}
        </div>
      ) : (
        <div className="flex-1 flex min-h-0">
          <BankRail
            banks={banks}
            selected={selected}
            onSelect={setSelected}
            onCreate={(name) => void act(() => createBank(name))}
            creating={busy}
          />
          <div className="flex-1 overflow-auto px-4 sm:px-6 lg:px-8 py-4 flex flex-col gap-4 min-w-0">
            {error ? (
              <StatusPanel
                variant="alert"
                icon={<AlertCircle className="w-full h-full" />}
                headline="failed to load memory"
                detail={error}
              />
            ) : !selected ? (
              <div className="flex flex-col items-center gap-2 py-16">
                <Brain className="w-6 h-6 text-ink-ghost" aria-hidden />
                <p className="font-mono text-[12px] lowercase text-ink-faint text-center max-w-md">
                  no banks yet — create one on the left, or just chat: the
                  default bank materializes when the first fact is saved
                </p>
              </div>
            ) : (
              <>
                <div className="flex items-center justify-between gap-3">
                  <ModeToggle<Panel>
                    value={panel}
                    onChange={setPanel}
                    options={PANEL_OPTIONS}
                  />
                  {actionError ? (
                    <span className="font-mono text-[11px] lowercase text-alert truncate">
                      {actionError}
                    </span>
                  ) : null}
                </div>
                {panel === 'facts' ? (
                  <FactsPanel
                    bank={selected}
                    facts={facts}
                    total={total}
                    offset={offset}
                    pageSize={pageSize}
                    onOffsetChange={setOffset}
                    includeSuperseded={includeSuperseded}
                    onToggleSuperseded={setIncludeSuperseded}
                    onSave={(text) =>
                      void act(() => saveFact(selected, text, false))
                    }
                    onPin={(fact: MemoryFact) =>
                      void act(() => pinFact(selected, fact.id, !fact.pinned))
                    }
                    onEdit={(fact, text) =>
                      void act(() => updateFact(selected, fact.id, text))
                    }
                    onDelete={(fact) =>
                      void act(() => deleteFact(selected, fact.id))
                    }
                    busy={busy}
                  />
                ) : panel === 'graph' ? (
                  <MemoryGraph
                    facts={facts}
                    totalFacts={total}
                    onShowFacts={() => setPanel('facts')}
                    onPin={(fact: MemoryFact) =>
                      void act(() => pinFact(selected, fact.id, !fact.pinned))
                    }
                    onDelete={(fact) =>
                      void act(() => deleteFact(selected, fact.id))
                    }
                    busy={busy}
                  />
                ) : panel === 'blocks' ? (
                  <BlocksPanel
                    blocks={blocks}
                    onSet={(name, content) =>
                      void act(() => setBlock(selected, name, content))
                    }
                    busy={busy}
                  />
                ) : (
                  <RecallPanel bank={selected} />
                )}
              </>
            )}
          </div>
        </div>
      )}
    </main>
  )
}
