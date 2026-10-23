import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { GitFileList } from '../GitFileList'
import type { CommitFile } from '../git-log-window'

const file = (path: string): CommitFile => ({ path, status: 'modified', rel: path, view: path })

const rows = (html: string) => (html.match(/class="shui-(git-file|scm-folder)"/g) ?? []).length

describe('the Git file list', () => {
  it('mounts only the rows in view, as a tree or a flat list, however many files a merge touched', () => {
    // The size of this repository's biggest merge.
    const files = Array.from({ length: 3173 }, (_, index) => file(`src/m${index % 40}/f${index}.ts`))
    for (const grouped of [true, false]) {
      const html = renderToStaticMarkup(
        <GitFileList files={files} prefix="" top="/r" grouped={grouped} onOpen={() => {}} />,
      )
      // Before measuring its viewport the window holds 1 row plus the overscan.
      expect(rows(html)).toBeLessThanOrEqual(9)
    }
  })

  it('lists folders before their files, open unless told otherwise', () => {
    const files = [file('b.ts'), file('src/a.ts')]
    const open = renderToStaticMarkup(<GitFileList files={files} prefix="" top="/r" onOpen={() => {}} />)
    expect(open).toMatch(/aria-expanded="true".*src.*a\.ts.*b\.ts/)
    const closed = renderToStaticMarkup(<GitFileList files={files} prefix="" top="/r" open={false} onOpen={() => {}} />)
    expect(closed).toMatch(/aria-expanded="false".*src.*b\.ts/)
    expect(closed).not.toContain('a.ts')
  })
})
