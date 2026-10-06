import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { VirtualList } from '../VirtualList'

const rows = Array.from({ length: 100 }, (_, index) => index)

function mounted(keepIndex: number | null): number[] {
  const html = renderToStaticMarkup(
    <VirtualList
      rows={rows}
      rowHeight={20}
      rowKey={(row) => String(row)}
      keepIndex={keepIndex}
      renderRow={(row) => <i>{row}</i>}
    />,
  )
  return [...html.matchAll(/<i>(\d+)<\/i>/g)].map((match) => Number(match[1]))
}

describe('the virtual list', () => {
  it('keeps a row outside the window mounted with its neighbours, in order', () => {
    // Before measuring its viewport the window is row 0 plus the overscan.
    const window = [0, 1, 2, 3, 4, 5, 6, 7, 8]
    expect(mounted(null)).toEqual(window)
    // Tab and Shift+Tab from the kept row land on a mounted neighbour.
    expect(mounted(50)).toEqual([...window, 49, 50, 51])
    expect(mounted(99)).toEqual([...window, 98, 99])
    // At the window's edge only what is missing joins it.
    expect(mounted(9)).toEqual([...window, 9, 10])
  })
})
