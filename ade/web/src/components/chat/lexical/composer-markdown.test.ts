// @vitest-environment jsdom

import { $isCodeNode } from '@lexical/code-core'
import { $isListItemNode, $isListNode } from '@lexical/list'
import {
  $createParagraphNode,
  $createTextNode,
  $getRoot,
  $isParagraphNode,
  createEditor,
  type LexicalEditor,
} from 'lexical'
import { describe, expect, it } from 'vitest'
import {
  $composerNodesFromMarkdown,
  $exportComposerMarkdown,
  $fillComposerMarkdown,
  COMPOSER_MARKDOWN_NODES,
  looksLikeComposerMarkdown,
} from './composer-markdown'
import { FileMentionNode } from './FileMentionNode'
import { FunctionMentionNode } from './FunctionMentionNode'
import { SlashCommandNode } from './SlashCommandNode'

function headless(): LexicalEditor {
  return createEditor({
    namespace: 'test',
    nodes: [
      ...COMPOSER_MARKDOWN_NODES,
      FunctionMentionNode,
      FileMentionNode,
      SlashCommandNode,
    ],
    onError(error) {
      throw error
    },
  })
}

/** Read `text` in and write it back out. */
function roundTrip(text: string): string {
  const editor = headless()
  editor.update(() => $fillComposerMarkdown(text), { discrete: true })
  return editor.getEditorState().read(() => $exportComposerMarkdown())
}

describe('composer markdown round trip', () => {
  it.each([
    ['prose', 'hello world'],
    ['blank lines between paragraphs', 'one\n\ntwo\n\n\nthree'],
    ['a bullet list', '- a\n- b\n- c'],
    ['a numbered list', '1. a\n2. b\n3. c'],
    ['a numbered list starting late', '4. d\n5. e'],
    ['a nested bullet list', '- a\n  - b\n    - c\n- d'],
    ['a nested list under a number', '1. a\n   - b\n   - c\n2. d'],
    ['a soft break inside an item', '- a\n  continued\n- b'],
    ['a list then prose', '- a\n- b\n\nthen prose'],
    ['prose then a list', 'intro\n- a\n- b'],
    ['a bullet list then a numbered one', '- a\n1. b'],
    ['a fenced block', '```\nlet a = 1\nlet b = 2\n```'],
    ['a fenced block with a language', '```ts\nconst x: number = 1\n```'],
    ['a fenced block with blank lines', '```\na\n\nb\n```'],
    ['an empty fenced block', '```js\n```'],
    ['a fence containing backticks', '````\n```\ninner\n```\n````'],
    ['inline code', 'run `npm test` now'],
    ['inline code with a backtick', 'the ``a`b`` span'],
    ['inline code starting with a backtick', 'the `` `a` `` span'],
    ['inline code inside an item', '- run `make`\n- done'],
    [
      'everything',
      'Fix this:\n- step `one`\n- step two\n  - detail\n\n```sh\nnpm test\n```\nthanks',
    ],
  ])('%s', (_, text) => {
    expect(roundTrip(text)).toBe(text)
  })

  it('never escapes or unescapes prose punctuation', () => {
    const text = 'match /\\.(ts|tsx)$/ and snake_case *not bold* \\( \\* &#32;'
    expect(roundTrip(text)).toBe(text)
  })

  it('keeps a `/skill:` pill and mention tokens as their text', () => {
    const text =
      '- use /skill:review here\n- see @fn(a::b) and #file(src/x.ts:1-2)'
    expect(roundTrip(text)).toBe(text)
  })

  it('leaves an unbalanced backtick alone', () => {
    expect(roundTrip('a ` b')).toBe('a ` b')
    expect(roundTrip('a `` b')).toBe('a `` b')
  })
})

describe('composer markdown structure', () => {
  it('builds lists, code blocks and paragraphs', () => {
    const editor = headless()
    editor.update(
      () => $fillComposerMarkdown('intro\n- a\n  - b\n```py\nprint(1)\n```'),
      { discrete: true },
    )
    editor.getEditorState().read(() => {
      const [p, list, code] = $getRoot().getChildren()
      expect($isParagraphNode(p)).toBe(true)
      expect($isListNode(list)).toBe(true)
      if (!$isListNode(list)) throw new Error('unreachable')
      const [a, wrapper] = list.getChildren()
      expect($isListItemNode(a) && a.getTextContent()).toBe('a')
      expect(
        $isListItemNode(wrapper) && $isListNode(wrapper.getFirstChild()),
      ).toBe(true)
      expect($isCodeNode(code)).toBe(true)
      if (!$isCodeNode(code)) throw new Error('unreachable')
      expect(code.getLanguage()).toBe('py')
      expect(code.getTextContent()).toBe('print(1)')
    })
  })

  it('marks inline code as the code text format', () => {
    const editor = headless()
    editor.update(() => $fillComposerMarkdown('run `x` now'), {
      discrete: true,
    })
    editor.getEditorState().read(() => {
      const texts = $getRoot().getAllTextNodes()
      expect(
        texts.map((t) => [t.getTextContent(), t.hasFormat('code')]),
      ).toEqual([
        ['run ', false],
        ['x', true],
        [' now', false],
      ])
    })
  })

  it('always yields at least one paragraph', () => {
    const editor = headless()
    editor.update(
      () => {
        expect($composerNodesFromMarkdown('')).toHaveLength(1)
      },
      { discrete: true },
    )
  })

  it('exports a code-formatted run as one span and closes an unterminated fence', () => {
    const editor = headless()
    editor.update(
      () => {
        const p = $createParagraphNode()
        p.append(
          $createTextNode('a'),
          $createTextNode('b').setFormat('code'),
          $createTextNode('c').setFormat('code'),
        )
        $getRoot().append(p)
      },
      { discrete: true },
    )
    expect(editor.getEditorState().read(() => $exportComposerMarkdown())).toBe(
      'a`bc`',
    )
    expect(roundTrip('```\nopen')).toBe('```\nopen\n```')
  })
})

describe('looksLikeComposerMarkdown', () => {
  it('spots line breaks, backticks and list lines', () => {
    expect(looksLikeComposerMarkdown('plain words')).toBe(false)
    expect(looksLikeComposerMarkdown('two\nlines')).toBe(true)
    expect(looksLikeComposerMarkdown('run `x`')).toBe(true)
    expect(looksLikeComposerMarkdown('- item')).toBe(true)
    expect(looksLikeComposerMarkdown('1. item')).toBe(true)
  })
})
