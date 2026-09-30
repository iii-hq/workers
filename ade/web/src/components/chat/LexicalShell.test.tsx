// @vitest-environment jsdom

import { $createCodeNode, $isCodeNode } from '@lexical/code-core'
import {
  $createListItemNode,
  $createListNode,
  $isListItemNode,
  $isListNode,
} from '@lexical/list'
import {
  $createLineBreakNode,
  $createParagraphNode,
  $createTextNode,
  $getRoot,
  $getSelection,
  $isParagraphNode,
  $isRangeSelection,
  COMMAND_PRIORITY_NORMAL,
  type ElementNode,
  KEY_ENTER_COMMAND,
  type LexicalEditor,
} from 'lexical'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { FunctionEntry } from '@/lib/functions'
import { LexicalShell } from './LexicalShell'
import { $exportComposerMarkdown } from './lexical/composer-markdown'

vi.mock('@/lib/iii-client', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  // The / palette fetches skills when it opens; keep that fetch pending.
  getIiiClient: () => new Promise(() => {}),
}))

let container: HTMLDivElement
let root: Root

const MAC_UA =
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36'
const WINDOWS_UA =
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36'

/** `shortcutPlatform()` reads the user agent, and jsdom's own names neither
 *  platform; the suite runs as a Mac unless a test says otherwise. */
function setUserAgent(value: string) {
  Object.defineProperty(navigator, 'userAgent', { value, configurable: true })
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  setUserAgent(MAC_UA)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
  // Back to the prototype's getter.
  Reflect.deleteProperty(navigator, 'userAgent')
})

/** A paragraph reading `text`. */
function $paragraph(text: string) {
  const paragraph = $createParagraphNode()
  paragraph.append($createTextNode(text))
  $getRoot().append(paragraph)
}

/** A paragraph reading `hello` — the default draft. */
function $helloParagraph() {
  $paragraph('hello')
}

/** Mount a real Lexical composer with a draft, caret at its end, and a submit spy. */
async function renderComposer({
  disabled = false,
  content = $helloParagraph,
  place = () => $getRoot().selectEnd(),
  onHistoryNav,
  functionEntries,
}: {
  disabled?: boolean
  content?: () => void
  /** The `@` menu's catalog; none by default. */
  functionEntries?: FunctionEntry[]
  /** Where the caret goes once mounted; the end of the draft by default. */
  place?: () => void
  onHistoryNav?: (direction: 'up' | 'down') => string | null
} = {}) {
  const onSubmit = vi.fn()
  const onChange = vi.fn<(text: string) => void>()
  let editor!: LexicalEditor
  await act(async () => {
    root.render(
      <LexicalShell
        clearToken={0}
        disabled={disabled}
        onChange={onChange}
        onSubmit={onSubmit}
        onHistoryNav={onHistoryNav}
        functionEntries={functionEntries}
        initialContent={(instance) => {
          editor = instance
          content()
        }}
      />,
    )
  })
  await act(async () => {
    editor.update(place, { discrete: true })
    // Let the DOM selectionchange event settle inside React's act scope.
    await new Promise<void>((resolve) => setTimeout(resolve, 0))
  })
  const editable = container.querySelector<HTMLElement>(
    '[aria-label="message composer"]',
  )
  if (!editable) throw new Error('Composer not mounted')
  return { editor, editable, onSubmit, onChange }
}

/** Dispatch a cancelable keydown and flush the resulting editor updates. */
async function pressKey(
  editable: HTMLElement,
  key: 'Enter' | 'ArrowDown',
  modifiers: KeyboardEventInit = {},
) {
  const event = new KeyboardEvent('keydown', {
    key,
    code: key,
    keyCode: key === 'Enter' ? 13 : 40,
    bubbles: true,
    cancelable: true,
    ...modifiers,
  })
  await act(async () => {
    editable.dispatchEvent(event)
  })
  return event
}

async function pressEnter(editable: HTMLElement, modifiers: KeyboardEventInit) {
  return pressKey(editable, 'Enter', modifiers)
}

/** Let the typeahead's state, portal and effects catch up. */
async function settle() {
  for (let i = 0; i < 5; i++) {
    await act(async () => {
      await new Promise<void>((resolve) => setTimeout(resolve, 0))
    })
  }
}

/** What a submit would send: the editor as composer markdown. */
function markdown(editor: LexicalEditor): string {
  return editor.getEditorState().read(() => $exportComposerMarkdown())
}

// Enter sends from anywhere, Shift+Enter always breaks the line, and Mod+Enter
// (⌘↵ on a Mac — the suite's default, see the top of the file — Ctrl+Enter
// elsewhere) sends too.
describe('composer keyboard submission', () => {
  it('Enter submits once without inserting a newline', async () => {
    const { editor, editable, onSubmit } = await renderComposer()
    const event = await pressEnter(editable, {})

    expect(onSubmit).toHaveBeenCalledTimes(1)
    expect(event.defaultPrevented).toBe(true)
    expect(markdown(editor)).toBe('hello')
  })

  it('Shift+Enter inserts a newline and never submits', async () => {
    const { editor, editable, onSubmit } = await renderComposer()
    await pressEnter(editable, { shiftKey: true })

    expect(onSubmit).not.toHaveBeenCalled()
    expect(markdown(editor)).toBe('hello\n')
  })

  it.each([
    ['Alt+Enter', { altKey: true }],
    ['Ctrl+Enter on a Mac', { ctrlKey: true }],
    ['Cmd+Shift+Enter', { metaKey: true, shiftKey: true }],
  ] as const)(
    '%s inserts a newline and never submits',
    async (_, modifiers) => {
      const { editor, editable, onSubmit } = await renderComposer()
      await pressEnter(editable, modifiers)

      expect(onSubmit).not.toHaveBeenCalled()
      expect(markdown(editor)).toBe('hello\n')
    },
  )

  it('Enter submits with prose selected, without replacing the selection', async () => {
    const { editor, editable, onSubmit } = await renderComposer({
      place: () => {
        const text = $getRoot().getAllTextNodes()[0]
        text.select(1, 4)
      },
    })
    const event = await pressEnter(editable, {})

    expect(onSubmit).toHaveBeenCalledTimes(1)
    expect(event.defaultPrevented).toBe(true)
    expect(markdown(editor)).toBe('hello')
  })

  it('Cmd+Enter submits once without inserting a newline', async () => {
    const { editor, editable, onSubmit } = await renderComposer()
    const event = await pressEnter(editable, { metaKey: true })

    expect(onSubmit).toHaveBeenCalledTimes(1)
    expect(event.defaultPrevented).toBe(true)
    expect(markdown(editor)).toBe('hello')
  })

  it('Ctrl+Enter submits on Windows and Linux', async () => {
    setUserAgent(WINDOWS_UA)
    const { editor, editable, onSubmit } = await renderComposer()
    const event = await pressEnter(editable, { ctrlKey: true })

    expect(onSubmit).toHaveBeenCalledTimes(1)
    expect(event.defaultPrevented).toBe(true)
    expect(markdown(editor)).toBe('hello')
  })

  it.each([
    ['Enter', {}],
    ['Cmd+Enter', { metaKey: true }],
  ] as const)(
    'does not submit a disabled composer with %s',
    async (_, modifiers) => {
      const { editable, onSubmit } = await renderComposer({ disabled: true })
      await pressEnter(editable, modifiers)

      expect(onSubmit).not.toHaveBeenCalled()
    },
  )

  // Desktop Safari fires the Enter that confirms an IME candidate after
  // compositionend, with keyCode 229; the others flag it isComposing.
  it.each([
    ['isComposing', { isComposing: true }],
    ['keyCode 229', { keyCode: 229 }],
  ] as const)(
    'leaves the Enter that confirms an IME candidate (%s) to the IME',
    async (_, init) => {
      const { editor, editable, onSubmit } = await renderComposer()
      const event = await pressEnter(editable, init)

      expect(onSubmit).not.toHaveBeenCalled()
      expect(event.defaultPrevented).toBe(false)
      expect(markdown(editor)).toBe('hello')
    },
  )
})

// A menu that shows options consumes Enter to pick one; a trigger that
// matches nothing shows no menu and leaves Enter to send.
describe('composer keyboard submission with a typeahead', () => {
  const functionEntries: FunctionEntry[] = [
    { id: 'shell::exec', description: 'run a shell command' },
  ]

  beforeEach(() => {
    // The menu measures the caret and its frame; jsdom does neither.
    if (!('getBoundingClientRect' in Range.prototype)) {
      Object.defineProperty(Range.prototype, 'getBoundingClientRect', {
        configurable: true,
        value: () => ({
          top: 0,
          left: 0,
          right: 0,
          bottom: 0,
          width: 0,
          height: 0,
          x: 0,
          y: 0,
        }),
      })
    }
    if (!('ResizeObserver' in globalThis)) {
      vi.stubGlobal(
        'ResizeObserver',
        class {
          observe() {}
          unobserve() {}
          disconnect() {}
        },
      )
    }
  })

  it('sends only after Enter handlers at NORMAL, where a menu picks its option', async () => {
    const { editor, editable, onSubmit } = await renderComposer()
    const pick = vi.fn(() => true)
    const off = editor.registerCommand(
      KEY_ENTER_COMMAND,
      pick,
      COMMAND_PRIORITY_NORMAL,
    )
    await pressEnter(editable, {})
    off()

    expect(pick).toHaveBeenCalledTimes(1)
    expect(onSubmit).not.toHaveBeenCalled()
  })

  it('Enter picks the highlighted @ option instead of sending', async () => {
    const { editor, editable, onSubmit } = await renderComposer({
      functionEntries,
      content: () => $paragraph('@sh'),
    })
    await settle()
    await pressEnter(editable, {})
    await settle()

    expect(onSubmit).not.toHaveBeenCalled()
    expect(markdown(editor)).toContain('shell::exec')
  })

  it('Enter still picks when the caret moves from an open / menu to an @ query', async () => {
    const { editor, editable, onSubmit } = await renderComposer({
      functionEntries,
      content: () => $paragraph('@sh /co'),
    })
    await settle()
    // One update: the / menu closes as the @ menu opens.
    await act(async () => {
      editor.update(() => $getRoot().getAllTextNodes()[0].select(3, 3), {
        discrete: true,
      })
    })
    await settle()
    await pressEnter(editable, {})
    await settle()

    expect(onSubmit).not.toHaveBeenCalled()
    expect(markdown(editor)).toContain('shell::exec')
  })

  it('Enter sends an @ query that matches nothing', async () => {
    const { editor, editable, onSubmit } = await renderComposer({
      functionEntries,
      content: () => $paragraph('thanks @zzz'),
    })
    await settle()
    const event = await pressEnter(editable, {})

    expect(onSubmit).toHaveBeenCalledTimes(1)
    expect(event.defaultPrevented).toBe(true)
    expect(markdown(editor)).toBe('thanks @zzz')
  })
})

// A code block that ends the draft: Down on its last line steps out of it.
describe('arrow down out of a trailing code block', () => {
  function $code(...lines: string[]) {
    const code = $createCodeNode()
    lines.forEach((line, i) => {
      if (i > 0) code.append($createLineBreakNode())
      code.append($createTextNode(line))
    })
    $getRoot().append(code)
  }

  it('adds a paragraph after the block and moves the caret there', async () => {
    const { editor, editable } = await renderComposer({
      content: () => $code('let a', 'let b'),
    })
    const event = await pressKey(editable, 'ArrowDown')

    expect(event.defaultPrevented).toBe(true)
    editor.getEditorState().read(() => {
      const [code, after] = $getRoot().getChildren()
      expect($isCodeNode(code)).toBe(true)
      expect($isParagraphNode(after)).toBe(true)
      const selection = $getSelection()
      expect(
        $isRangeSelection(selection) &&
          selection.anchor.getNode().is(after as ElementNode),
      ).toBe(true)
    })
    expect(markdown(editor)).toBe('```\nlet a\nlet b\n```\n')
  })

  it('leaves Down alone on an earlier line of the block', async () => {
    const { editor, editable } = await renderComposer({
      content: () => $code('let a', 'let b'),
      place: () => {
        const first = $getRoot().getAllTextNodes()[0]
        first.select(first.getTextContentSize(), first.getTextContentSize())
      },
    })
    const event = await pressKey(editable, 'ArrowDown')

    expect(event.defaultPrevented).toBe(false)
    expect(markdown(editor)).toBe('```\nlet a\nlet b\n```')
  })

  it('leaves Down alone when something follows the block', async () => {
    const { editor, editable } = await renderComposer({
      content: () => {
        $code('let a')
        const paragraph = $createParagraphNode()
        paragraph.append($createTextNode('after'))
        $getRoot().append(paragraph)
      },
      place: () => $getRoot().getFirstChild()?.selectEnd(),
    })
    const event = await pressKey(editable, 'ArrowDown')

    expect(event.defaultPrevented).toBe(false)
    expect(markdown(editor)).toBe('```\nlet a\n```\nafter')
  })

  it('lets a pristine draft browse the queue first', async () => {
    const onHistoryNav = vi.fn(() => 'queued message')
    const { editor, editable } = await renderComposer({
      content: () => $code('let a'),
      onHistoryNav,
    })
    await pressKey(editable, 'ArrowDown')

    expect(onHistoryNav).toHaveBeenCalledWith('down')
    expect(markdown(editor)).toBe('queued message')
  })
})

// The WYSIWYG blocks: what the editor holds is what the message will render.
// Shift+Enter inside them is structure; Enter sends from them too.
describe('composer markdown blocks', () => {
  function $bulletList(...items: string[]) {
    const list = $createListNode('bullet')
    for (const text of items) {
      const item = $createListItemNode()
      if (text.length > 0) item.append($createTextNode(text))
      list.append(item)
    }
    $getRoot().append(list)
  }

  it('reports the draft as composer markdown on change', async () => {
    const { onChange } = await renderComposer({
      content: () => {
        $bulletList('one', 'two')
        const code = $createCodeNode('ts')
        code.append(
          $createTextNode('const a = 1'),
          $createLineBreakNode(),
          $createTextNode('const b = 2'),
        )
        $getRoot().append(code)
      },
    })

    expect(onChange).toHaveBeenLastCalledWith(
      '- one\n- two\n\n```ts\nconst a = 1\nconst b = 2\n```',
    )
  })

  it.each([
    ['Enter', {}],
    ['Cmd+Enter', { metaKey: true }],
  ] as const)('%s sends from inside a list', async (_, modifiers) => {
    const { editor, editable, onSubmit } = await renderComposer({
      content: () => $bulletList('one'),
    })
    const event = await pressEnter(editable, modifiers)

    expect(onSubmit).toHaveBeenCalledTimes(1)
    expect(event.defaultPrevented).toBe(true)
    expect(markdown(editor)).toBe('- one')
  })

  it('Shift+Enter inside a list item starts the next item instead of sending', async () => {
    const { editor, editable, onSubmit } = await renderComposer({
      content: () => $bulletList('one'),
    })
    await pressEnter(editable, { shiftKey: true })

    expect(onSubmit).not.toHaveBeenCalled()
    editor.getEditorState().read(() => {
      const list = $getRoot().getFirstChild()
      expect($isListNode(list) && list.getChildrenSize()).toBe(2)
    })
  })

  it('Shift+Enter on an empty list item leaves the list', async () => {
    const { editor, editable, onSubmit } = await renderComposer({
      content: () => $bulletList('one', ''),
    })
    await pressEnter(editable, { shiftKey: true })

    expect(onSubmit).not.toHaveBeenCalled()
    editor.getEditorState().read(() => {
      const [list, after] = $getRoot().getChildren()
      expect($isListNode(list) && list.getChildrenSize()).toBe(1)
      expect($isParagraphNode(after)).toBe(true)
    })
    expect(markdown(editor)).toBe('- one\n')
  })

  it('Shift+Enter inside a code block adds a line instead of sending', async () => {
    const { editor, editable, onSubmit } = await renderComposer({
      content: () => {
        const code = $createCodeNode()
        code.append($createTextNode('let a'))
        $getRoot().append(code)
      },
    })
    await pressEnter(editable, { shiftKey: true })

    expect(onSubmit).not.toHaveBeenCalled()
    expect(markdown(editor)).toBe('```\nlet a\n\n```')
  })

  it('Shift+Enter after prose starts a line where a fence still opens a code block', async () => {
    const { editor, editable, onSubmit } = await renderComposer({
      content: () => $paragraph('look:'),
    })
    await pressEnter(editable, { shiftKey: true })
    await act(async () => {
      editor.update(
        () => {
          const selection = $getSelection()
          if ($isRangeSelection(selection)) selection.insertText('```ts')
        },
        { discrete: true },
      )
    })
    await pressEnter(editable, { shiftKey: true })

    expect(onSubmit).not.toHaveBeenCalled()
    editor.getEditorState().read(() => {
      const [first, second] = $getRoot().getChildren()
      expect($isParagraphNode(first)).toBe(true)
      expect($isCodeNode(second) && second.getLanguage()).toBe('ts')
    })
    expect(markdown(editor)).toBe('look:\n```ts\n```')
  })

  it('Enter on a ``` line sends it as typed', async () => {
    const { editor, editable, onSubmit } = await renderComposer({
      content: () => {
        const paragraph = $createParagraphNode()
        paragraph.append($createTextNode('```py'))
        $getRoot().append(paragraph)
      },
    })
    await pressEnter(editable, {})

    expect(onSubmit).toHaveBeenCalledTimes(1)
    editor.getEditorState().read(() => {
      expect($isParagraphNode($getRoot().getFirstChild())).toBe(true)
    })
  })

  it('``` and Shift+Enter open a code block with the language instead of sending', async () => {
    const { editor, editable, onSubmit } = await renderComposer({
      content: () => {
        const paragraph = $createParagraphNode()
        paragraph.append($createTextNode('```py'))
        $getRoot().append(paragraph)
      },
    })
    const event = await pressEnter(editable, { shiftKey: true })

    expect(onSubmit).not.toHaveBeenCalled()
    expect(event.defaultPrevented).toBe(true)
    editor.getEditorState().read(() => {
      const code = $getRoot().getFirstChild()
      expect($isCodeNode(code) && code.getLanguage()).toBe('py')
    })
    expect(markdown(editor)).toBe('```py\n```')
  })

  it.each([
    ['Enter', {}],
    ['Cmd+Enter', { metaKey: true }],
  ] as const)('%s sends from inside a code block', async (_, modifiers) => {
    const { editor, editable, onSubmit } = await renderComposer({
      content: () => {
        const code = $createCodeNode()
        code.append($createTextNode('let a'))
        $getRoot().append(code)
      },
    })
    const event = await pressEnter(editable, modifiers)

    expect(onSubmit).toHaveBeenCalledTimes(1)
    expect(event.defaultPrevented).toBe(true)
    expect(markdown(editor)).toBe('```\nlet a\n```')
  })

  it('a new code line keeps the indentation of the line above', async () => {
    const { editor, editable, onSubmit } = await renderComposer({
      content: () => {
        const code = $createCodeNode()
        code.append(
          $createTextNode('fn main() {'),
          $createLineBreakNode(),
          $createTextNode('    let a'),
        )
        $getRoot().append(code)
      },
    })
    await pressEnter(editable, { shiftKey: true })

    expect(onSubmit).not.toHaveBeenCalled()
    expect(markdown(editor)).toBe('```\nfn main() {\n    let a\n    \n```')
  })

  it('Shift+Enter on a second blank line leaves the code block', async () => {
    const { editor, editable, onSubmit } = await renderComposer({
      content: () => {
        const code = $createCodeNode()
        code.append(
          $createTextNode('done'),
          $createLineBreakNode(),
          $createLineBreakNode(),
        )
        $getRoot().append(code)
      },
    })
    await pressEnter(editable, { shiftKey: true })

    expect(onSubmit).not.toHaveBeenCalled()
    editor.getEditorState().read(() => {
      const [code, after] = $getRoot().getChildren()
      expect($isCodeNode(code) && code.getTextContent()).toBe('done')
      expect($isParagraphNode(after)).toBe(true)
    })
    expect(markdown(editor)).toBe('```\ndone\n```\n')
  })

  it('leaves the code block from two indented blank lines, dropping them', async () => {
    const { editor, editable, onSubmit } = await renderComposer({
      content: () => {
        const code = $createCodeNode()
        code.append(
          $createTextNode('  return a'),
          $createLineBreakNode(),
          $createTextNode('  '),
          $createLineBreakNode(),
          $createTextNode('  '),
        )
        $getRoot().append(code)
      },
    })
    await pressEnter(editable, { shiftKey: true })

    expect(onSubmit).not.toHaveBeenCalled()
    expect(markdown(editor)).toBe('```\n  return a\n```\n')
  })

  it('a nested list item exports under its parent marker', async () => {
    const { editor } = await renderComposer({
      content: () => {
        const list = $createListNode('number')
        const first = $createListItemNode()
        first.append($createTextNode('one'))
        const nested = $createListNode('bullet')
        const inner = $createListItemNode()
        inner.append($createTextNode('detail'))
        nested.append(inner)
        const wrapper = $createListItemNode()
        wrapper.append(nested)
        list.append(first, wrapper)
        $getRoot().append(list)
      },
    })
    editor.getEditorState().read(() => {
      const list = $getRoot().getFirstChild()
      expect(
        $isListNode(list) &&
          list.getChildren().every((item) => $isListItemNode(item)),
      ).toBe(true)
    })
    expect(markdown(editor)).toBe('1. one\n   - detail')
  })
})
