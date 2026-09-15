import { readFileSync } from 'node:fs'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { SheetPage } from '@/components/ui/SheetNavigation'
import { DirectoryPicker } from './DirectoryPicker'

vi.mock('@/lib/backend/projects', () => ({
  listHarnessProjects: vi.fn().mockResolvedValue([]),
  upsertHarnessProject: vi.fn(),
  deleteHarnessProject: vi.fn(),
}))

describe('DirectoryPicker narrow layout', () => {
  it('allows the embedded page and picker to shrink below path width', () => {
    const html = renderToStaticMarkup(
      <SheetPage
        title="Working directory"
        dialogSemantics={false}
        contentClassName="overflow-hidden"
      >
        <DirectoryPicker
          presentation="embedded"
          value={null}
          onChange={vi.fn()}
          defaultDir={`/workspace/${'long-project-name'.repeat(20)}`}
        />
      </SheetPage>,
    )
    expect(html).toContain('flex min-h-0 min-w-0 flex-1 flex-col')
    expect(html).toContain(
      'min-h-0 min-w-0 flex-1 overscroll-contain overflow-hidden',
    )
    expect(html).toContain('flex h-full min-h-0 min-w-0 w-full flex-col')
    expect(html).toContain('overflow-x-hidden overflow-y-auto')
  })

  it('keeps mobile scrolling inside the list and wraps long errors', () => {
    const source = readFileSync(
      new URL('./DirectoryPicker.tsx', import.meta.url),
      'utf8',
    )
    expect(
      source.match(/embedded \|\| mobileSheet \? 'min-h-0 flex-1'/g),
    ).toHaveLength(3)
    expect(source).toContain('[overflow-wrap:anywhere]')
    expect(source).toContain('h-[min(36rem,calc(100dvh-1.5rem))]')
  })
})
