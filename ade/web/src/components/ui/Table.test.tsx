import { readFileSync } from 'node:fs'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import {
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from './Table'

describe('Table', () => {
  it('composes responsive semantic table markup with shared recipes', () => {
    const html = renderToStaticMarkup(
      <TableViewport className="custom-viewport">
        <TableFrame>
          <Table density="compact" aria-label="Example fields">
            <TableHeader>
              <TableRow>
                <TableHead>Field</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow interactive selected>
                <TableCell>name</TableCell>
              </TableRow>
            </TableBody>
          </Table>
        </TableFrame>
      </TableViewport>,
    )

    // The shadcn table parts, under the console's viewport and frame.
    expect(html).toContain('data-slot="table-container"')
    expect(html).toContain('custom-viewport')
    expect(html).toContain('rounded-lg border border-border')
    expect(html).toContain('data-density="compact"')
    expect(html).toContain('data-interactive="true"')
    expect(html).toContain('data-selected="true"')
    expect(html).toContain('data-state="selected"')
    expect(html).toContain('data-slot="table-header"')
    expect(html).toMatch(/<th [^>]*data-slot="table-head"[^>]*>Field<\/th>/)
    expect(html).toMatch(/<td [^>]*data-slot="table-cell"[^>]*>name<\/td>/)
  })

  it('keeps the shared visual sans, natural-case, and horizontally divided', () => {
    const css = readFileSync(
      new URL('../../styles/ui-recipes.css', import.meta.url),
      'utf8',
    )
    const tableCss = css.slice(
      css.indexOf('.iii-ui-table-viewport'),
      css.indexOf('.iii-ui-tabs-list'),
    )

    expect(tableCss).toContain('font-family: var(--font-sans)')
    expect(tableCss).toContain('border-bottom: 1px solid var(--color-edge)')
    expect(tableCss).not.toContain('text-transform:')
    expect(tableCss).not.toContain('border-left:')
    expect(tableCss).not.toContain('border-right:')
  })
})
