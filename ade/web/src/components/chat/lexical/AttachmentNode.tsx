import { Tooltip as TooltipPrimitive } from '@base-ui/react/tooltip'
import { $isCodeNode } from '@lexical/code-core'
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext'
import { useLexicalNodeSelection } from '@lexical/react/useLexicalNodeSelection'
import {
  $createParagraphNode,
  $createTextNode,
  $getNodeByKey,
  $getRoot,
  $getSelection,
  $isParagraphNode,
  $isRangeSelection,
  CLICK_COMMAND,
  COMMAND_PRIORITY_LOW,
  DecoratorNode,
  type DOMConversion,
  type DOMConversionMap,
  type DOMConversionOutput,
  type DOMExportOutput,
  type EditorConfig,
  type EditorState,
  KEY_BACKSPACE_COMMAND,
  KEY_DELETE_COMMAND,
  type LexicalNode,
  mergeRegister,
  type NodeKey,
  type ParagraphNode,
  type SerializedLexicalNode,
  type Spread,
} from 'lexical'
import { type JSX, type ReactNode, useContext, useEffect, useRef } from 'react'
import { NEUTRAL_SURFACE } from '@/lib/neutral-surface'
import { cn } from '@/lib/utils'
import type { Attachment } from '@/types/chat'
import { AttachmentChip } from '../AttachmentChip'
import { ComposerAttachmentsContext } from './attachment-context'

export type SerializedAttachmentNode = Spread<
  { attachmentId: string; name: string; mime: string },
  SerializedLexicalNode
>

/**
 * A file attached to the draft, as a chip in the text where it was added:
 * "compare [shot.png] with [notes.md]". The node holds the chip's id and a
 * name for its serialized form; the bytes, thumbnail and server id come
 * from the composer through `ComposerAttachmentsContext`, which also owns
 * the list itself. It contributes nothing to the message text — the
 * attachments travel beside it — so `getTextContent()` is empty.
 */
export class AttachmentNode extends DecoratorNode<JSX.Element> {
  __id: string
  __name: string
  __mime: string

  static getType(): string {
    return 'attachment'
  }

  static clone(node: AttachmentNode): AttachmentNode {
    return new AttachmentNode(node.__id, node.__name, node.__mime, node.__key)
  }

  /* A chip pasted back in (from this editor or another) comes with its id;
     the composer restores one it remembers and removes a stray. */
  static importDOM(): DOMConversionMap | null {
    return {
      span: (el: HTMLElement): DOMConversion<HTMLElement> | null => {
        const id = el.getAttribute('data-lexical-attachment')
        if (!id) return null
        return {
          conversion: (element: HTMLElement): DOMConversionOutput => ({
            node: new AttachmentNode(
              id,
              element.textContent ?? '',
              element.getAttribute('data-mime') ?? '',
            ),
          }),
          priority: 1,
        }
      },
    }
  }

  static importJSON(serialized: SerializedAttachmentNode): AttachmentNode {
    return new AttachmentNode(
      serialized.attachmentId,
      serialized.name,
      serialized.mime,
    )
  }

  constructor(id: string, name: string, mime: string, key?: NodeKey) {
    super(key)
    this.__id = id
    this.__name = name
    this.__mime = mime
  }

  exportJSON(): SerializedAttachmentNode {
    return {
      type: AttachmentNode.getType(),
      version: 1,
      attachmentId: this.__id,
      name: this.__name,
      mime: this.__mime,
    }
  }

  exportDOM(): DOMExportOutput {
    const element = document.createElement('span')
    element.setAttribute('data-lexical-attachment', this.__id)
    element.setAttribute('data-mime', this.__mime)
    element.textContent = this.__name
    return { element }
  }

  createDOM(_config: EditorConfig): HTMLElement {
    const span = document.createElement('span')
    span.style.display = 'inline-block'
    span.style.verticalAlign = 'middle'
    return span
  }

  updateDOM(): false {
    return false
  }

  isInline(): true {
    return true
  }

  isKeyboardSelectable(): true {
    return true
  }

  getTextContent(): string {
    return ''
  }

  getId(): string {
    return this.__id
  }

  decorate(): JSX.Element {
    return <EditableAttachmentChip id={this.__id} nodeKey={this.__key} />
  }
}

export function $createAttachmentNode(attachment: Attachment): AttachmentNode {
  return new AttachmentNode(attachment.id, attachment.name, attachment.type)
}

export function $isAttachmentNode(
  node: LexicalNode | null | undefined,
): node is AttachmentNode {
  return node instanceof AttachmentNode
}

export type AttachmentPlacement = 'caret' | 'start' | 'end'

/**
 * Put chips for `attachments` into the draft, each followed by a space so
 * typing carries on after it. `caret` lands them where the person is
 * writing (a code block excepted: a chip there would read as code), `start`
 * at the head of the draft (a restored draft, a browsed queue message),
 * `end` after everything.
 */
export function $insertAttachmentNodes(
  attachments: readonly Attachment[],
  placement: AttachmentPlacement = 'caret',
): void {
  if (attachments.length === 0) return
  const nodes = attachments.flatMap((attachment) => [
    $createAttachmentNode(attachment),
    $createTextNode(' '),
  ])
  const selection = $getSelection()
  if (
    placement === 'caret' &&
    $isRangeSelection(selection) &&
    !$isCodeNode(selection.anchor.getNode().getTopLevelElement())
  ) {
    selection.insertNodes(nodes)
    return
  }
  const root = $getRoot()
  const atStart = placement === 'start'
  const edge = atStart ? root.getFirstChild() : root.getLastChild()
  let paragraph: ParagraphNode
  if ($isParagraphNode(edge)) {
    paragraph = edge
  } else {
    paragraph = $createParagraphNode()
    if (atStart) root.splice(0, 0, [paragraph])
    else root.append(paragraph)
  }
  if (atStart) paragraph.splice(0, 0, nodes)
  else paragraph.append(...nodes)
}

/** The id of the attachment node under `key` in `state`, if it is one. */
export function attachmentIdOf(
  state: EditorState,
  key: NodeKey,
): string | null {
  return state.read(() => {
    const node = $getNodeByKey(key)
    return $isAttachmentNode(node) ? node.getId() : null
  })
}

/**
 * The thumbnail on hover: a 150px card above the chip, for a picture whose
 * bytes are here. Other files, and pictures still being fetched, show
 * nothing beyond the chip.
 */
function AttachmentPreview({
  attachment,
  children,
}: {
  attachment: Attachment
  children: ReactNode
}) {
  const src = attachment.type.startsWith('image/')
    ? attachment.dataUrl
    : undefined
  if (!src) return <>{children}</>
  return (
    <TooltipPrimitive.Root>
      <TooltipPrimitive.Trigger
        delay={120}
        closeDelay={0}
        render={<span className="inline-flex" />}
      >
        {children}
      </TooltipPrimitive.Trigger>
      <TooltipPrimitive.Portal>
        <TooltipPrimitive.Positioner
          side="top"
          sideOffset={8}
          className="isolate z-50"
        >
          <TooltipPrimitive.Popup
            data-attachment-preview={attachment.id}
            className={cn(
              NEUTRAL_SURFACE,
              'origin-(--transform-origin) rounded-xl bg-popover p-1.5 shadow-md ring-1 ring-foreground/10 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95',
            )}
          >
            <img
              src={src}
              alt={attachment.name}
              className="block size-[150px] rounded-lg object-cover"
            />
          </TooltipPrimitive.Popup>
        </TooltipPrimitive.Positioner>
      </TooltipPrimitive.Portal>
    </TooltipPrimitive.Root>
  )
}

/**
 * The chip in the editor: selectable like the mention pills (click, then
 * Backspace or Delete removes it; the arrows step off it), with the
 * attachment's own remove button. Clicks on the chip's buttons are theirs —
 * the thumbnail opens the viewer, the X removes — and do not select.
 */
function EditableAttachmentChip({
  id,
  nodeKey,
}: {
  id: string
  nodeKey: NodeKey
}) {
  const [editor] = useLexicalComposerContext()
  const [isSelected, setSelected, clearSelection] =
    useLexicalNodeSelection(nodeKey)
  const ref = useRef<HTMLSpanElement | null>(null)
  const { byId } = useContext(ComposerAttachmentsContext)
  const attachment = byId.get(id)

  useEffect(() => {
    const removeIfSelected = (event: KeyboardEvent): boolean => {
      if (!isSelected) return false
      event.preventDefault()
      editor.update(() => {
        $getNodeByKey(nodeKey)?.remove()
      })
      return true
    }
    return mergeRegister(
      editor.registerCommand(
        CLICK_COMMAND,
        (event: MouseEvent) => {
          const target = event.target as Node | null
          if (!ref.current || !target || !ref.current.contains(target)) {
            return false
          }
          if ((target as Element).closest?.('button')) return false
          event.preventDefault()
          clearSelection()
          setSelected(true)
          return true
        },
        COMMAND_PRIORITY_LOW,
      ),
      editor.registerCommand(
        KEY_BACKSPACE_COMMAND,
        removeIfSelected,
        COMMAND_PRIORITY_LOW,
      ),
      editor.registerCommand(
        KEY_DELETE_COMMAND,
        removeIfSelected,
        COMMAND_PRIORITY_LOW,
      ),
    )
  }, [editor, nodeKey, isSelected, setSelected, clearSelection])

  if (!attachment) return null

  const remove = () => {
    editor.update(() => {
      $getNodeByKey(nodeKey)?.remove()
    })
  }

  return (
    <span
      ref={ref}
      contentEditable={false}
      data-attachment-node={id}
      className="inline-flex select-none align-middle"
    >
      <AttachmentPreview attachment={attachment}>
        <AttachmentChip
          attachment={attachment}
          appearance="composer"
          onRemove={remove}
          className={cn(isSelected && 'ring-rule-focus')}
        />
      </AttachmentPreview>
    </span>
  )
}
