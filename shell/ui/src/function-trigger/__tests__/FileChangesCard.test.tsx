import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { FileChangesCard } from '../FileChangesCard'
import type { FileChangesSummary } from '../file-changes'

const summary: FileChangesSummary = {
  action: 'updated',
  rows: [
    { path: 'one.ts', status: 'updated', additions: 2 },
    { path: 'two.ts', status: 'updated', additions: 1, deletions: 1 },
    { path: 'three.ts', status: 'updated' },
    { path: 'four.ts', status: 'updated' },
    { path: 'five.ts', status: 'updated' },
    { path: 'six.ts', status: 'failed' },
  ],
}

describe('FileChangesCard', () => {
  it('keeps running and settled titles in one transition slot', () => {
    const running = renderToStaticMarkup(<FileChangesCard summary={summary} running />)
    const settled = renderToStaticMarkup(<FileChangesCard summary={summary} running={false} />)

    expect(running).toContain('data-state="running"')
    expect(running).toContain('Changing 6 files')
    expect(running).toContain('Updated 6 files')
    expect(running).toContain('data-visible="true"')
    expect(settled).toContain('data-state="settled"')
    expect(settled).toContain('Changing 6 files')
    expect(settled).toContain('Updated 6 files')
    expect(settled).toContain('1 failed')
    expect(settled).toContain('aria-label="Updated 6 files"')
  })

  it('keeps overflow rows mounted and inert inside an accordion', () => {
    const html = renderToStaticMarkup(<FileChangesCard summary={summary} running={false} />)

    expect(html).toContain('class="shui-file-changes-overflow"')
    expect(html).toContain('aria-hidden="true"')
    expect(html).toContain('inert=""')
    expect(html).toContain('six.ts')
    expect(html).toContain('Show 1 more file')
    expect(html).toContain('Show fewer files')
    expect(html).toContain('aria-expanded="false"')
  })

  it('does not add a disclosure when every row already fits', () => {
    const html = renderToStaticMarkup(
      <FileChangesCard summary={{ ...summary, rows: summary.rows.slice(0, 5) }} running={false} />,
    )

    expect(html).not.toContain('shui-file-changes-more')
    expect(html).not.toContain('shui-file-changes-overflow')
  })
})
