/**
 * Saved annotation sets: list what the `state` worker holds, preview one
 * over its stored picture with every mark painted on, and send it to the
 * chat, download it, or delete it. Reached from the ⋮ menu, ⌘K, or the
 * palette's saved-set rows.
 */

import {
  Button,
  ConfirmDialog,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  EmptyState,
  type Host,
  List,
  ListItem,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useState } from 'react'
import { downloadFile, errorMessage, formatAgo } from '../lib/browser'
import {
  type AnnotationSet,
  annotationFileName,
  annotationsMarkdown,
  renderAnnotatedImage,
} from './annotations'
import {
  deleteAnnotationSet,
  listAnnotationSets,
  readAnnotationSet,
  type SavedSetSummary,
} from './annotations-store'

interface SavedSetsDialogProps {
  host: Host
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Select this set when opening (a palette row names one). */
  initialKey?: string | null
}

async function renderAnnotatedFile(set: AnnotationSet): Promise<File> {
  const blob = await renderAnnotatedImage(set)
  return new File([blob], annotationFileName(set, 'png'), {
    type: 'image/png',
  })
}

export function SavedSetsDialog({
  host,
  open,
  onOpenChange,
  initialKey = null,
}: SavedSetsDialogProps) {
  const [sets, setSets] = useState<SavedSetSummary[]>([])
  const [selectedKey, setSelectedKey] = useState<string | null>(null)
  const [selectedSet, setSelectedSet] = useState<AnnotationSet | null>(null)
  const [preview, setPreview] = useState<string | null>(null)
  const [status, setStatus] = useState<string | null>(null)
  const [confirmingDelete, setConfirmingDelete] = useState(false)

  const refresh = useCallback(() => {
    void listAnnotationSets(host.iii)
      .then(setSets)
      .catch((e: unknown) => setStatus(errorMessage(e)))
  }, [host])

  useEffect(() => {
    if (!open) return
    setStatus(null)
    setSelectedKey(initialKey)
    refresh()
  }, [open, initialKey, refresh])

  useEffect(() => {
    // Clear at once so a slow read never shows the previous set's picture
    // under the new selection.
    setSelectedSet(null)
    if (!open || !selectedKey) return
    let cancelled = false
    void readAnnotationSet(host.iii, selectedKey)
      .then((set) => {
        if (cancelled) return
        setSelectedSet(set)
        if (!set) setStatus('that set could not be read')
      })
      .catch((e: unknown) => {
        if (!cancelled) setStatus(errorMessage(e))
      })
    return () => {
      cancelled = true
    }
  }, [open, selectedKey, host])

  // The preview is the stored picture with the marks painted on, the same
  // export the chat receives.
  useEffect(() => {
    if (!selectedSet) {
      setPreview(null)
      return
    }
    let url: string | null = null
    let cancelled = false
    void renderAnnotatedImage(selectedSet).then((blob) => {
      if (cancelled) return
      url = URL.createObjectURL(blob)
      setPreview(url)
    })
    return () => {
      cancelled = true
      if (url) URL.revokeObjectURL(url)
    }
  }, [selectedSet])

  const sendToChat = useCallback(() => {
    const set = selectedSet
    if (!set || !host.chat?.compose) return
    void renderAnnotatedFile(set)
      .then((file) => {
        host.chat?.compose?.({ text: annotationsMarkdown(set), files: [file] })
        setStatus('sent to the chat')
      })
      .catch((e: unknown) => setStatus(errorMessage(e)))
  }, [selectedSet, host])

  const download = useCallback(() => {
    const set = selectedSet
    if (!set) return
    void renderAnnotatedFile(set)
      .then(downloadFile)
      .catch((e: unknown) => setStatus(errorMessage(e)))
  }, [selectedSet])

  const remove = useCallback(() => {
    const key = selectedKey
    if (!key) return
    void deleteAnnotationSet(host.iii, key)
      .then(() => {
        setSelectedKey(null)
        refresh()
      })
      .catch((e: unknown) => setStatus(errorMessage(e)))
  }, [selectedKey, host, refresh])
  const askRemove = useCallback(() => setConfirmingDelete(true), [])

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="br-ui-sets">
        <DialogTitle>Saved annotations</DialogTitle>
        <DialogDescription>
          Sets saved from a session; anyone on this engine sees the same list.
        </DialogDescription>
        {sets.length === 0 ? (
          <EmptyState
            title={status ?? 'Nothing saved yet'}
            description="Save a set while annotating and it appears here for everyone on this engine."
          />
        ) : (
          <div className="br-ui-sets-body">
            <List className="br-ui-sets-list" aria-label="saved sets">
              {sets.map((s) => (
                <ListItem
                  key={s.key}
                  selected={s.key === selectedKey}
                  aria-pressed={s.key === selectedKey}
                  onClick={() => setSelectedKey(s.key)}
                  title={s.subject}
                  label={<span className="br-ui-mono">{s.subject}</span>}
                  description={
                    <span className="br-ui-mono br-ui-num">
                      {s.count} {s.count === 1 ? 'mark' : 'marks'}
                      <span aria-hidden> · </span>
                      {formatAgo(s.capturedAt)}
                    </span>
                  }
                />
              ))}
            </List>
            <div className="br-ui-sets-preview">
              {preview && selectedSet ? (
                <>
                  <img
                    src={preview}
                    alt={`saved annotations on ${selectedSet.subject}`}
                    className="br-ui-sets-image"
                  />
                  <div className="br-ui-sets-actions">
                    <Button
                      variant="primary"
                      size="sm"
                      onClick={sendToChat}
                      disabled={typeof host.chat?.compose !== 'function'}
                    >
                      Send to chat
                    </Button>
                    <Button variant="ghost" size="sm" onClick={download}>
                      Download
                    </Button>
                    <Button variant="ghost" size="sm" onClick={askRemove}>
                      Delete
                    </Button>
                  </div>
                  {status ? (
                    <p className="br-ui-sets-status">{status}</p>
                  ) : null}
                </>
              ) : (
                <EmptyState
                  title={status ?? 'Pick a set'}
                  description="Select a saved set to preview it with its marks painted on."
                />
              )}
            </div>
          </div>
        )}
        <ConfirmDialog
          open={confirmingDelete}
          onOpenChange={setConfirmingDelete}
          title="Delete this saved set?"
          description="It disappears for everyone on this engine."
          confirmLabel="Delete"
          onConfirm={remove}
        />
      </DialogContent>
    </Dialog>
  )
}
