// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { List, ListItem } from './List'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

let host: HTMLDivElement
let root: Root
beforeEach(() => {
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})
afterEach(() => {
  act(() => root.unmount())
  host.remove()
})

const rows = () => [...host.querySelectorAll<HTMLElement>('[data-list-item]')]
const stops = () => rows().filter((row) => row.tabIndex === 0)
const key = (target: Element, name: string) =>
  act(() => {
    target.dispatchEvent(
      new KeyboardEvent('keydown', { key: name, bubbles: true }),
    )
  })

function render(selected: string | null, names = ['a', 'b', 'c', 'd']) {
  act(() =>
    root.render(
      <List>
        {names.map((name) => (
          <ListItem key={name} selected={name === selected} label={name} />
        ))}
      </List>,
    ),
  )
}

describe('List', () => {
  // QA 2026-11-16 F3: a fifteen-row scopes list took fifteen Tab presses to
  // leave. A list is one Tab stop; the arrows move inside it.
  it('is one Tab stop: the selected row, else the first', () => {
    render(null)
    expect(stops()).toEqual([rows()[0]])
    render('c')
    expect(stops()).toEqual([rows()[2]])
  })

  it('moves its Tab stop with the focus, so Tab leaves from where you are', () => {
    render('a')
    act(() => rows()[0].focus())
    key(rows()[0], 'ArrowDown')
    expect(document.activeElement).toBe(rows()[1])
    expect(stops()).toEqual([rows()[1]])
    key(rows()[1], 'End')
    expect(document.activeElement).toBe(rows()[3])
    expect(stops()).toEqual([rows()[3]])
  })

  it('keeps exactly one stop when rows come and go', () => {
    render('a')
    act(() => rows()[3].focus())
    render('a', ['a', 'b'])
    expect(stops()).toEqual([rows()[0]])
  })

  it('leaves a row with its own tabIndex to its caller', () => {
    act(() =>
      root.render(
        <List>
          <ListItem label="a" />
          <ListItem label="b" tabIndex={0} />
        </List>,
      ),
    )
    expect(rows()[1].tabIndex).toBe(0)
    expect(rows()[0].tabIndex).toBe(0)
  })
})
