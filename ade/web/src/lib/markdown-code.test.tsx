// @vitest-environment jsdom
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Markdown } from './markdown'

function renderCode(source: string, language = '', streaming = false) {
  const container = document.createElement('div')
  container.innerHTML = renderToStaticMarkup(
    <Markdown
      streaming={streaming}
    >{`\`\`\`${language}\n${source}${streaming ? '' : '\n```'}`}</Markdown>,
  )
  return container
}

describe('Markdown code highlighting', () => {
  it.each(['rust', 'Rust'])(
    'highlights %s keywords without changing the source',
    (language) => {
      const source =
        'let mut my_list: Vec<_> = vec![1, 2];\n// keep this comment'
      const container = renderCode(source, language)
      expect(container.querySelector('pre code')?.textContent).toBe(source)
      expect(container.querySelector('.token.keyword')?.textContent).toBe('let')
      expect(
        container.querySelector<HTMLElement>('.token.keyword')?.style.color,
      ).toBe('var(--color-accent)')
      expect(container.querySelector('.token.comment')?.textContent).toBe(
        '// keep this comment',
      )
    },
  )

  it.each([
    ['typescript', 'const value: number = 42;'],
    ['python', 'if True:\n    print("hello")'],
    ['bash', 'if true; then echo "hello"; fi'],
  ])('highlights %s through the shared renderer', (language, source) => {
    const container = renderCode(source, language)
    expect(container.querySelector('pre code')?.textContent).toBe(source)
    expect(container.querySelector('.token.keyword')).not.toBeNull()
  })

  it('preserves JSON highlighting', () => {
    const source = '{"enabled": true}'
    const container = renderCode(source, 'json')
    expect(container.querySelector('pre code')?.textContent).toBe(source)
    expect(container.querySelector('.token.boolean')?.textContent).toBe('true')
  })

  it.each(['', 'text', 'unknown-language'])(
    'keeps %s blocks readable and treats markup as text',
    (language) => {
      const source =
        '<script>alert("literal")</script>\n@fn(engine::echo) #file(src/main.rs)'
      const container = renderCode(source, language)
      expect(container.querySelector('pre code')?.textContent?.trimEnd()).toBe(
        source,
      )
      expect(
        container.querySelector('script, [data-function-id], [data-file-path]'),
      ).toBeNull()
      expect(container.querySelector('.token.keyword')).toBeNull()
    },
  )

  it('highlights incomplete fences while streaming', () => {
    const source = 'let mut values = vec!['
    const container = renderCode(source, 'rust', true)
    expect(container.querySelector('pre code')?.textContent).toBe(source)
    expect(container.querySelector('.token.keyword')?.textContent).toBe('let')
  })

  it('keeps inline code inline and literal', () => {
    const container = document.createElement('div')
    container.innerHTML = renderToStaticMarkup(
      <Markdown>{'Use `let mut values` here.'}</Markdown>,
    )
    expect(container.querySelector('code')?.textContent).toBe('let mut values')
    expect(container.querySelector('pre')).toBeNull()
  })

  it('keeps Mermaid on its diagram path while streaming', () => {
    const container = renderCode('graph LR\n  A --> B', 'mermaid', true)
    expect(container.textContent).toContain(
      'Diagram will render when the message is complete.',
    )
    expect(container.querySelector('.prism-code')).toBeNull()
  })
})
