import type { ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { type BrowserDownload, navigateBrowser } from '../lib/browser'
import { newDownloadGuids } from './DownloadsPanel'
import { PageMenu, type PageMenuActions } from './PageMenu'

vi.mock('@iii-dev/console-ui', () => ({
  DropdownMenu: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
  DropdownMenuTrigger: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
  DropdownMenuContent: ({ children }: { children?: ReactNode }) => <div role="menu">{children}</div>,
  DropdownMenuItem: ({ children }: { children?: ReactNode }) => (
    <button type="button" role="menuitem">
      {children}
    </button>
  ),
  DropdownMenuSeparator: () => <hr />,
  EmptyState: () => null,
  IconButton: () => null,
  StatusPanel: () => null,
}))

const download = (guid: string): BrowserDownload => ({
  guid,
  file_name: `${guid}.bin`,
  url: `https://example.com/${guid}.bin`,
  state: 'in_progress',
  received_bytes: 0,
  total_bytes: 0,
  started_ms: 0,
})

describe('downloads reach the person', () => {
  it('the ⋮ menu has a Downloads row', () => {
    const actions = new Proxy({}, { get: () => () => {} }) as PageMenuActions
    const html = renderToStaticMarkup(
      <PageMenu actions={actions} zoom={100} devtoolsOpen={false} canSendToChat />,
    )
    expect(html).toContain('<button type="button" role="menuitem">Downloads</button>')
  })

  it('a tab’s first list is history; later guids are new downloads', () => {
    expect(newDownloadGuids(null, [download('a')])).toEqual([])
    expect(newDownloadGuids(new Set(['a']), [download('a')])).toEqual([])
    expect(newDownloadGuids(new Set(['a']), [download('a'), download('b')])).toEqual(['b'])
    // One removed and another begun: the count is the same, the file is new.
    expect(newDownloadGuids(new Set(['a']), [download('c')])).toEqual(['c'])
  })

  it('navigate reports a url that downloaded instead of loading', async () => {
    const iii = {
      call: vi.fn(async () => ({
        ok: true,
        url: 'https://example.com/',
        timed_out: false,
        download: { guid: 'g1', file_name: '1Mb.dat' },
      })),
    }
    const result = await navigateBrowser(iii as never, 's1', 'https://proof.ovh.net/files/1Mb.dat')
    expect(result?.download).toEqual({ guid: 'g1', file_name: '1Mb.dat' })
    expect(result?.url).toBe('https://example.com/')
  })
})
