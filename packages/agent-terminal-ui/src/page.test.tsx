/**
 * The page as the console renders it, statically: the console's runtime
 * components (`@iii-dev/console-ui`, served by the console's import map) and
 * xterm are stand-ins here, so what is checked is this package's own markup
 * and the props it hands the folder picker.
 */
import type { ReactElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const picker = vi.hoisted(() => ({ props: [] as Record<string, unknown>[] }))

vi.mock('@iii-dev/console-ui', () => {
  const Pass = ({ children }: { children?: unknown }) => children as ReactElement
  return {
    Button: ({ children }: { children?: unknown }) => <button type="button">{children as string}</button>,
    PageShell: Pass,
    PageBody: Pass,
    PageMain: Pass,
    PageHeader: ({ title }: { title: string }) => <header>{title}</header>,
    DirectoryPicker: (props: Record<string, unknown>) => {
      picker.props.push(props)
      return (
        <span
          data-testid="directory-picker"
          data-value={String(props.value ?? '')}
          data-default={String(props.defaultDir ?? '')}
          data-disabled={String(Boolean(props.disabled))}
        >
          {String(props.emptyLabel ?? '')}
        </span>
      )
    },
  }
})
vi.mock('@xterm/xterm', () => ({ Terminal: class {} }))
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class {} }))

const { createAgentTerminalPage } = await import('./index')
const { FolderPill } = await import('./folder-pill')

type Registered = { id: string; render: (props: Record<string, unknown>) => ReactElement }

function mount(options: Parameters<typeof createAgentTerminalPage>[0], props: Record<string, unknown>) {
  const pages: Registered[] = []
  const host = {
    iii: { on: () => () => undefined, browserId: 'b1', call: async () => ({}) },
    pages: { register: (page: Registered) => pages.push(page) },
  }
  createAgentTerminalPage(options)(host)
  return { page: pages[0], html: renderToStaticMarkup(pages[0].render({ tabId: 't1', ...props })) }
}

beforeEach(() => {
  picker.props.length = 0
})

describe('the claude page', () => {
  const claude = { worker: 'claude', title: 'claude', description: 'Claude Code', folderPicker: true }

  it("has its own folder pill in the bottom row, starting at the chat's working directory", () => {
    const { html } = mount(claude, { workingDir: '/chat/dir' })
    expect(html).toContain('data-testid="directory-picker"')
    expect(html).toContain('data-value="/chat/dir"')
    // In the status bar, before the billing and font controls.
    const bar = html.slice(html.indexOf('agent-terminal-statusbar'))
    expect(bar).toContain('directory-picker')
    expect(bar.indexOf('directory-picker')).toBeLessThan(bar.indexOf('agent-terminal-font'))
  })

  it('shows the project folder label until the worker names the folder', () => {
    const { html } = mount(claude, { workingDir: null })
    expect(html).toContain('data-value=""')
    expect(html).toContain('Project folder')
  })

  it('locks the pill while a session is starting', () => {
    const { html } = mount(claude, { workingDir: '/chat/dir' })
    expect(html).toContain('data-disabled="true"')
  })
})

describe('the pi page', () => {
  it('has no folder pill: the folder stays opt-in', () => {
    const { html } = mount({ worker: 'pi', title: 'pi', description: 'pi' }, { workingDir: '/chat/dir' })
    expect(html).not.toContain('directory-picker')
    expect(picker.props).toEqual([])
  })
})

describe('FolderPill', () => {
  it('hands the picker the folder, the project folder and a change handler', () => {
    const changes: string[] = []
    renderToStaticMarkup(
      <FolderPill value="/work/a" projectDir="/home/u/proj" disabled={false} onChange={(d) => changes.push(d)} />,
    )
    const props = picker.props.at(-1) as { value: string; defaultDir: string; onChange: (d: string) => void }
    expect(props.value).toBe('/work/a')
    expect(props.defaultDir).toBe('/home/u/proj')
    props.onChange('/work/b')
    expect(changes).toEqual(['/work/b'])
  })

  it('does not restart for the folder it is already in', () => {
    const changes: string[] = []
    renderToStaticMarkup(<FolderPill value="/work/a" disabled={false} onChange={(d) => changes.push(d)} />)
    const props = picker.props.at(-1) as { onChange: (d: string) => void }
    props.onChange('/work/a')
    expect(changes).toEqual([])
  })
})
