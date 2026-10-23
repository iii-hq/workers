import { Button, Dialog, DialogContent, DialogDescription, DialogTitle } from '@iii-dev/console-ui'
import { createContext, useCallback, useContext, useEffect, useLayoutEffect, useRef, useState, type Dispatch, type ReactNode, type SetStateAction } from 'react'

interface ActiveDraft { key: string; dirty: boolean; busy: boolean; clear(): void }
interface DraftMemory {
  values: Map<string, unknown>
  records: Map<string, unknown>
  active: ActiveDraft | null
  notify(): void
  clearPrefix(prefix: string): void
}
const Memory = createContext<DraftMemory | null>(null)
const Scope = createContext('')

/** Drafts exist only until this page closes; files remain File references in memory. */
export function DraftProvider({ children }: { children: ReactNode }) {
  const [, changed] = useState(0)
  const memory = useRef<DraftMemory | null>(null)
  if (!memory.current) memory.current = {
    values: new Map(), records: new Map(), active: null, notify: () => changed((value) => value + 1),
    clearPrefix(prefix) {
      for (const key of this.values.keys()) if (key.startsWith(prefix)) this.values.delete(key)
      for (const key of this.records.keys()) if (key.startsWith(prefix)) this.records.delete(key)
      this.notify()
    },
  }
  return <Memory.Provider value={memory.current}>{children}</Memory.Provider>
}
export function DraftScope({ id, children }: { id: string; children: ReactNode }) {
  return <Scope.Provider value={id}>{children}</Scope.Provider>
}
export const useDraftScope = () => useContext(Scope)
export const useDraftMemory = () => useContext(Memory)

export function useDraftState<T>(key: string, initial: T, isDirty: (value: T) => boolean): [T, Dispatch<SetStateAction<T>>, () => void] {
  return useMemoryState(key, initial, 'values', isDirty)
}

/** Receipts and test evidence share the page lifetime without becoming unsent drafts. */
export function usePageState<T>(key: string, initial: T): [T, Dispatch<SetStateAction<T>>, () => void] {
  return useMemoryState(key, initial, 'records', () => true)
}

function useMemoryState<T>(key: string, initial: T, collection: 'values' | 'records', retain: (value: T) => boolean): [T, Dispatch<SetStateAction<T>>, () => void] {
  const memory = useContext(Memory)
  const entries = memory?.[collection]
  const defaults = useRef(initial)
  const remembered = entries?.has(key) ? entries.get(key) as T : initial
  const [state, update] = useState(() => ({ key, value: remembered }))
  const value = state.key === key ? state.value : remembered
  if (state.key !== key) { defaults.current = initial; update({ key, value }) }
  const current = useRef(value); current.current = value
  const predicate = useRef(retain); predicate.current = retain
  const setValue = useCallback<Dispatch<SetStateAction<T>>>((next) => {
    const resolved = typeof next === 'function' ? (next as (previous: T) => T)(current.current) : next
    current.current = resolved
    if (entries) {
      if (predicate.current(resolved)) entries.set(key, resolved)
      else entries.delete(key)
      if (collection === 'values') memory?.notify()
    }
    update({ key, value: resolved })
  }, [key, memory, entries, collection])
  const clear = useCallback(() => {
    entries?.delete(key)
    current.current = defaults.current
    update({ key, value: defaults.current })
    if (collection === 'values') memory?.notify()
  }, [key, memory, entries, collection])
  return [value, setValue, clear]
}

export function useDraftRegistration(key: string, draft: Omit<ActiveDraft, 'key'>) {
  const memory = useContext(Memory)
  useLayoutEffect(() => {
    if (!memory) return
    const active = { key, ...draft }
    memory.active = active
    return () => { if (memory.active === active) memory.active = null }
  }, [memory, key, draft.dirty, draft.busy, draft.clear])
}

type DraftChoice = 'edit' | 'keep' | 'discard'
/** Uses the console's focus-trapped dialog and its cancel-first focus convention. */
export function useDraftNavigation() {
  const [open, setOpen] = useState(false)
  const resolve = useRef<((choice: DraftChoice) => void) | null>(null)
  const cancel = useRef<HTMLButtonElement>(null)
  const ask = useCallback(() => new Promise<DraftChoice>((done) => { resolve.current = done; setOpen(true) }), [])
  const settle = (choice: DraftChoice) => { setOpen(false); resolve.current?.(choice); resolve.current = null }
  useEffect(() => () => resolve.current?.('edit'), [])
  const dialog = open ? <Dialog open={open} onOpenChange={(next) => { if (!next) settle('edit') }}>
    <DialogContent role="alertdialog" className="wa-ui-draft-dialog">
      <DialogTitle className="wa-ui-draft-title">Keep this draft?</DialogTitle>
      <DialogDescription className="wa-ui-draft-description">Keep it to resume here later, or discard it before switching. Kept drafts are cleared when this page closes.</DialogDescription>
      <div className="wa-ui-draft-actions">
        <Button ref={cancel} variant="ghost" type="button" onClick={() => settle('edit')}>Keep editing</Button>
        <Button variant="ghost" type="button" onClick={() => settle('discard')}>Discard draft and switch</Button>
        <Button type="button" onClick={() => settle('keep')}>Keep draft and switch</Button>
      </div>
    </DialogContent>
  </Dialog> : null
  return { ask, dialog }
}
