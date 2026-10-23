// @vitest-environment jsdom
import type { Host, PageCommand } from '@iii-dev/console-ui'
import { act, useEffect, useState, type ComponentProps, type ReactElement, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { WhatsAppPage } from '../src/page'
import type { Session } from '../src/model'
import { useDraftRegistration, useDraftScope, useDraftState } from '../src/drafts'

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
    Dialog: ({ open, children, onOpenChange }: { open: boolean; children: ReactNode; onOpenChange(value: boolean): void }) => open ? <div onKeyDown={(event) => { if (event.key === 'Escape') onOpenChange(false) }}>{children}</div> : null,
    DialogContent: ({ children, ...props }: ComponentProps<'div'>) => {
      useEffect(() => () => {
        // Match the shared dialog's deferred cleanup when its focused action
        // disappears, after React has committed the destination.
        window.setTimeout(() => (document.activeElement as HTMLElement | null)?.blur(), 0)
      }, [])
      return <div {...props}>{children}</div>
    },
    DialogTitle: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
    DialogDescription: ({ children }: { children: ReactNode }) => <p>{children}</p>,
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
    TabsTrigger: ({ value, children }: { value: string; children: ReactNode }) => {
      const tabs = useContext(TabContext)
      return <button role="tab" aria-selected={tabs.value === value} data-state={tabs.value === value ? 'active' : 'inactive'} onClick={() => tabs.onValueChange(value)} onFocus={() => { if (tabs.value !== value) tabs.onValueChange(value) }}>{children}</button>
    },
    TabsContent: ({ value, children }: { value: string; children: ReactNode }) => useContext(TabContext).value === value ? <div role="tabpanel" data-state="active">{children}</div> : null,
    Skeleton: () => <p>Loading…</p>,
    EmptyState: ({ title }: { title: string }) => <p>{title}</p>,
    StatusPanel: ({ headline, detail, action }: { headline: ReactNode; detail?: ReactNode; action?: ReactNode }) => <div role="alert">{headline}{detail}{action}</div>,
    useConfirm: () => ({ confirm, dialog: null }),
  }
})
vi.mock('../src/configuration', () => ({ RuntimeStatus: () => null }))
vi.mock('../src/session', () => ({
  SessionBadge: () => <span>Stopped</span>,
  SessionDetail: ({ sessionId, onDirty }: { sessionId: string; onDirty(value: boolean): void }) => {
    const key = `${useDraftScope()}:test`
    const [draft, setDraft, clear] = useDraftState(key, '', (value) => !!value)
    const [pending, setPending] = useState(false)
    useEffect(() => { onDirty(!!draft || pending); return () => onDirty(false) }, [draft, pending, onDirty])
    useDraftRegistration(key, { dirty: !!draft, busy: pending, clear })
    return <section data-session={sessionId}><p>Phone connection fixture</p><p>{draft}</p><textarea aria-label="Native test message" value={draft} onChange={(event) => setDraft(event.target.value)} /><button onClick={() => setDraft('Saved phone draft')}>Draft phone message</button><button onClick={() => setPending(true)}>Begin send</button><button onClick={() => { clear(); setPending(false) }}>Finish send</button></section>
  },
}))
vi.mock('../src/cloud', () => ({
  CloudPanel: ({ onDirty }: { onDirty(value: boolean): void }) => {
    const key = 'cloud:waba:phone:test'
    const [draft, setDraft, clear] = useDraftState(key, '', (value) => !!value)
    useEffect(() => { onDirty(!!draft); return () => onDirty(false) }, [draft, onDirty])
    useDraftRegistration(key, { dirty: !!draft, busy: false, clear })
    return <section data-cloud><p>Cloud API fixture</p><p>{draft}</p><textarea aria-label="Cloud test message" value={draft} onChange={(event) => setDraft(event.target.value)} /><button onClick={() => setDraft('Saved Cloud draft')}>Draft Cloud message</button></section>
  },
}))
vi.mock('../src/messages', () => ({ MessageBrowser: () => <p>Phone messages fixture</p> }))
vi.mock('../src/groups', () => ({ GroupBrowser: () => <p>Phone groups fixture</p> }))

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true
const cleanups: Array<() => void> = []
const session = (session_id: string): Session => ({ session_id, created_at_ms: 1, status: 'stopped' })

async function render(initial: Session[] = [session('support')]) {
  let sessions = initial
  let registeredCommands: readonly PageCommand[] = []
  const commands = { register(value: readonly PageCommand[]) { registeredCommands = value; return () => { registeredCommands = [] } } }
  const trigger = vi.fn(async (id: string, input?: { session_id?: string }) => {
    if (id === 'whatsapp::sessions::list') return sessions
    if (id === 'whatsapp::sessions::create') {
      const created = session(input!.session_id!)
      sessions = [...sessions, created]
      return created
    }
    throw new Error(`Unexpected function: ${id}`)
  })
  const host = { iii: { trigger, on: () => () => {}, registerTrigger: () => () => {}, browserId: 'test' } } as unknown as Host
  const setDirty = vi.fn()
  const container = document.createElement('div'); document.body.append(container)
  const root = createRoot(container)
  await act(async () => root.render(<WhatsAppPage host={host} configurationId="transport-test" paneId="pane" commands={commands} setDirty={setDirty} />))
  cleanups.push(() => { act(() => root.unmount()); container.remove() })
  return {
    container, trigger, setDirty,
    async click(label: string) {
      const button = Array.from(container.querySelectorAll('button')).find((item) => item.getAttribute('aria-label') === label || item.textContent === label)
      expect(button, `Missing button: ${label}`).toBeDefined()
      await act(async () => { button!.focus(); button!.click() })
      await act(async () => { await vi.advanceTimersByTimeAsync(0) })
    },
    async typeSessionName(value: string) {
      const input = container.querySelector<HTMLInputElement>('input')
      expect(input, 'Session name field must be present').not.toBeNull()
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, value)
        input!.dispatchEvent(new Event('input', { bubbles: true }))
      })
    },
    async runCommand(id: string) {
      const command = registeredCommands.find((item) => item.id === id)
      expect(command, `Missing command: ${id}`).toBeDefined()
      await act(async () => { await command!.run() })
    },
    tabLabels() { return Array.from(container.querySelectorAll('[role="tab"]')).map((item) => item.textContent) },
    mutationCalls() { return trigger.mock.calls.filter(([id]) => id !== 'whatsapp::sessions::list') },
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

describe('WhatsApp transport navigation', () => {
  it('opens session creation after the narrow menu closes and restores focus on cancel', async () => {
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ width: 390, height: 844, top: 0, left: 0, right: 390, bottom: 844, x: 0, y: 0, toJSON: () => ({}) })
    const view = await render()
    await view.click('Session actions')
    await view.click('New session')
    expect(view.container.querySelector('[role="menu"]')).toBeNull()
    expect(view.container.querySelector('form')).not.toBeNull()
    expect(document.activeElement).toBe(view.container.querySelector('input'))
    await view.click('Cancel')
    expect(view.container.querySelector('form')).toBeNull()
    expect(document.activeElement).toBe(view.container.querySelector('[aria-label="Session actions"]'))
    await view.click('Session actions')
    await view.click('Refresh sessions')
    expect(view.container.querySelector('[role="menu"]')).toBeNull()
    expect(view.trigger.mock.calls.filter(([id]) => id === 'whatsapp::sessions::list')).toHaveLength(2)
    expect(view.mutationCalls()).toHaveLength(0)
  })

  it('keeps phone connection, messages and groups within Linked phone', async () => {
    const view = await render()
    expect(view.tabLabels()).toEqual(['Linked phone', 'Meta Cloud API', 'Connection', 'Messages', 'Groups'])
    expect(view.container.querySelector('[aria-label="WhatsApp session: support"]')).not.toBeNull()
    expect(view.container.querySelector('[data-session="support"]')).not.toBeNull()
    await view.click('Messages')
    expect(view.container.textContent).toContain('Phone messages fixture')
    await view.click('Groups')
    expect(view.container.textContent).toContain('Phone groups fixture')
    expect(view.mutationCalls()).toHaveLength(0)
  })

  it('hides all phone session controls and an open creation form in Meta Cloud API', async () => {
    const view = await render()
    await view.click('New session')
    expect(view.container.querySelector('form')).not.toBeNull()
    await view.click('Meta Cloud API')
    expect(view.tabLabels()).toEqual(['Linked phone', 'Meta Cloud API'])
    expect(view.container.querySelector('[data-cloud]')).not.toBeNull()
    expect(view.container.querySelector('select')).toBeNull()
    expect(view.container.querySelector('form')).toBeNull()
    for (const label of ['New session', 'Refresh sessions', 'Remove session']) expect(view.container.querySelector(`[aria-label="${label}"]`)).toBeNull()
    expect(view.container.querySelector('[data-session]')).toBeNull()
    expect(view.mutationCalls()).toHaveLength(0)
  })

  it('offers a single phone onboarding form without inactive message or group views', async () => {
    const view = await render([])
    expect(view.tabLabels()).toEqual(['Linked phone', 'Meta Cloud API'])
    expect(view.container.textContent).toContain('Link a WhatsApp phone')
    expect(view.container.querySelectorAll('form')).toHaveLength(1)
    expect(view.container.textContent).not.toContain('Connect a WhatsApp account')
    expect(view.container.textContent).not.toContain('Phone messages fixture')
    expect(view.container.textContent).not.toContain('Phone groups fixture')
    expect(view.mutationCalls()).toHaveLength(0)
    await view.click('Meta Cloud API')
    expect(view.container.querySelector('form')).toBeNull()
    expect(view.container.querySelector('[data-cloud]')).not.toBeNull()
  })

  it('creates a phone session only after submission and opens Connection without starting it', async () => {
    const view = await render([])
    await view.typeSessionName('support')
    expect(view.mutationCalls()).toHaveLength(0)
    await view.click('Create session')
    expect(view.mutationCalls()).toEqual([['whatsapp::sessions::create', { session_id: 'support' }]])
    expect(view.container.querySelector('[data-session="support"]')).not.toBeNull()
    expect(view.container.querySelector('[role="tab"][aria-selected="true"]')?.textContent).toBe('Linked phone')
    expect(Array.from(view.container.querySelectorAll('[role="tab"][aria-selected="true"]')).map((item) => item.textContent)).toContain('Connection')
    expect(view.container.querySelector('form')).toBeNull()
  })

  it('preserves the Cloud API view when draft discard is cancelled and switches only on confirmation', async () => {
    const view = await render()
    await view.click('Meta Cloud API')
    await view.click('Draft Cloud message')
    await view.click('Linked phone')
    expect(view.container.querySelector('[role="alertdialog"]')?.textContent).toContain('Keep this draft?')
    await view.click('Keep editing')
    expect(document.activeElement).toBe(view.container.querySelector('[aria-label="Cloud test message"]'))
    expect(view.container.querySelector('[data-cloud]')).not.toBeNull()
    expect(view.container.querySelector('select')).toBeNull()
    await view.click('Linked phone')
    await view.click('Discard draft and switch')
    expect(view.container.querySelector('[data-cloud]')).toBeNull()
    expect(view.container.querySelector('[data-session="support"]')).not.toBeNull()
    expect(view.mutationCalls()).toHaveLength(0)
  })

  it.each(['Keep draft and switch', 'Discard draft and switch'])('focuses the destination after %s and dialog cleanup', async (choice) => {
    const view = await render()
    await view.click('Meta Cloud API')
    await view.click('Draft Cloud message')
    await view.click('Linked phone')
    await view.click(choice)
    const destination = Array.from(view.container.querySelectorAll<HTMLElement>('[role="tab"][data-state="active"]')).at(-1)
    expect(destination?.textContent).toBe('Connection')
    expect(document.activeElement).toBe(destination)
    expect(document.activeElement).not.toBe(document.body)
    expect(view.container.querySelector('[role="alertdialog"]')).toBeNull()
  })

  it('focuses the Cloud destination after keeping a native draft, and keeps the previous context on cancel', async () => {
    const view = await render()
    await view.click('Draft phone message')
    await view.click('Meta Cloud API')
    await view.click('Keep editing')
    expect(document.activeElement).toBe(view.container.querySelector('[aria-label="Native test message"]'))
    expect(view.container.querySelector('[data-session="support"]')).not.toBeNull()
    expect(view.container.querySelector('[data-cloud]')).toBeNull()
    expect(view.container.textContent).toContain('Saved phone draft')
    expect(Array.from(view.container.querySelectorAll('[role="tab"][data-state="active"]')).map((tab) => tab.textContent)).toEqual(['Linked phone', 'Connection'])
    await view.click('Meta Cloud API')
    await view.click('Keep draft and switch')
    const destination = view.container.querySelector('[role="tab"][data-state="active"]')
    expect(destination?.textContent).toBe('Meta Cloud API')
    expect(document.activeElement).toBe(destination)
  })

  it('restores the native initiating control after Escape and preserves its draft', async () => {
    const view = await render()
    await view.click('Draft phone message')
    await view.click('Meta Cloud API')
    const dialog = view.container.querySelector('[role="alertdialog"]')!
    await act(async () => dialog.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })))
    await act(async () => { await vi.advanceTimersByTimeAsync(0) })
    expect(view.container.querySelector('[role="alertdialog"]')).toBeNull()
    expect(view.container.querySelector('[data-session="support"]')).not.toBeNull()
    expect(view.container.querySelector('[data-cloud]')).toBeNull()
    expect(view.container.textContent).toContain('Saved phone draft')
    expect(document.activeElement).toBe(view.container.querySelector('[aria-label="Native test message"]'))
    expect(document.activeElement).not.toBe(document.body)
  })

  it.each(['Escape', 'Keep editing'])('does not reactivate a rejected native tab after keyboard cancellation with %s', async (choice) => {
    const view = await render()
    await view.click('Draft phone message')
    const groupTab = Array.from(view.container.querySelectorAll<HTMLElement>('[role="tab"]')).find((tab) => tab.textContent === 'Groups')!
    await act(async () => groupTab.focus())
    expect(view.container.querySelector('[role="alertdialog"]')).not.toBeNull()
    if (choice === 'Escape') {
      await act(async () => view.container.querySelector('[role="alertdialog"]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })))
    } else await view.click('Keep editing')
    await act(async () => { await vi.advanceTimersByTimeAsync(180) })
    expect(view.container.querySelector('[role="alertdialog"]')).toBeNull()
    expect(view.container.textContent).toContain('Saved phone draft')
    expect(view.container.textContent).not.toContain('Phone groups fixture')
    expect(document.activeElement).toBe(view.container.querySelector('[aria-label="Native test message"]'))
    expect(Array.from(view.container.querySelectorAll('[role="tab"][data-state="active"]')).map((tab) => tab.textContent)).toEqual(['Linked phone', 'Connection'])
    expect(view.mutationCalls()).toHaveLength(0)
  })

  it('blocks switching and removal while a send is pending, and keeps saved drafts protected when closing', async () => {
    const view = await render()
    await view.click('Draft phone message')
    await view.click('Begin send')
    await view.click('Meta Cloud API')
    expect(view.container.querySelector('[data-cloud]')).toBeNull()
    expect(view.container.querySelector('[role="alertdialog"]')).toBeNull()
    expect(view.container.textContent).toContain('Wait for the current request to finish')
    await view.click('Remove session')
    expect(confirm).not.toHaveBeenCalled()
    await view.click('Finish send')
    await view.click('Draft phone message')
    await view.click('Meta Cloud API')
    await view.click('Keep draft and switch')
    expect(view.setDirty).toHaveBeenLastCalledWith('WhatsApp message')
    expect(view.mutationCalls()).toHaveLength(0)
  })

  it('keeps drafts in their transport and discards only the current draft', async () => {
    const view = await render()
    await view.click('Draft phone message')
    await view.click('Meta Cloud API')
    await view.click('Keep draft and switch')
    expect(view.container.textContent).not.toContain('Saved phone draft')
    await view.click('Draft Cloud message')
    await view.click('Linked phone')
    await view.click('Keep draft and switch')
    expect(view.container.textContent).toContain('Saved phone draft')
    expect(view.container.textContent).not.toContain('Saved Cloud draft')
    await view.click('Draft phone message')
    await view.click('Meta Cloud API')
    await view.click('Discard draft and switch')
    expect(view.container.textContent).toContain('Saved Cloud draft')
    await view.click('Draft Cloud message')
    await view.click('Linked phone')
    await view.click('Keep draft and switch')
    expect(view.container.textContent).not.toContain('Saved phone draft')
    expect(view.mutationCalls()).toHaveLength(0)
  })

  it('remembers the phone view when switching between independent transports', async () => {
    const view = await render()
    await view.click('Messages')
    await view.click('Meta Cloud API')
    expect(view.container.textContent).not.toContain('Phone messages fixture')
    await view.click('Linked phone')
    expect(view.container.textContent).toContain('Phone messages fixture')
    expect(view.container.querySelector('[data-cloud]')).toBeNull()
    expect(view.mutationCalls()).toHaveLength(0)
  })

  it('opens the remembered phone tool from a command while Cloud API is active', async () => {
    const view = await render()
    await view.click('Meta Cloud API')
    await view.runCommand('connection')
    expect(view.container.querySelector('[data-cloud]')).toBeNull()
    expect(view.container.querySelector('[data-session="support"]')).not.toBeNull()
    await view.click('Messages')
    await view.click('Meta Cloud API')
    await view.runCommand('messages')
    expect(view.container.querySelector('[data-cloud]')).toBeNull()
    expect(view.container.textContent).toContain('Phone messages fixture')
    expect(view.mutationCalls()).toHaveLength(0)
  })

  it('keeps the current transport while a draft confirmation is pending and blocks competing navigation', async () => {
    const view = await render()
    await view.click('Draft phone message')
    await view.click('Meta Cloud API')
    expect(view.container.querySelector('[data-session="support"]')).not.toBeNull()
    expect(view.container.querySelector('[data-cloud]')).toBeNull()
    await view.runCommand('groups')
    await view.click('Meta Cloud API')
    expect(view.container.querySelectorAll('[role="alertdialog"]')).toHaveLength(1)
    expect(view.container.textContent).not.toContain('Phone groups fixture')
    await view.click('Keep editing')
    expect(view.container.querySelector('[data-session="support"]')).not.toBeNull()
    await view.click('Meta Cloud API')
    await view.click('Keep draft and switch')
    expect(view.container.querySelector('[data-cloud]')).not.toBeNull()
    await view.click('Linked phone')
    expect(view.container.textContent).toContain('Saved phone draft')
    expect(view.mutationCalls()).toHaveLength(0)
  })
})
