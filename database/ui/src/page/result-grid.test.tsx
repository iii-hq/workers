import type { ComponentProps, ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { ResultGrid } from './result-grid'

// vi.mock is hoisted above plain declarations; these ride along with it.
const { passthrough, fragment } = vi.hoisted(() => ({
  passthrough:
    (Tag: 'table' | 'thead' | 'tbody' | 'tr' | 'th' | 'td') =>
    ({ children, ...props }: { children?: ReactNode }) => <Tag {...(props as object)}>{children}</Tag>,
  fragment: ({ children }: { children?: ReactNode }) => <>{children}</>,
}))

vi.mock('@iii-dev/console-ui', () => ({
  Button: ({ children }: { children?: ReactNode }) => <button type="button">{children}</button>,
  StatusBar: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
  ShadcnTable: passthrough('table'),
  ShadcnTableHeader: passthrough('thead'),
  ShadcnTableBody: passthrough('tbody'),
  ShadcnTableRow: passthrough('tr'),
  ShadcnTableHead: passthrough('th'),
  ShadcnTableCell: passthrough('td'),
  ShadcnDropdownMenu: fragment,
  ShadcnDropdownMenuTrigger: fragment,
  ShadcnDropdownMenuContent: fragment,
  ShadcnDropdownMenuGroup: fragment,
  ShadcnDropdownMenuLabel: fragment,
  ShadcnDropdownMenuItem: fragment,
  ShadcnDropdownMenuSeparator: () => null,
}))
vi.mock('@iii-dev/console-ui/format', () => ({ copyText: () => Promise.resolve() }))
vi.mock('@iii-dev/console-ui/hooks', () => ({ useCopyFlash: () => ({ state: 'idle', copy: () => {} }) }))

const render = (props: Partial<ComponentProps<typeof ResultGrid>>) =>
  renderToStaticMarkup(<ResultGrid rows={[]} {...props} />)

describe('ResultGrid', () => {
  it('right-aligns an untyped column whose values are all numbers', () => {
    const html = render({ rows: [{ n: 1, s: 'a' }, { n: null, s: 'b' }] })
    expect(html).toMatch(/<td class="num">(?:(?!<\/td>).)*db-cell-num">1</)
    expect(html).toMatch(/<td>(?:(?!<\/td>).)*db-cell-str">a</)
  })

  it('tells an empty string apart from NULL', () => {
    const html = render({ rows: [{ a: '', b: null }] })
    expect(html).toContain('class="db-cell-empty"')
    expect(html).toContain('class="db-cell-null">NULL')
  })

  it('numbers rows from the page offset in a leading gutter', () => {
    const html = render({ rows: [{ a: 'x' }, { a: 'y' }], rowNumbers: true, rowOffset: 50 })
    expect(html).toContain('<td class="db-gutter">51</td>')
    expect(html).toContain('<td class="db-gutter">52</td>')
  })

  it('puts a long value on hover instead of only clipping it', () => {
    const long = 'x'.repeat(60)
    expect(render({ rows: [{ a: long }] })).toContain(`title="${long}"`)
    expect(render({ rows: [{ a: 'short' }] })).not.toContain('title="short"')
  })
})
