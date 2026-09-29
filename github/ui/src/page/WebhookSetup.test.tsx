// @vitest-environment jsdom

import { act, type ButtonHTMLAttributes, type InputHTMLAttributes, type ReactNode } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { QuickTunnelInstallDialog } from './WebhookSetup'

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
    LiveRegion: () => null,
    PageMain: Passthrough,
    SettingsList: Passthrough,
    SettingsRow: () => null,
    SettingsSection: Passthrough,
    uiClasses: { spin: 'spin' },
    useConfirm: () => ({ confirm: vi.fn(), dialog: null }),
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
