/**
 * The Dictate section: a large mic control on the shared dictation
 * controller, the transcript as it forms (committed utterances as rows, the
 * in-progress words in accent), and what to do with it: send to the chat
 * composer, copy, clear.
 */

import { type Host, IconButton, ShadcnButton, StatusDot } from '@iii-dev/console-ui'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { Copy, Mic, Send, Square, Trash2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import type { DictationController } from '../lib/dictation'
import { useDictation } from '../lib/dictation'
import { StatusBadge } from '../ui/status'
import { PanelBody, PanelEmpty, SectionCard } from './shared'

const COPIED_MS = 2000

export function DictateSection({
  host,
  controller,
  autoStartSignal,
}: {
  host: Host
  controller: DictationController
  autoStartSignal: number
}) {
  const { state, start, stop, cancel } = useDictation(controller)
  const [draft, setDraft] = useState<string[]>([])
  const [cleared, setCleared] = useState(false)
  const appliedSignalRef = useRef(0)

  useEffect(() => {
    if (autoStartSignal > appliedSignalRef.current) {
      appliedSignalRef.current = autoStartSignal
      setDraft([])
      setCleared(false)
      start()
    }
  }, [autoStartSignal, start])

  const listening = state.status === 'listening' || state.status === 'starting'
  const entries = (() => {
    if (cleared) return []
    if (!listening && draft.length > 0) return draft.map((text) => ({ id: `draft-${text.length}`, text }))
    return state.committed.map((text, index) => ({ id: `segment-${state.committedIds[index] ?? text.length}`, text }))
  })()
  const text = [...entries.map((entry) => entry.text), listening ? state.partial : ''].filter(Boolean).join(' ')
  const hasText = text.trim().length > 0
  const { state: copyState, copy } = useCopyFlash(text.trim(), COPIED_MS)

  const onStop = async () => {
    const result = (await stop()).trim()
    setCleared(false)
    setDraft(result ? [result] : [])
  }

  const onStart = () => {
    setDraft([])
    setCleared(false)
    start()
  }

  const sendToChat = () => {
    if (!hasText) return
    host.chat?.compose?.({ text: `${text.trim()} ` })
  }

  const clear = () => {
    setDraft([])
    setCleared(true)
    if (listening) cancel()
  }

  const statusLabel = (() => {
    switch (state.status) {
      case 'starting':
        return 'Starting'
      case 'listening':
        return 'Listening'
      case 'stopping':
        return 'Finishing'
      case 'error':
        return 'Error'
      default:
        return 'Ready'
    }
  })()
  const statusTone: 'accent' | 'alert' | 'ink' = (() => {
    if (listening) return 'accent'
    if (state.status === 'error') return 'alert'
    return 'ink'
  })()

  return (
    <>
      <SectionCard
        title="Dictate"
        description="Speak in whole sentences; each pause commits one, and the final text is re-decoded for punctuation."
        actions={
          <span className="voice-choice">
            <StatusDot tone={statusTone} pulse={listening} />
            <span className="voice-row-hint">{statusLabel}</span>
          </span>
        }
      >
        <div className="voice-hero">
          {listening ? (
            <ShadcnButton size="lg" onClick={onStop}>
              <Square />
              Stop and keep text
            </ShadcnButton>
          ) : (
            <ShadcnButton size="lg" onClick={onStart} disabled={state.status === 'stopping'}>
              <Mic />
              Start dictation
            </ShadcnButton>
          )}
          <p className="voice-row-hint">In chat, hold the mic beside attach to talk and release to insert.</p>
        </div>
        {state.status === 'error' && state.error ? <p className="voice-panel-note voice-alert">{state.error}</p> : null}
      </SectionCard>

      <SectionCard
        title="Transcript"
        actions={
          <>
            {copyState === 'copied' ? <StatusBadge tone="ok">Copied</StatusBadge> : null}
            <IconButton label="Copy transcript" variant="ghost" onClick={copy} disabled={!hasText}>
              <Copy />
            </IconButton>
            <IconButton label="Clear" variant="ghost" onClick={clear} disabled={!hasText && !listening}>
              <Trash2 />
            </IconButton>
            {host.chat?.compose ? (
              <ShadcnButton variant="secondary" size="sm" onClick={sendToChat} disabled={!hasText}>
                <Send />
                Send to chat
              </ShadcnButton>
            ) : null}
          </>
        }
      >
        {!hasText ? (
          <PanelEmpty
            title={listening ? 'Listening' : 'Nothing dictated yet'}
            description={
              listening
                ? 'Words appear here as the recognizer hears them.'
                : 'Start dictation and speak; the transcript collects here.'
            }
          />
        ) : (
          <PanelBody>
            <output className="voice-transcript" aria-live="polite">
              {entries.map((entry) => (
                <p key={entry.id} className="voice-transcript-line">
                  {entry.text}
                </p>
              ))}
              {listening && state.partial ? <p className="voice-transcript-partial">{state.partial}</p> : null}
            </output>
          </PanelBody>
        )}
      </SectionCard>
    </>
  )
}
