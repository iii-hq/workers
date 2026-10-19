import type { ComponentProps, ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { rawFilter } from '../lib/rpc'
import { FilterBar } from './FilterBar'

vi.mock('@iii-dev/console-ui', () => ({
  Button: ({
    children,
    variant: _v,
    size: _s,
    ...props
  }: ComponentProps<'button'> & { variant?: string; size?: string }) => (
    <button type="button" {...props}>
      {children}
    </button>
  ),
  DropdownMenu: ({ children }: { children?: ReactNode }) => <>{children}</>,
  DropdownMenuTrigger: ({ children }: { children?: ReactNode }) => <>{children}</>,
  DropdownMenuContent: ({ children }: { children?: ReactNode }) => <>{children}</>,
  DropdownMenuItem: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
  Input: ({
    onChange: _onChange,
    preserveCase: _p,
    ...props
  }: { onChange?: (next: string) => void; preserveCase?: boolean } & Omit<ComponentProps<'input'>, 'onChange'>) => (
    <input readOnly {...props} />
  ),
}))

const columns = [
  { name: 'status', type: 'text' },
  { name: 'total', type: 'numeric' },
]

const render = (props: Partial<ComponentProps<typeof FilterBar>>) =>
  renderToStaticMarkup(<FilterBar columns={columns} filters={[]} onChange={() => {}} {...props} />)

describe('FilterBar', () => {
  it('offers a raw SQL chip only when the worker takes one', () => {
    expect(render({ allowRaw: true })).toContain('>SQL</button>')
    expect(render({})).not.toContain('>SQL</button>')
  })

  it('paints a raw condition with the SQL lexer under a transparent input', () => {
    const html = render({ allowRaw: true, filters: [rawFilter("total > 100 and status <> 'void'")] })

    expect(html).toContain('class="db-chip raw"')
    expect(html).toContain('<span class="db-tok-ident">total</span>')
    expect(html).toContain('<span class="db-tok-number">100</span>')
    expect(html).toContain('<span class="db-tok-keyword">and</span>')
    expect(html).toContain('<span class="db-tok-string">&#x27;void&#x27;</span>')
    expect(html).toContain('aria-label="SQL condition — Enter applies"')
  })

  it('marks an empty raw chip as a draft and counts it', () => {
    const html = render({ allowRaw: true, filters: [rawFilter()] })

    expect(html).toContain('class="db-chip raw draft"')
    expect(html).toContain('0 applied · 1 draft')
  })
})
