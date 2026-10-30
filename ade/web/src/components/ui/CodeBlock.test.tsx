// @vitest-environment jsdom
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Markdown } from '@/lib/markdown'
import { CodeBlock, shellPromptLines } from './CodeBlock'

function render(source: string, language = '') {
  const container = document.createElement('div')
  container.innerHTML = renderToStaticMarkup(
    <Markdown>{`\`\`\`${language}\n${source}\n\`\`\``}</Markdown>,
  )
  return container
}

describe('CodeBlock', () => {
  it('names the language in the header and offers a copy', () => {
    const container = render('cd expense-tracker\nnpm run dev', 'bash')
    const card = container.querySelector('[data-code-block]')
    expect(card?.getAttribute('data-language')).toBe('bash')
    expect(card?.querySelector('.chat-code-block__lang')?.textContent).toBe(
      'bash',
    )
    expect(card?.querySelector('button[aria-label="Copy code"]')).not.toBeNull()
  })

  it('marks shell command lines for the prompt without touching the text', () => {
    const container = render(
      '# install\ncd expense-tracker\nnpm run dev \\\n  --host\n\necho done',
      'bash',
    )
    const card = container.querySelector('[data-code-block]')
    expect(card?.hasAttribute('data-shell')).toBe(true)
    const lines = [...(card?.querySelectorAll('.token-line') ?? [])]
    expect(lines.map((line) => line.hasAttribute('data-prompt'))).toEqual([
      false,
      true,
      true,
      false,
      false,
      true,
    ])
    expect(card?.querySelector('pre code')?.textContent).toBe(
      '# install\ncd expense-tracker\nnpm run dev \\\n  --host\n\necho done',
    )
  })

  it('leaves non-shell fences without prompts', () => {
    const container = render('const a = 1', 'ts')
    const card = container.querySelector('[data-code-block]')
    expect(card?.hasAttribute('data-shell')).toBe(false)
    expect(card?.querySelector('[data-prompt]')).toBeNull()
    expect(card?.querySelector('.token.keyword')?.textContent).toBe('const')
  })

  it('renders a bare fence as a card with an empty label', () => {
    const container = render('plain text')
    const card = container.querySelector('[data-code-block]')
    expect(card).not.toBeNull()
    expect(card?.getAttribute('data-language')).toBeNull()
    expect(card?.querySelector('.chat-code-block__lang')?.textContent).toBe('')
  })

  it('exposes the prompt rule directly', () => {
    expect(shellPromptLines('ls\n# c\n  cont\n')).toEqual([
      true,
      false,
      false,
      false,
    ])
  })

  it('renders standalone', () => {
    const html = renderToStaticMarkup(<CodeBlock code="ls" language="sh" />)
    expect(html).toContain('data-shell')
    expect(html).toContain('data-prompt')
  })
})

describe('Markdown images', () => {
  it('shows a machine path as a file chip, not a broken image', () => {
    const container = document.createElement('div')
    container.innerHTML = renderToStaticMarkup(
      <Markdown>{'![Answer, dark](</Users/me/.context/shot.png>)'}</Markdown>,
    )
    expect(container.querySelector('img')).toBeNull()
    const chip = container.querySelector('[data-image-chip]')
    expect(chip?.textContent).toContain('shot.png')
    expect(chip?.textContent).toContain('Answer, dark')
  })

  it('keeps a web image inline', () => {
    const container = document.createElement('div')
    container.innerHTML = renderToStaticMarkup(
      <Markdown>{'![Logo](https://example.com/logo.png)'}</Markdown>,
    )
    expect(container.querySelector('img.chat-prose-image')).not.toBeNull()
    expect(container.querySelector('[data-image-chip]')).toBeNull()
  })
})
