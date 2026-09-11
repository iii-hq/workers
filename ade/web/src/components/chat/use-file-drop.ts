/**
 * Dropping a file anywhere on a chat, and pasting one into it.
 *
 * Two things make this harder than an `onDrop` prop, and both were live
 * failures before this hook existed.
 *
 * The editor eats the event. A drop lands on the innermost element under the
 * cursor, which over the composer is Lexical's `contenteditable`, and its
 * plain-text plugin handles `dragover`/`drop`/`paste` on that element. React
 * props on an ancestor run afterwards, if at all. So the listeners here are
 * NATIVE and CAPTURE-phase: capture runs root to target, so a file drop is
 * claimed and stopped before the editor ever sees it. Text drags are left
 * alone — dragging a word out of the transcript into the composer still works.
 *
 * The target is the whole conversation, not the composer box. People drop a
 * screenshot onto the message they are reading, which is the transcript, a
 * hundred pixels above the box. Scoping the zone to the composer meant most
 * real drops hit nothing at all, which reads as "drag and drop does not work".
 * The zone is the enclosing chat pane, found from the composer's own node, so
 * two chats side by side keep separate zones and a worker page's own drop zone
 * is untouched.
 */

import { type RefObject, useEffect, useRef, useState } from 'react'

/** Marks the chat pane. `ChatView` puts this on its root `<section>`. */
const CHAT_PANE_SELECTOR = '[data-chat-session-id]'

/**
 * `true` when a drag carries files rather than selected text. During a drag
 * the browser withholds the data itself, so `types` is the only thing to read.
 */
function carriesFiles(e: DragEvent): boolean {
  // `types` is already a readonly array of strings, and this runs on every
  // `dragover` for the whole gesture, so there is nothing to copy it into.
  return (e.dataTransfer?.types ?? []).includes('Files')
}

interface FileDropOptions {
  /** Any node inside the pane that should accept drops. */
  anchorRef: RefObject<HTMLElement | null>
  /** No attaching while the composer is blocked or a turn is locked. */
  disabled: boolean
  onFiles: (files: File[]) => void
}

/** `true` while a file drag is over the pane, for the drop affordance. */
export function useFileDrop({
  anchorRef,
  disabled,
  onFiles,
}: FileDropOptions): boolean {
  const [dragging, setDragging] = useState(false)

  // Read through refs so a re-render never rebinds the listeners: rebinding
  // mid-drag drops the depth count and the highlight sticks on.
  const disabledRef = useRef(disabled)
  disabledRef.current = disabled
  const onFilesRef = useRef(onFiles)
  onFilesRef.current = onFiles

  useEffect(() => {
    const anchor = anchorRef.current
    if (!anchor) return
    const host = anchor.closest(CHAT_PANE_SELECTOR) ?? anchor

    // Depth counter, not a boolean: dragging across a child fires `dragleave`
    // on the element being left before `dragenter` on the one being entered,
    // so a boolean flickers the highlight off over every control in the pane.
    let depth = 0

    // A file drag is consumed whether or not the composer will accept it.
    // Returning early while disabled leaves the browser's own handling in
    // place, and the browser's handling of a dropped PDF is to navigate to it:
    // the console is replaced by a document viewer and the conversation is
    // gone. Disabled means "do not attach", not "let the page eat itself".
    const handleDragEnter = (e: DragEvent) => {
      if (!carriesFiles(e)) return
      e.preventDefault()
      if (disabledRef.current) return
      depth += 1
      setDragging(true)
    }

    const handleDragOver = (e: DragEvent) => {
      if (!carriesFiles(e)) return
      // Without a prevented `dragover` the browser never fires `drop` at all.
      e.preventDefault()
      e.stopPropagation()
      if (disabledRef.current) return
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy'
    }

    const handleDragLeave = (e: DragEvent) => {
      if (!carriesFiles(e)) return
      depth = Math.max(0, depth - 1)
      if (depth === 0) setDragging(false)
    }

    const handleDrop = (e: DragEvent) => {
      if (!carriesFiles(e)) return
      e.preventDefault()
      e.stopPropagation()
      depth = 0
      setDragging(false)
      if (disabledRef.current) return
      const files = Array.from(e.dataTransfer?.files ?? [])
      if (files.length > 0) onFilesRef.current(files)
    }

    // A screenshot pasted from the clipboard carries no text, and a file copied
    // from a file manager carries only its name as text — which would land in
    // the message beside its own chip. Consume the paste in both cases.
    const handlePaste = (e: ClipboardEvent) => {
      const files = Array.from(e.clipboardData?.files ?? [])
      if (files.length === 0) return
      e.preventDefault()
      e.stopPropagation()
      if (disabledRef.current) return
      onFilesRef.current(files)
    }

    // `host` is an `Element`, whose overloads type every listener as taking a
    // bare `Event`; the drag and clipboard events are narrowed above.
    const bindings: Array<[string, EventListener]> = [
      ['dragenter', handleDragEnter as EventListener],
      ['dragover', handleDragOver as EventListener],
      ['dragleave', handleDragLeave as EventListener],
      ['drop', handleDrop as EventListener],
      ['paste', handlePaste as EventListener],
    ]
    for (const [type, listener] of bindings) {
      host.addEventListener(type, listener, true)
    }
    return () => {
      for (const [type, listener] of bindings) {
        host.removeEventListener(type, listener, true)
      }
    }
  }, [anchorRef])

  return dragging && !disabled
}
