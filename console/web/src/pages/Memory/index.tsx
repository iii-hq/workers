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
  deleteMemory,
  type MemoryItem,
  pinMemory,
  reloadStore,
  saveMemory,
  setRule,
  updateMemory,
} from '@/lib/memory'
import { cn } from '@/lib/utils'
import { BankRail } from './components/BankRail'
import { MemoriesPanel } from './components/MemoriesPanel'
import { MemoryGraph } from './components/MemoryGraph'
import { RecallPanel } from './components/RecallPanel'
import { RulesPanel } from './components/RulesPanel'
import { useMemoryLive } from './hooks/useMemoryLive'

/**
 * The memory worker's visible-and-editable surface: banks in a left rail,
 * the selected bank's memories (pin/edit/tombstone in place), its
 * always-injected markdown rules, and a recall dry-run. Everything
 * re-reads live off `memory::item-changed` / `memory::bank-changed`
 * (poll fallback while bindings are unavailable). Memory that acts
 * visibly, not magically — watch a memory appear the moment it's learned.
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

export function Memory() {
  const conversations = useConversationsCtx()
  const { backend } = conversations
  const status = useMemoryStatus(backend.id === 'real')
  const available = isMemoryAvailable(status)

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
  } = useMemoryLive(available)

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
              onClick={() => void act(() => reloadStore())}
              disabled={loading || busy}
              className="gap-1.5"
              title="re-read every bank from disk — picks up hand-edited rules/memories files (live events already keep this page current otherwise)"
            >
              <RefreshCw
                className={cn(
                  'w-3.5 h-3.5',
                  (loading || busy) && 'animate-spin',
                )}
                aria-hidden
              />
              reload from disk
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
            onCreate={(name) => act(() => createBank(name))}
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
                  default bank materializes when the first memory is saved
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
                {panel === 'memories' ? (
                  <MemoriesPanel
                    key={selected}
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
                    onOpenChat={conversations.select}
                    onSave={(text) =>
                      act(() => saveMemory(selected, text, false))
                    }
                    onPin={(memory: MemoryItem) =>
                      void act(() =>
                        pinMemory(selected, memory.id, !memory.pinned),
                      )
                    }
                    onEdit={(memory, text) =>
                      act(() => updateMemory(selected, memory.id, text))
                    }
                    onDelete={(memory) =>
                      void act(() => deleteMemory(selected, memory.id))
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
                        pinMemory(selected, memory.id, !memory.pinned),
                      )
                    }
                    onDelete={(memory) =>
                      void act(() => deleteMemory(selected, memory.id))
                    }
                    busy={busy}
                  />
                ) : panel === 'rules' ? (
                  <RulesPanel
                    rules={rules}
                    onSet={(name, content) =>
                      act(() => setRule(selected, name, content))
                    }
                    busy={busy}
                  />
                ) : (
                  <RecallPanel
                    key={selected}
                    bank={selected}
                    memories={memories}
                    tags={tags}
                  />
                )}
              </>
            )}
          </div>
        </div>
      )}
    </main>
  )
}
