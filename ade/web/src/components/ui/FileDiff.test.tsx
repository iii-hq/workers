import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { renderToStaticMarkup } from 'react-dom/server'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { FileDiff, resolveFileDiffEditState } from './FileDiff'

const { renderedProps } = vi.hoisted(() => ({
  renderedProps: [] as Array<Record<string, unknown>>,
}))

vi.mock('@pierre/diffs', () => ({
  DEFAULT_THEMES: { light: 'light', dark: 'dark' },
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

describe('FileDiff', () => {
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
