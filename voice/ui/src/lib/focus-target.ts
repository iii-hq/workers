/**
 * Dictated text goes where you were typing, the way Wispr Flow types into
 * the focused app: the field that had focus when dictation began (the IDE's
 * editor or terminal, a search box, a form field), else the chat composer.
 *
 * A plain field gets `insertText`, which types like a person (undo works,
 * frameworks see an input event). Monaco types through Chrome's EditContext
 * and xterm through a proxy textarea, and neither takes `insertText`; both
 * handle a paste, so they get one, the way dictation apps insert text.
 */

const TEXT_INPUT_TYPES = new Set(['text', 'search', 'url', 'email', 'tel', ''])

/** The element with focus, through open shadow roots. */
export function deepActiveElement(root: Document = document): Element | null {
  let element = root.activeElement
  while (element?.shadowRoot?.activeElement) element = element.shadowRoot.activeElement
  return element
}

/** `element`, when text can be typed into it. */
export function editableTarget(element: Element | null): HTMLElement | null {
  if (!(element instanceof HTMLElement)) return null
  if (element instanceof HTMLTextAreaElement) return element.disabled || element.readOnly ? null : element
  if (element instanceof HTMLInputElement) {
    if (!TEXT_INPUT_TYPES.has(element.type) || element.disabled || element.readOnly) return null
    return element
  }
  if (hasEditContext(element)) return element
  return element.isContentEditable ? element : null
}

/** Monaco's editor surface in Chrome: a div typing through an EditContext. */
function hasEditContext(element: HTMLElement): boolean {
  return (element as HTMLElement & { editContext?: unknown }).editContext != null
}

/** Hand `text` over as a paste; true when the editor took it. */
function pasteInto(element: HTMLElement, text: string): boolean {
  const data = new DataTransfer()
  data.setData('text/plain', text)
  const event = new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true })
  element.dispatchEvent(event)
  return event.defaultPrevented
}

/** The chat composer is the console's own editor: it takes text through `host.chat.compose`. */
export function isChatComposer(element: HTMLElement): boolean {
  return element.closest('[data-lexical-editor]') !== null
}

/** The field's own text before the caret, for spacing; null where the
    textarea is only a proxy (Monaco, xterm) or there is no plain value. */
function textBefore(element: HTMLElement): string | null {
  if (!(element instanceof HTMLTextAreaElement || element instanceof HTMLInputElement)) return null
  if (element.closest('.monaco-editor, .xterm')) return null
  const caret = element.selectionStart ?? element.value.length
  return element.value.slice(0, caret)
}

/** Type `text` into `element` at its caret. False when it could not. */
export function typeInto(element: HTMLElement, text: string): boolean {
  if (!element.isConnected) return false
  const before = textBefore(element)
  // Run on from what is there as speech would: a space after a word, none at the start.
  const spaced = before !== null && before !== '' && !/\s$/.test(before) ? ` ${text}` : text
  element.focus({ preventScroll: true })
  if (hasEditContext(element) || element.closest('.xterm')) return pasteInto(element, spaced)
  // biome-ignore lint/suspicious/noExplicitAny: execCommand is deprecated in the types but is the one API that types like a person
  if ((document as any).execCommand?.('insertText', false, spaced)) return true
  if (pasteInto(element, spaced)) return true
  if (element instanceof HTMLTextAreaElement || element instanceof HTMLInputElement) {
    const start = element.selectionStart ?? element.value.length
    const end = element.selectionEnd ?? start
    element.setRangeText(spaced, start, end, 'end')
    element.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText', data: spaced }))
    return true
  }
  return false
}
