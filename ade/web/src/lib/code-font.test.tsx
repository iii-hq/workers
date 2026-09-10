import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { CodeEditor } from '@/components/ui/CodeEditor'
import { FileDiff } from '@/components/ui/FileDiff'
import { Markdown } from '@/lib/markdown'
import { CodeHighlight } from '@/lib/syntax'

describe('code font boundaries', () => {
  it('uses font-code for inline and fenced chat code without changing prose', () => {
    const html = renderToStaticMarkup(
      <Markdown>
        {'Regular prose with `inline()` code.\n\n```ts\nrun()\n```'}
      </Markdown>,
    )

    expect(html).toContain('font-mono text-[14px]')
    expect(html).toContain('font-code text-[12.5px]')
  })

  it('uses font-code in shared highlighted code', () => {
    const html = renderToStaticMarkup(
      <CodeHighlight code="const ready = true" language="javascript" />,
    )

    expect(html).toContain('font-code')
  })

  it('uses font-code in the editor fallback before Monaco loads', () => {
    const html = renderToStaticMarkup(
      <CodeEditor
        value="const ready = true"
        onChange={() => {}}
        language="typescript"
      />,
    )

    expect(html).toContain('font-code')
  })

  it('passes the shared code font token into the diff renderer', () => {
    const html = renderToStaticMarkup(
      <FileDiff
        oldFile={{ name: 'example.ts', contents: 'const ready = false' }}
        newFile={{ name: 'example.ts', contents: 'const ready = true' }}
      />,
    )

    expect(html).toContain('--diffs-font-family:var(--font-code)')
  })
})
