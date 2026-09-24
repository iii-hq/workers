// @vitest-environment jsdom

import type { ExtensionIii } from '@iii-dev/console-ui'
import { act, type ReactNode, useEffect } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { JudgeSessionPicker, SESSION_PROVIDER_KEY } from './index'

// The console provides these via its import map; the menu opens on mount and
// a click on a radio row picks its value.
vi.mock('@iii-dev/console-ui', () => ({
  DropdownMenu: ({ children, onOpenChange }: { children: ReactNode; onOpenChange?(open: boolean): void }) => {
    // Open once, on mount (the real menu opens on a click).
    // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only open
    useEffect(() => onOpenChange?.(true), [])
    return <div>{children}</div>
  },
  DropdownMenuTrigger: ({ children, ...props }: { children: ReactNode }) => <button {...props}>{children}</button>,
  DropdownMenuContent: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DropdownMenuLabel: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DropdownMenuSeparator: () => <hr />,
  DropdownMenuItem: ({ children }: { children: ReactNode }) => <div data-item>{children}</div>,
  DropdownMenuRadioGroup: ({ children, value, onValueChange }: { children: ReactNode; value?: string; onValueChange?(v: string): void }) => (
    <div
      data-value={value}
      onClick={(event) => {
        const picked = (event.target as HTMLElement).dataset.pick
        if (picked !== undefined) onValueChange?.(picked)
      }}
    >
      {children}
    </div>
  ),
  DropdownMenuRadioItem: ({ children, value }: { children: ReactNode; value: string }) => (
    <button type="button" data-pick={value}>
      {children}
    </button>
  ),
}))

let root: Root | null = null
let host: HTMLElement | null = null
afterEach(() => {
  act(() => root?.unmount())
  host?.remove()
  root = null
  host = null
})

const engine = {
  trigger: vi.fn(async () => ({
    functions: [
      { function_id: 'judge-typesafe::evaluate', worker_name: 'judge-typesafe' },
      { function_id: 'judge-semif::evaluate', worker_name: 'judge-semif' },
    ],
  })),
} as unknown as Pick<ExtensionIii, 'trigger'>

async function mount(metadata: Record<string, unknown>, setMetadata: (patch: Record<string, unknown>) => void) {
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () => {
    root!.render(
      <JudgeSessionPicker iii={engine} sessionId="s1" isStreaming={false} metadata={metadata} setMetadata={setMetadata} />,
    )
  })
  return host
}

describe('per-session judge provider', () => {
  it('shows the session value and writes only this session\'s metadata key', async () => {
    const setMetadata = vi.fn()
    const view = await mount({ [SESSION_PROVIDER_KEY]: 'typesafe', model: 'm' }, setMetadata)
    expect(view.querySelector('button[aria-label]')?.textContent).toBe('judge · typesafe')
    const semif = view.querySelector<HTMLElement>('[data-pick="semif"]')
    expect(semif?.textContent).toBe('semif')
    await act(async () => semif!.click())
    expect(setMetadata).toHaveBeenLastCalledWith({ [SESSION_PROVIDER_KEY]: 'semif' })
    await act(async () => view.querySelector<HTMLElement>('[data-pick=""]')!.click())
    expect(setMetadata).toHaveBeenLastCalledWith({ [SESSION_PROVIDER_KEY]: undefined })
  })

  it('keeps a stored provider that is not running selectable', async () => {
    const view = await mount({ [SESSION_PROVIDER_KEY]: 'laya' }, vi.fn())
    expect(view.querySelector('[data-pick="laya"]')?.textContent).toBe('laya (not running)')
  })
})
