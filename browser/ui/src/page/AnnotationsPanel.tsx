/**
 * The Annotations dock pane: one row per pin with its note, and the three
 * things a set of pins is for — sending it to the chat, saving it, dropping
 * it. The pins themselves live on the viewport; this pane shares their list.
 */

import * as ConsoleUi from '@iii-dev/console-ui'
import { Button } from '@iii-dev/console-ui'
import type { Annotation } from './annotations'

interface AnnotationsPanelProps {
  annotations: readonly Annotation[]
  selectedId: string | null
  annotating: boolean
  sending: boolean
  canSend: boolean
  onSelect: (id: string | null) => void
  onNote: (id: string, note: string) => void
  onRemove: (id: string) => void
  onSend: () => void
  onDownload: () => void
  onClear: () => void
  onStart: () => void
}

export function AnnotationsPanel({
  annotations,
  selectedId,
  annotating,
  sending,
  canSend,
  onSelect,
  onNote,
  onRemove,
  onSend,
  onDownload,
  onClear,
  onStart,
}: AnnotationsPanelProps) {
  const List = ConsoleUi.AnnotationList
  return (
    <div className="br-ui-annotations">
      {!annotating && annotations.length === 0 ? (
        <div className="br-ui-annotations-empty">
          <p>
            Freeze the live view and drop numbered pins on it. Each pin takes a
            note; the set goes to the chat as one picture plus the notes.
          </p>
          <Button variant="pill" size="sm" onClick={onStart}>
            Start annotating
          </Button>
        </div>
      ) : List ? (
        <List
          annotations={annotations}
          selectedId={selectedId}
          onSelect={onSelect}
          onNote={onNote}
          onRemove={onRemove}
          emptyText="Click the frozen view to add a pin."
          className="br-ui-annotations-list"
        />
      ) : null}
      {annotations.length > 0 ? (
        <div className="br-ui-annotations-actions">
          <Button
            variant="primary"
            size="sm"
            onClick={onSend}
            disabled={!canSend || sending}
            title={
              canSend
                ? 'attach the annotated picture and the notes to the chat'
                : 'open a conversation first'
            }
          >
            {sending ? 'Sending…' : 'Send to chat'}
          </Button>
          <Button variant="ghost" size="sm" onClick={onDownload}>
            Download
          </Button>
          <Button variant="ghost" size="sm" onClick={onClear}>
            Clear
          </Button>
        </div>
      ) : null}
    </div>
  )
}
