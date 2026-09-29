// @vitest-environment jsdom

import { act, type ButtonHTMLAttributes, type InputHTMLAttributes, type ReactNode } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { type Host } from '@iii-dev/console-ui'
import { QuickTunnelInstallDialog, WebhookSetup } from './WebhookSetup'

const mocks = vi.hoisted(() => ({ confirm: vi.fn() }))

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true
vi.mock('@iii-dev/console-ui', async () => {
  const React = await import('react')
  const CloseContext = React.createContext<((open: boolean) => void) | undefined>(undefined)
  const Passthrough = ({ children }: { children?: ReactNode }) => <>{children}</>
  return {
    Badge: Passthrough,
    Button: ({ children, ...props }: ButtonHTMLAttributes<HTMLButtonElement>) => (
      <button type="button" {...props}>
        {children}
      </button>
    ),
    Checkbox: ({ label, ...props }: InputHTMLAttributes<HTMLInputElement> & { label?: ReactNode }) => (
      <label>
        <input type="checkbox" {...props} />
        {label}
      </label>
    ),
    Dialog: ({
      open,
      onOpenChange,
      children,
    }: {
      open?: boolean
      onOpenChange?(open: boolean): void
      children?: ReactNode
    }) => (
      <CloseContext.Provider value={onOpenChange}>{open ? children : null}</CloseContext.Provider>
    ),
    DialogClose: ({ children }: { children?: ReactNode; asChild?: boolean }) => {
      const onOpenChange = React.useContext(CloseContext)
      if (!React.isValidElement<ButtonHTMLAttributes<HTMLButtonElement>>(children)) return children
      return React.cloneElement(children, {
        onClick: (event) => {
          children.props.onClick?.(event)
          onOpenChange?.(false)
        },
      })
    },
    DialogContent: Passthrough,
    DialogDescription: Passthrough,
    DialogTitle: Passthrough,
    EmptyState: Passthrough,
    LiveRegion: Passthrough,
    PageMain: Passthrough,
    SettingsList: Passthrough,
    SettingsRow: ({
      label,
      description,
      meta,
      control,
      action,
    }: {
      label?: ReactNode
      description?: ReactNode
      meta?: ReactNode
      control?: ReactNode
      action?: ReactNode
    }) => (
      <div>
        <span>{label}</span>
        <span>{description}</span>
        {meta}
        {control}
        {action}
      </div>
    ),
    SettingsSection: ({
      title,
      description,
      action,
      children,
    }: {
      title?: ReactNode
      description?: ReactNode
      action?: ReactNode
      children?: ReactNode
    }) => (
      <section>
        <h2>{title}</h2>
        <p>{description}</p>
        {action}
        {children}
      </section>
    ),
    uiClasses: { spin: 'spin' },
    useConfirm: () => ({ confirm: mocks.confirm, dialog: null }),
  }
})

vi.mock('@iii-dev/console-ui/format', () => ({
  errorMessage: (error: unknown) => String(error),
}))

let root: Root | undefined

function mount(onInstall = vi.fn(), onOpenChange = vi.fn()) {
  const container = document.body.appendChild(document.createElement('div'))
  root = createRoot(container)
  act(() =>
    root!.render(
      <QuickTunnelInstallDialog open onOpenChange={onOpenChange} onInstall={onInstall} />,
    ),
  )
  return { container, onInstall, onOpenChange }
}

function button(container: HTMLElement, label: string) {
  return [...container.querySelectorAll<HTMLButtonElement>('button')].find(
    (candidate) => candidate.textContent === label,
  )!
}

function normalizedText(container: HTMLElement) {
  return container.textContent?.replace(/\s+/g, ' ').trim()
}

afterEach(() => {
  if (root) act(() => root!.unmount())
  root = undefined
  document.body.innerHTML = ''
  mocks.confirm.mockReset()
  vi.restoreAllMocks()
})

describe('QuickTunnelInstallDialog', () => {
  it('shows the requested webhook benefit, cloudflared warning, and official guide', () => {
    const { container } = mount()
    expect(normalizedText(container)).toContain(
      'Would you like to add Webhook support so your agents can react to Github events?',
    )
    expect(normalizedText(container)).toContain(
      'Clicking install will install the quick-tunnel worker to this project which supports cloudflared. You must first install cloudflared on this computer before proceeding.',
    )
    expect(container.querySelector('label')?.textContent).toBe('I have installed cloudflared')
    expect(container.querySelector<HTMLAnchorElement>('a')?.href).toBe(
      'https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/',
    )
  })

  it('keeps Install disabled until cloudflared is confirmed and installs only once', () => {
    const { container, onInstall } = mount()
    const install = button(container, 'Install')
    const checkbox = container.querySelector<HTMLInputElement>('input[type="checkbox"]')!
    expect(install.disabled).toBe(true)
    act(() => checkbox.click())
    expect(install.disabled).toBe(false)
    act(() => {
      install.click()
      install.click()
    })
    expect(onInstall).toHaveBeenCalledTimes(1)
  })

  it('closes without installing when cancelled', () => {
    const { container, onInstall, onOpenChange } = mount()
    act(() => button(container, 'Cancel').click())
    expect(onOpenChange).toHaveBeenCalledExactlyOnceWith(false)
    expect(onInstall).not.toHaveBeenCalled()
  })
})

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

async function settle(ms = 0) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms))
  })
}

const updateHttpStatus = {
  enabled: false,
  active: false,
  ready: false,
  restart_required: false,
  checks: [
    { id: 'quick_tunnel', title: 'quick-tunnel worker', state: 'ok', detail: 'Installed.' },
    { id: 'cloudflared', title: 'cloudflared binary', state: 'ok', detail: 'Found.' },
    {
      id: 'http_listener',
      title: 'http webhook listener',
      state: 'unknown',
      detail: 'This http version has no webhook listener; update it to 0.22 or later.',
      fix: {
        kind: 'update_worker',
        worker: 'http',
        command: 'compose::update { worker: "http" }',
      },
    },
  ],
} as const

const installQuickTunnelStatus = {
  enabled: false,
  active: false,
  ready: false,
  restart_required: false,
  checks: [
    {
      id: 'quick_tunnel',
      title: 'quick-tunnel worker',
      state: 'missing',
      detail: 'Install quick-tunnel.',
      fix: {
        kind: 'install_worker',
        worker: 'quick-tunnel',
        command: 'compose::add { worker: "quick-tunnel" }',
      },
    },
    { id: 'cloudflared', title: 'cloudflared binary', state: 'blocked', detail: 'Waiting.' },
    { id: 'http_listener', title: 'http webhook listener', state: 'blocked', detail: 'Waiting.' },
  ],
} as const

function mountSetup(trigger: Host['iii']['trigger']) {
  const container = document.body.appendChild(document.createElement('div'))
  root = createRoot(container)
  act(() => root!.render(<WebhookSetup host={{ iii: { trigger } } as Host} />))
  return container
}

describe('WebhookSetup worker updates', () => {
  it('installs quick-tunnel with compose::add, then follows and re-checks the operation', async () => {
    const operation = deferred<{ status: string }>()
    const trigger = vi.fn((functionId: string) => {
      if (functionId === 'github::setup::webhooks-status') {
        return Promise.resolve(installQuickTunnelStatus)
      }
      if (functionId === 'compose::add') return Promise.resolve({ changed: true })
      if (functionId === 'compose::operation') return operation.promise
      return Promise.reject(new Error(`Unexpected function: ${functionId}`))
    })
    const container = mountSetup(trigger as Host['iii']['trigger'])

    await settle(550)
    act(() => button(container, 'Install quick-tunnel').click())
    const checkbox = container.querySelector<HTMLInputElement>('input[type="checkbox"]')!
    expect(button(container, 'Install').disabled).toBe(true)
    act(() => checkbox.click())
    act(() => button(container, 'Install').click())
    await settle()

    expect(trigger).toHaveBeenCalledWith(
      'compose::add',
      expect.objectContaining({
        workers: ['quick-tunnel'],
        operation_id: expect.stringMatching(/^github-install-quick-tunnel-/),
      }),
      { timeoutMs: 60_000 },
    )
    expect(trigger.mock.calls.some(([functionId]) => functionId === 'compose::install')).toBe(false)

    operation.resolve({ status: 'succeeded' })
    await settle(550)
    expect(
      trigger.mock.calls.filter(([functionId]) => functionId === 'github::setup::webhooks-status'),
    ).toHaveLength(2)
  })

  it('offers and follows an http update, then re-checks the prerequisites', async () => {
    const operation = deferred<{ status: string }>()
    const trigger = vi.fn((functionId: string) => {
      if (functionId === 'github::setup::webhooks-status') return Promise.resolve(updateHttpStatus)
      if (functionId === 'compose::update') return Promise.resolve({ changed: true })
      if (functionId === 'compose::operation') return operation.promise
      return Promise.reject(new Error(`Unexpected function: ${functionId}`))
    })
    mocks.confirm.mockResolvedValue(true)
    const container = mountSetup(trigger as Host['iii']['trigger'])

    await settle(550)
    const update = button(container, 'Update http')
    expect(update).toBeTruthy()
    expect(container.textContent).not.toContain('compose::update')

    act(() => update.click())
    await settle()
    expect(mocks.confirm).toHaveBeenCalledWith({
      title: 'Update the http worker?',
      description:
        'The http worker restarts while Compose applies its latest compatible version, so its calls may be briefly unavailable.',
      confirmLabel: 'Update http',
    })
    expect(button(container, 'Updating…').disabled).toBe(true)
    expect(button(container, 'Updating…').getAttribute('aria-busy')).toBe('true')
    expect(trigger).toHaveBeenCalledWith(
      'compose::update',
      expect.objectContaining({
        workers: ['http'],
        operation_id: expect.stringMatching(/^github-update-http-/),
      }),
      { timeoutMs: 60_000 },
    )

    operation.resolve({ status: 'succeeded' })
    await settle(550)
    expect(
      trigger.mock.calls.filter(([functionId]) => functionId === 'github::setup::webhooks-status'),
    ).toHaveLength(2)
    expect(button(container, 'Update http').disabled).toBe(false)
  })


  it('does not start an update when confirmation is cancelled', async () => {
    const trigger = vi.fn((functionId: string) => {
      if (functionId === 'github::setup::webhooks-status') return Promise.resolve(updateHttpStatus)
      return Promise.reject(new Error(`Unexpected function: ${functionId}`))
    })
    mocks.confirm.mockResolvedValue(false)
    const container = mountSetup(trigger as Host['iii']['trigger'])

    await settle(550)
    act(() => button(container, 'Update http').click())
    await settle()

    expect(mocks.confirm).toHaveBeenCalledOnce()
    expect(trigger).not.toHaveBeenCalledWith(
      'compose::update',
      expect.anything(),
      expect.anything(),
    )
    expect(button(container, 'Update http').disabled).toBe(false)
  })

  it.each(['failed', 'cancelled'])('reports a %s update outcome', async (status) => {
    const trigger = vi.fn((functionId: string) => {
      if (functionId === 'github::setup::webhooks-status') return Promise.resolve(updateHttpStatus)
      if (functionId === 'compose::update') return Promise.resolve({ changed: true })
      if (functionId === 'compose::operation') {
        return Promise.resolve({ status, last_event: { detail: 'worker unavailable' } })
      }
      return Promise.reject(new Error(`Unexpected function: ${functionId}`))
    })
    mocks.confirm.mockResolvedValue(true)
    const container = mountSetup(trigger as Host['iii']['trigger'])

    await settle(550)
    act(() => button(container, 'Update http').click())
    await settle()

    expect(container.textContent).toContain(`Updating http ${status}: worker unavailable`)
    expect(button(container, 'Update http').disabled).toBe(false)
  })


  it('stops polling and reports when the update times out', async () => {
    const clock = { now: 0 }
    const trigger = vi.fn((functionId: string) => {
      if (functionId === 'github::setup::webhooks-status') return Promise.resolve(updateHttpStatus)
      if (functionId === 'compose::update') return Promise.resolve({ changed: true })
      if (functionId === 'compose::operation') {
        clock.now = 600_001
        return Promise.resolve({ status: 'running' })
      }
      return Promise.reject(new Error(`Unexpected function: ${functionId}`))
    })
    mocks.confirm.mockResolvedValue(true)
    const container = mountSetup(trigger as Host['iii']['trigger'])

    await settle(550)
    vi.spyOn(Date, 'now').mockImplementation(() => clock.now)
    act(() => button(container, 'Update http').click())
    await settle()

    expect(container.textContent).toContain(
      'Updating http is still running after 10 minutes; check compose logs.',
    )
    expect(button(container, 'Update http').disabled).toBe(false)
  })
})
