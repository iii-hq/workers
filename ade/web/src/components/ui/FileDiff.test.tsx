// @vitest-environment jsdom

import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { FileDiff, resolveFileDiffEditState } from './FileDiff'

const { renderedProps } = vi.hoisted(() => ({
  renderedProps: [] as Array<Record<string, unknown>>,
}))

vi.mock('@pierre/diffs', () => ({
  DEFAULT_THEMES: { light: 'light', dark: 'dark' },
  registerCustomCSSVariableTheme: vi.fn(),
}))

vi.mock('@pierre/diffs/react', () => ({
  EditProvider: ({ children }: { children?: React.ReactNode }) => (
    <div data-edit-provider="true">{children}</div>
  ),
  MultiFileDiff: (props: Record<string, unknown>) => {
    renderedProps.push(props)
    return <div data-edit={String(props.edit)} />
  },
}))

vi.mock('@/hooks/use-theme', () => ({
  useTheme: () => ['dark', vi.fn()],
}))

const SOURCE = readFileSync(
  join(dirname(fileURLToPath(import.meta.url)), 'FileDiff.tsx'),
  'utf8',
)
const oldFile = { name: 'app.ts', contents: 'const value = 1\n' }
const newFile = { name: 'app.ts', contents: 'const value = 2\n' }

beforeEach(() => {
  renderedProps.length = 0
})

const contextFixtures: Array<() => void> = []

afterEach(() => {
  for (const cleanup of contextFixtures.splice(0)) cleanup()
  document.body.replaceChildren()
})

function contextFixture() {
  const review = document.createElement('section')
  review.tabIndex = -1
  review.setAttribute('aria-label', 'Review pending changes')
  const host = document.createElement('diffs-container')
  review.append(host)
  document.body.append(review)
  const shadow = host.attachShadow({ mode: 'open' })

  renderToStaticMarkup(
    <FileDiff
      oldFile={oldFile}
      newFile={newFile}
      contextExpansionFocusFallback={{ current: review }}
    />,
  )
  const options = renderedProps.at(-1)?.options
  if (
    !options ||
    typeof options !== 'object' ||
    !('onPostRender' in options) ||
    typeof options.onPostRender !== 'function'
  )
    throw new Error('The renderer must receive a post-render hook')
  const hook = options.onPostRender
  const render = (phase = 'update') => hook(host, undefined, phase)
  contextFixtures.push(() => render('unmount'))

  const action = (hunk = '0', direction = 'down', all = false) => {
    const group = document.createElement('div')
    group.setAttribute('data-expand-index', hunk)
    for (const gutter of [true, false]) {
      const wrapper = document.createElement('div')
      if (gutter) wrapper.setAttribute('data-gutter', '')
      const control = document.createElement('div')
      control.setAttribute('data-expand-button', '')
      if (direction !== 'both')
        control.setAttribute(`data-expand-${direction}`, '')
      if (all) control.setAttribute('data-expand-all-button', '')
      wrapper.append(control)
      group.append(wrapper)
    }
    shadow.append(group)
    return group
  }
  const button = (group: HTMLElement) => {
    const control = group.querySelector<HTMLButtonElement>('button')
    if (!control) throw new Error('The expander must be a native button')
    return control
  }
  return { review, host, shadow, render, action, button }
}

describe('FileDiff', () => {
  it('returns focus to the existing review when its focused context action disappears', () => {
    const fixture = contextFixture()
    const group = fixture.action()
    fixture.render('mount')
    const control = fixture.button(group)
    control.focus()
    control.click()
    group.remove()
    const focus = vi.spyOn(fixture.review, 'focus')

    fixture.render()

    expect(document.activeElement).toBe(fixture.review)
    expect(focus).toHaveBeenCalledExactlyOnceWith({ preventScroll: true })
    expect(fixture.review.tabIndex).toBe(-1)
  })

  it('retains the same hunk, direction and action after the renderer replaces its focused button', () => {
    const fixture = contextFixture()
    const group = fixture.action('2', 'up', true)
    fixture.render('mount')
    const control = fixture.button(group)
    control.focus()
    control.click()
    group.remove()
    fixture.action('1', 'up', true)
    fixture.action('2', 'down', true)
    fixture.action('2', 'up', false)
    const equivalent = fixture.action('2', 'up', true)

    fixture.render()

    expect(fixture.shadow.activeElement).toBe(fixture.button(equivalent))
    expect(document.activeElement).toBe(fixture.host)
    expect(fixture.shadow.querySelectorAll('button')).toHaveLength(4)
  })

  it.each(['unfocused-button', 'decorative-control'])(
    'does not move focus after a %s expands context',
    (target) => {
      const fixture = contextFixture()
      const group = fixture.action()
      fixture.render('mount')
      fixture.review.focus()
      const control =
        target === 'unfocused-button'
          ? fixture.button(group)
          : group.querySelector<HTMLElement>('[aria-hidden="true"]')
      if (!control) throw new Error('The physical control must exist')
      control.click()
      group.remove()
      const focus = vi.spyOn(fixture.review, 'focus')

      fixture.render()

      expect(document.activeElement).toBe(fixture.review)
      expect(focus).not.toHaveBeenCalled()
    },
  )

  it.each(['inside', 'outside'])(
    'does not replace newer %s focus while context is rerendered',
    (location) => {
      const fixture = contextFixture()
      const group = fixture.action()
      fixture.render('mount')
      const control = fixture.button(group)
      control.focus()
      control.click()
      group.remove()
      const newer = document.createElement('button')
      ;(location === 'inside' ? fixture.shadow : document.body).append(newer)
      newer.focus()

      fixture.render()

      expect(
        location === 'inside'
          ? fixture.shadow.activeElement
          : document.activeElement,
      ).toBe(newer)
    },
  )

  it('consumes a recovery once and leaves a surviving focused button alone', () => {
    const fixture = contextFixture()
    const group = fixture.action()
    fixture.render('mount')
    const control = fixture.button(group)
    control.focus()
    control.click()
    const focus = vi.spyOn(fixture.review, 'focus')
    fixture.render()
    expect(fixture.shadow.activeElement).toBe(control)
    expect(focus).not.toHaveBeenCalled()

    group.remove()
    fixture.render()
    expect(focus).not.toHaveBeenCalled()

    const next = fixture.action()
    fixture.render()
    fixture.button(next).focus()
    fixture.button(next).click()
    next.remove()
    fixture.render()
    expect(document.activeElement).toBe(fixture.review)
    fixture.review.blur()
    fixture.render()
    expect(focus).toHaveBeenCalledTimes(1)
  })

  it('cleans pending recovery and its click listener on unmount', () => {
    const fixture = contextFixture()
    const group = fixture.action()
    fixture.render('mount')
    const control = fixture.button(group)
    control.focus()
    control.click()
    fixture.render('unmount')
    control.click()
    group.remove()
    const focus = vi.spyOn(fixture.review, 'focus')

    fixture.render()

    expect(focus).not.toHaveBeenCalled()
  })

  it('reports the requested editor lifecycle without treating read-only as loading', () => {
    expect(resolveFileDiffEditState(false, false, false)).toBeNull()
    expect(resolveFileDiffEditState(true, false, false)).toBe('loading')
    expect(resolveFileDiffEditState(true, true, false)).toBe('ready')
    expect(resolveFileDiffEditState(true, false, true)).toBe('error')
  })

  it('preserves the read-only default and existing diff options', () => {
    const html = renderToStaticMarkup(
      <FileDiff
        oldFile={oldFile}
        newFile={newFile}
        diffStyle="split"
        ignoreWhitespace
      />,
    )

    expect(html).toContain('data-edit="false"')
    expect(html).not.toContain('data-edit-provider')
    expect(html).not.toContain('Loading inline editor')
    expect(renderedProps).toHaveLength(1)
    expect(renderedProps[0]).toMatchObject({
      oldFile,
      newFile,
      edit: false,
      editorOptions: undefined,
      options: {
        diffStyle: 'split',
        parseDiffOptions: { ignoreWhitespace: true },
        themeType: 'dark',
      },
    })
  })

  it('keys a read-only diff on its text so changed contents render again', () => {
    renderToStaticMarkup(<FileDiff oldFile={oldFile} newFile={newFile} />)
    renderToStaticMarkup(
      <FileDiff
        oldFile={oldFile}
        newFile={{ ...newFile, contents: 'const value = 3\n' }}
      />,
    )

    const keys = renderedProps.map(
      (props) => (props.newFile as { cacheKey?: string }).cacheKey,
    )
    expect(keys[0]).toBeDefined()
    expect(keys[0]).not.toBe(keys[1])
  })

  it('hands the renderer the same inputs when a parent re-renders with equal values', async () => {
    vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
    const root = createRoot(document.createElement('div'))
    const render = (ignoreWhitespace: boolean) =>
      act(async () =>
        root.render(
          <FileDiff
            oldFile={{ ...oldFile }}
            newFile={{ ...newFile }}
            ignoreWhitespace={ignoreWhitespace}
          />,
        ),
      )

    await render(false)
    await render(false)
    const [first, second] = renderedProps
    // Pierre memoizes the parse on these identities: equal values reuse it.
    expect(second.oldFile).toBe(first.oldFile)
    expect(second.newFile).toBe(first.newFile)
    expect(second.options).toBe(first.options)

    await render(true)
    const third = renderedProps[2] as { options: Record<string, unknown> }
    expect(third.options).not.toBe(first.options)
    expect(third.options.parseDiffOptions).toEqual({ ignoreWhitespace: true })

    await act(async () => root.unmount())
    vi.unstubAllGlobals()
  })

  it('keeps the diff read-only while an explicitly requested editor loads', () => {
    const html = renderToStaticMarkup(
      <FileDiff oldFile={oldFile} newFile={newFile} edit onChange={vi.fn()} />,
    )

    expect(html).toContain('data-edit="false"')
    expect(html).toContain('role="status"')
    expect(html).toContain('Loading inline editor')
    expect(html).not.toContain('data-edit-provider')
  })

  it('loads Pierre editing only through the edit-gated dynamic import', () => {
    const guard = SOURCE.indexOf('if (!edit || EditorConstructor) return')
    const lazyImport = SOURCE.indexOf("import('@pierre/diffs/edit')")

    expect(guard).toBeGreaterThan(-1)
    expect(lazyImport).toBeGreaterThan(guard)
    expect(SOURCE).toContain(
      "import type { Editor, EditorOptions } from '@pierre/diffs/edit'",
    )
    expect(SOURCE).not.toMatch(
      /^import \{ Editor(?:, EditorOptions)? \} from '@pierre\/diffs\/edit'$/m,
    )
    expect(SOURCE).toContain('onEditStateChange?.(editState)')
  })
})
