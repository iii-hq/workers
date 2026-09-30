import { useEffect, useRef, useState } from 'react'
import { useConfirm } from '@/components/ui/ConfirmDialog'
import type { ChatBackend } from '@/lib/backend'
import { errText } from '@/lib/errors'
import type { Message, ModelId, ModelOption } from '@/types/chat'
import { formatModelLabel } from './model-picker-presentation'

interface Options {
  backend: ChatBackend
  sessionId: string
  currentModel: ModelId | null
  models: ModelOption[]
  draft: boolean
  disabled: boolean
  revision: number
  transcript: Message[]
  onSwitch: (model: ModelId) => void
  onCompacted: (switchApplied: boolean) => void
}

function withoutOwnCompactionMarker(
  messages: Message[],
  entryId: string,
): Message[] | null {
  const indexes = messages
    .map((message, index) => (message.id === entryId ? index : -1))
    .filter((index) => index >= 0)
  if (indexes.length !== 1) return null
  return messages.filter((_, index) => index !== indexes[0])
}

function changedOnlyByOwnCompaction(
  startTranscript: Message[],
  currentTranscript: Message[],
  entryId: string,
): boolean {
  if (startTranscript.some((message) => message.id === entryId)) return false
  const withoutMarker = withoutOwnCompactionMarker(currentTranscript, entryId)
  return (
    withoutMarker !== null &&
    currentTranscript.length === startTranscript.length + 1 &&
    JSON.stringify(withoutMarker) === JSON.stringify(startTranscript)
  )
}

/** Selection stays tentative until the user consents and compaction succeeds. */
export function useModelSwitch(options: Options) {
  const { confirm, dialog } = useConfirm()
  const [phase, setPhase] = useState<
    'checking' | 'confirming' | 'compacting' | null
  >(null)
  const [error, setError] = useState<string | null>(null)
  const current = useRef(options)
  current.current = options
  const active = useRef<symbol | null>(null)
  useEffect(
    () => () => {
      active.current = null
    },
    [],
  )

  const request = async (
    model: ModelId,
    thinkingLevel?: string,
  ): Promise<boolean> => {
    const start = current.current
    const startTranscript = start.transcript.slice()
    if (start.disabled || active.current) return false
    if (model === start.currentModel) return true
    const token = Symbol('model-switch')
    active.current = token
    const activeRequest = () => active.current === token
    const valid = () =>
      activeRequest() &&
      current.current.sessionId === start.sessionId &&
      current.current.currentModel === start.currentModel &&
      !current.current.disabled
    setError(null)
    setPhase('checking')
    try {
      // A real persisted session must not bypass a failed/missing preview.
      if (
        !start.draft &&
        start.backend.id === 'real' &&
        !start.backend.previewModelSwitch
      ) {
        throw new Error('Context checks are unavailable.')
      }
      const preview =
        !start.draft && start.backend.previewModelSwitch
          ? await start.backend.previewModelSwitch(
              start.sessionId,
              model,
              thinkingLevel,
            )
          : null
      if (!activeRequest()) return false
      if (current.current.revision !== start.revision)
        throw new Error(
          'The conversation changed while checking. Select the model again.',
        )
      if (!valid())
        throw new Error(
          'The model switch is no longer available. Select the model again.',
        )
      if (preview?.needsCompaction) {
        const label = formatModelLabel(
          start.models.find((option) => option.id === model)?.label ?? model,
        )
        setPhase('confirming')
        const accepted = await confirm({
          title: 'Compact this conversation to switch models?',
          description: (
            <>
              This conversation’s history exceeds the context limit of{' '}
              <strong className="font-medium text-ink">{label}</strong>.
              <br />
              <br />
              To continue, {label} will need to summarize the older parts of
              this conversation. Recent messages will be preserved, but some
              older details may be lost in the summary.
            </>
          ),
          cancelLabel: 'Cancel switch',
          confirmLabel: 'Compact and switch',
        })
        if (!accepted) return false
        if (!activeRequest()) return false
        if (!valid())
          throw new Error(
            'The model switch is no longer available. Select the model again.',
          )
        if (current.current.revision !== start.revision)
          throw new Error(
            'The conversation changed while the confirmation was open. Select the model again to check its updated context.',
          )
        if (!start.backend.compactSession)
          throw new Error('Compaction is unavailable.')
        setPhase('compacting')
        // Do not pass catalog window shortcuts: resolve the actual Router
        // output budget server-side, just as the read-only preview does.
        const result = await start.backend.compactSession(
          start.sessionId,
          model,
        )
        if (!activeRequest()) return false
        if (result.status !== 'ok') {
          throw new Error(
            result.status === 'busy'
              ? 'The conversation is busy. Wait for it to finish and try again.'
              : result.status === 'empty'
                ? 'There is not enough history to compact.'
                : result.message,
          )
        }
        const transcriptChanged =
          current.current.revision !== start.revision ||
          JSON.stringify(current.current.transcript) !==
            JSON.stringify(startTranscript)
        const ownMarkerChange = changedOnlyByOwnCompaction(
          startTranscript,
          current.current.transcript,
          result.compactionEntryId,
        )
        if (!valid() || (transcriptChanged && !ownMarkerChange)) {
          start.onCompacted(false)
          throw new Error(
            transcriptChanged
              ? 'The conversation was compacted, but the model switch was not applied because the conversation changed. Select the model again.'
              : 'The conversation was compacted, but the model switch was not applied because the current state no longer allows it. Select the model again.',
          )
        }
        start.onCompacted(true)
      }
      if (!valid())
        throw new Error(
          'The model switch is no longer available. Select the model again.',
        )
      // Apply the destination exactly once for both fitting and compacted paths.
      start.onSwitch(model)
      return true
    } catch (cause) {
      if (active.current === token) {
        const modelStatus =
          current.current.currentModel === start.currentModel
            ? ' Your current model has not changed.'
            : ' The current model changed elsewhere. Select it again.'
        setError(`${errText(cause)}${modelStatus}`)
      }
      return false
    } finally {
      if (active.current === token) {
        active.current = null
        setPhase(null)
      }
    }
  }

  return {
    request,
    dialog,
    pending: phase !== null,
    error,
    notice:
      phase === 'checking'
        ? 'Checking the selected model’s context…'
        : phase === 'compacting'
          ? 'Compacting the conversation before switching models…'
          : null,
  }
}
