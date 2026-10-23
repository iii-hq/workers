// @vitest-environment jsdom
import type { Host } from '@iii-dev/console-ui'
import { act, type ComponentProps, type ReactElement, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { WhatsAppPage } from '../src/page'
import type { Session } from '../src/model'

const { confirm } = vi.hoisted(() => ({ confirm: vi.fn() }))
vi.mock('@iii-dev/console-ui', async () => {
  const { cloneElement, createContext, useContext, useEffect } = await import('react')
  const MenuContext = createContext({ open: false, onOpenChange: (_value: boolean) => {} })
  const TabContext = createContext({ value: '', onValueChange: (_value: string) => {} })
  const Box = ({ children }: { children?: ReactNode }) => <div>{children}</div>
  return {
    PageShell: ({ children, ...props }: ComponentProps<'div'>) => <div {...props}>{children}</div>,
    PageHeader: ({ children, actions }: { children: ReactNode; actions: ReactNode }) => <header>{children}{actions}</header>,
    SettingsList: Box, TabsList: Box, StatusBar: Box,
    DropdownMenu: ({ open = false, onOpenChange, children }: { open?: boolean; onOpenChange(value: boolean): void; children: ReactNode }) => <MenuContext.Provider value={{ open, onOpenChange }}>{children}</MenuContext.Provider>,
    DropdownMenuTrigger: ({ children }: { children: ReactElement<ComponentProps<'button'>> }) => {
      const menu = useContext(MenuContext)
      return cloneElement(children, { onClick: () => menu.onOpenChange(!menu.open), 'aria-expanded': menu.open })
    },
    DropdownMenuContent: ({ children }: { children: ReactNode }) => {
      const menu = useContext(MenuContext)
      useEffect(() => {
        if (!menu.open) return
        // Model the shared menu's deferred focus restoration on close.
        return () => { window.setTimeout(() => document.querySelector<HTMLButtonElement>('[aria-label="Session actions"]')?.focus(), 0) }
      }, [menu.open])
      return menu.open ? <div role="menu">{children}</div> : null
    },
    DropdownMenuItem: ({ children, onSelect, disabled }: { children: ReactNode; onSelect?(event: Event): void; disabled?: boolean }) => {
      const menu = useContext(MenuContext)
      return <button role="menuitem" disabled={disabled} onClick={() => { onSelect?.(new Event('select')); menu.onOpenChange(false) }}>{children}</button>
    },
    Button: ({ children, variant: _variant, ...props }: ComponentProps<'button'> & { variant?: string }) => <button {...props}>{children}</button>,
    IconButton: ({ label, children, ...props }: ComponentProps<'button'> & { label: string }) => <button {...props} aria-label={label}>{children}</button>,
    Input: ({ onChange, ...props }: Omit<ComponentProps<'input'>, 'onChange'> & { onChange(value: string): void }) => <input {...props} onChange={(event) => onChange(event.target.value)} />,
    Select: ({ value, onChange, options, ...props }: { value?: string; onChange(value: string): void; options: { value: string; label: string }[]; disabled?: boolean; 'aria-label'?: string }) => <select {...props} value={value ?? ''} onChange={(event) => onChange(event.target.value)}><option value="">Choose a session</option>{options.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}</select>,
    SettingsField: ({ field, renderControl }: { field: string; renderControl(props: { id: string }): ReactNode }) => renderControl({ id: field }),
    Tabs: ({ value, onValueChange, children }: { value: string; onValueChange(value: string): void; children: ReactNode }) => <TabContext.Provider value={{ value, onValueChange }}>{children}</TabContext.Provider>,
    TabsTrigger: ({ value, children }: { value: string; children: ReactNode }) => { const tabs = useContext(TabContext); return <button role="tab" onClick={() => tabs.onValueChange(value)}>{children}</button> },
    TabsContent: ({ value, children }: { value: string; children: ReactNode }) => useContext(TabContext).value === value ? children : null,
    Skeleton: () => <p>Loading…</p>,
    EmptyState: ({ title }: { title: string }) => <p>{title}</p>,
    StatusPanel: ({ headline, detail, action }: { headline: ReactNode; detail?: ReactNode; action?: ReactNode }) => <div role="alert">{headline}{detail}{action}</div>,
    useConfirm: () => ({ confirm, dialog: null }),
  }
})
vi.mock('../src/session', () => ({
  SessionBadge: () => <span>Stopped</span>,
  SessionDetail: ({ sessionId, onDirty }: { sessionId: string; onDirty(value: boolean): void }) => <section data-session={sessionId}><button onClick={() => onDirty(true)}>Make draft</button><button>Start session</button></section>,
}))
vi.mock('../src/configuration', () => ({ RuntimeStatus: () => null }))
vi.mock('../src/cloud', () => ({ CloudPanel: () => <p>Cloud fixture</p> }))
vi.mock('../src/messages', () => ({ MessageBrowser: () => <p>Message fixture</p> }))
vi.mock('../src/groups', () => ({ GroupBrowser: () => <p>Group fixture</p> }))

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true
const cleanups: Array<() => void> = []
const session = (session_id: string, created_at_ms = 1): Session => ({ session_id, created_at_ms, status: 'stopped' })
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => { resolve = done })
  return { promise, resolve }
}
async function render(initial = [session('first'), session('second')]) {
  let sessions = initial
  let failure: Error | undefined
  let pendingDelete: Promise<Session> | undefined, pendingList: Promise<Session[]> | undefined
  const trigger = vi.fn(async (id: string, input?: { session_id?: string }) => {
    if (id === 'whatsapp::sessions::list') return pendingList ?? sessions
    if (id !== 'whatsapp::sessions::delete') throw new Error(`Unexpected function: ${id}`)
    if (failure) throw failure
    if (pendingDelete) return pendingDelete
    const removed = sessions.find((item) => item.session_id === input?.session_id)!
    sessions = sessions.filter((item) => item !== removed)
    return removed
  })
  const host = { iii: { trigger, on: () => () => {}, registerTrigger: () => () => {}, browserId: 'test' } } as unknown as Host
  const container = document.createElement('div'); document.body.append(container)
  const root = createRoot(container)
  await act(async () => root.render(<WhatsAppPage host={host} configurationId="test" paneId="pane" />))
  let mounted = true
  const unmount = () => { if (mounted) { act(() => root.unmount()); mounted = false }; container.remove() }
  cleanups.push(unmount)
  return {
    container, trigger, unmount,
    setSessions(value: Session[]) { sessions = value },
    fail(error: Error) { failure = error },
    deferDelete(promise: Promise<Session>) { pendingDelete = promise },
    deferList(promise: Promise<Session[]>) { pendingList = promise },
    async click(label: string) {
      const button = Array.from(container.querySelectorAll('button')).find((item) => item.getAttribute('aria-label') === label || item.textContent === label)!
      expect(button, `Missing button: ${label}`).toBeDefined()
      await act(async () => button.click())
      await act(async () => { await vi.advanceTimersByTimeAsync(0) })
    },
    selected() { return container.querySelector<HTMLSelectElement>('select')?.value ?? '' },
    deleteCalls() { return trigger.mock.calls.filter(([id]) => id.endsWith('::delete')) },
  }
}
beforeEach(() => {
  vi.useFakeTimers()
  const storage = new Map<string, string>()
  vi.stubGlobal('localStorage', { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => storage.set(key, value) })
  confirm.mockReset(); confirm.mockResolvedValue(true)
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
})
afterEach(() => { for (const cleanup of cleanups.splice(0)) cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers() })

describe('Session removal', () => {
  it('groups session actions in one narrow-pane menu and restores focus after cancellation', async () => {
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ width: 390, height: 844, top: 0, left: 0, right: 390, bottom: 844, x: 0, y: 0, toJSON: () => ({}) })
    confirm.mockResolvedValue(false)
    const view = await render()
    expect(view.container.querySelector('[aria-label="Session actions"]')).not.toBeNull()
    expect(view.container.querySelector('[role="menu"]')).toBeNull()
    expect(view.container.querySelector('[aria-label="Remove session"]')).toBeNull()
    await view.click('Session actions')
    expect(Array.from(view.container.querySelectorAll('[role="menuitem"]')).map((item) => item.textContent)).toEqual(['New session', 'Refresh sessions', 'Remove session'])
    await view.click('Remove session')
    expect(view.container.querySelector('[role="menu"]')).toBeNull()
    expect(view.deleteCalls()).toHaveLength(0)
    expect(view.selected()).toBe('first')
    expect(document.activeElement).toBe(view.container.querySelector('[aria-label="Session actions"]'))
  })

  it('restores focus to the remaining session after removal from the narrow-pane menu', async () => {
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ width: 390, height: 844, top: 0, left: 0, right: 390, bottom: 844, x: 0, y: 0, toJSON: () => ({}) })
    const view = await render()
    await view.click('Session actions')
    await view.click('Remove session')
    expect(view.container.querySelector('[role="menu"]')).toBeNull()
    expect(view.selected()).toBe('second')
    expect(document.activeElement).toBe(view.container.querySelector('select'))
    expect(view.deleteCalls()).toHaveLength(1)
  })

  it('requires confirmation and deletes only the selected session', async () => {
    const view = await render()
    expect(view.deleteCalls()).toHaveLength(0)
    await view.click('Remove session')
    expect(confirm).toHaveBeenCalledWith(expect.objectContaining({ title: 'Remove session "first"?', tone: 'danger', confirmLabel: 'Remove session', cancelLabel: 'Keep session' }))
    expect(confirm.mock.calls[0][0].description).toContain('permanently deletes its saved credentials and local message archive')
    expect(confirm.mock.calls[0][0].description).toContain('does not unlink the device')
    expect(view.deleteCalls()).toEqual([['whatsapp::sessions::delete', { session_id: 'first' }, { timeoutMs: 730000 }]])
    expect(view.selected()).toBe('second')
    expect(document.activeElement).toBe(view.container.querySelector('select'))
    expect(view.container.querySelector('[data-session]')?.getAttribute('data-session')).toBe('second')
    expect(view.container.textContent).toContain('Session "first" removed')
  })

  it('keeps credentials, selection and draft when confirmation is cancelled', async () => {
    confirm.mockResolvedValue(false)
    const view = await render()
    await view.click('Make draft'); await view.click('Remove session')
    expect(confirm.mock.calls[0][0].details).toEqual(['Your unsent draft will be discarded. A send already requested may still complete.'])
    expect(view.deleteCalls()).toHaveLength(0)
    expect(view.selected()).toBe('first')
  })

  it('shows session creation after deleting the last session', async () => {
    const view = await render([session('only')])
    await view.click('Remove session')
    expect(view.selected()).toBe('')
    expect(view.container.querySelector('[data-session]')).toBeNull()
    expect(view.container.textContent).toContain('Create session')
    expect(document.activeElement).toBe(view.container.querySelector('input'))
    expect(view.container.querySelector('select')).toBeNull()
    expect(view.container.querySelector('[aria-label="Remove session"]')).toBeNull()
  })

  it('hides a deleted incarnation while a stale list is in flight', async () => {
    const old = session('only')
    const view = await render([old])
    const list = deferred<Session[]>()
    view.deferList(list.promise)
    await view.click('Refresh sessions'); await view.click('Remove session')
    expect(view.selected()).toBe('')
    await act(async () => list.resolve([old]))
    expect(view.selected()).toBe('')
    expect(Array.from(view.container.querySelectorAll('option')).map((item) => item.value)).not.toContain('only')
    // A newly created session with the same name is a separate incarnation.
    view.deferList(Promise.resolve([session('only', 2)]))
    await act(async () => { await vi.advanceTimersByTimeAsync(10000) })
    expect(view.selected()).toBe('only')
  })

  it('blocks duplicate requests and session controls during removal', async () => {
    const view = await render()
    const deletion = deferred<Session>()
    view.deferDelete(deletion.promise)
    await view.click('Remove session'); await view.click('Remove session')
    expect(view.deleteCalls()).toHaveLength(1)
    expect(view.container.querySelector<HTMLSelectElement>('select')?.disabled).toBe(true)
    expect(view.container.querySelector('[data-session]')).toBeNull()
    expect(view.container.textContent).toContain('Removing session "first"')
    await act(async () => deletion.resolve(session('first')))
    expect(view.selected()).toBe('second')
  })

  it('reports an ambiguous error without retrying or claiming removal', async () => {
    const view = await render()
    view.fail(new Error('Request timed out; outcome unknown'))
    await view.click('Remove session')
    await act(async () => { await vi.advanceTimersByTimeAsync(15000) })
    expect(view.deleteCalls()).toHaveLength(1)
    expect(view.selected()).toBe('first')
    expect(view.container.textContent).toContain('Session removal not confirmed')
    expect(view.container.textContent).not.toContain('Session "first" removed')
  })

  it('does not delete after closing while confirmation is pending', async () => {
    const answer = deferred<boolean>(); confirm.mockReturnValue(answer.promise)
    const view = await render()
    await view.click('Remove session'); view.unmount()
    await act(async () => answer.resolve(true))
    expect(view.deleteCalls()).toHaveLength(0)
  })

  it('clears an absent session after an uncertain removal without claiming success', async () => {
    const view = await render([session('only')])
    view.setSessions([])
    view.fail(new Error('Request timed out; outcome unknown'))
    await view.click('Remove session')
    expect(view.selected()).toBe('')
    expect(view.container.querySelector('[data-session]')).toBeNull()
    expect(view.container.textContent).toContain('Session removal not confirmed')
    expect(view.container.textContent).not.toContain('Session "only" removed')
    expect(view.deleteCalls()).toHaveLength(1)
  })

  it('does not expose linked-device deletion in the Cloud API tab', async () => {
    const view = await render()
    await view.click('Meta Cloud API')
    expect(view.container.querySelector('[aria-label="Remove session"]')).toBeNull()
    expect(view.deleteCalls()).toHaveLength(0)
  })
})
