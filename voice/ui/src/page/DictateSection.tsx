/**
 * The Dictate section: a large mic control on the shared dictation
 * controller, the transcript as it forms (committed utterances as rows, the
 * in-progress words in accent), and what to do with it: send to the chat
 * composer, copy, clear.
 */

import { Button, Chip, EmptyState, type Host, IconButton, StatusDot } from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import type { DictationController } from '../lib/dictation'
import { useDictation } from '../lib/dictation'
import { CopyIcon, MicIcon, SendIcon, StopIcon, TrashIcon } from '../lib/icons'
import { SectionCard } from './shared'

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
  const [copied, setCopied] = useState(false)
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

  const copy = () => {
    if (!hasText) return
    navigator.clipboard
      .writeText(text.trim())
      .then(() => {
        setCopied(true)
        window.setTimeout(() => setCopied(false), COPIED_MS)
      })
      .catch(() => {})
  }

  const clear = () => {
    setDraft([])
    setCleared(true)
    if (listening) cancel()
  }

  const statusLabel = (() => {
    switch (state.status) {
      case 'starting':
        return 'starting'
      case 'listening':
        return 'listening'
      case 'stopping':
        return 'finishing'
      case 'error':
        return 'error'
      default:
        return 'ready'
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
        actions={
          <span className="voice-fact-line">
            <StatusDot tone={statusTone} pulse={listening} />
            <span className="voice-sub">{statusLabel}</span>
          </span>
        }
      >
        <div className="voice-hero">
          {listening ? (
            <Button variant="primary" size="lg" className="voice-hero-btn" onClick={onStop}>
              <StopIcon />
              Stop and keep text
            </Button>
          ) : (
            <Button
              variant="primary"
              size="lg"
              className="voice-hero-btn"
              onClick={onStart}
              disabled={state.status === 'stopping'}
            >
              <MicIcon />
              Start dictation
            </Button>
          )}
          <p className="voice-note">
            Speak in whole sentences. Each pause commits an utterance; the final text is re-decoded for punctuation. In
            chat, hold the mic beside attach to talk and release to insert.
          </p>
        </div>
        {state.status === 'error' && state.error ? <p className="voice-note voice-alert">{state.error}</p> : null}
      </SectionCard>

      <SectionCard
        title="Transcript"
        actions={
          <span className="voice-card-actions">
            {copied ? <Chip tone="success">copied</Chip> : null}
            <IconButton label="Copy transcript" variant="ghost" onClick={copy} disabled={!hasText}>
              <CopyIcon />
            </IconButton>
            <IconButton label="Clear" variant="ghost" onClick={clear} disabled={!hasText && !listening}>
              <TrashIcon />
            </IconButton>
            {host.chat?.compose ? (
              <Button variant="primary" size="sm" onClick={sendToChat} disabled={!hasText}>
                <SendIcon />
                Send to chat
              </Button>
            ) : null}
          </span>
        }
      >
        {!hasText ? (
          <EmptyState
            icon={MicIcon}
            title={listening ? 'Listening' : 'Nothing dictated yet'}
            description={
              listening
                ? 'Words appear here as the recognizer hears them.'
                : 'Start dictation and speak; the transcript collects here.'
            }
          />
        ) : (
          <output className="voice-transcript" aria-live="polite">
            {entries.map((entry) => (
              <p key={entry.id} className="voice-transcript-line">
                {entry.text}
              </p>
            ))}
            {listening && state.partial ? <p className="voice-transcript-partial">{state.partial}</p> : null}
          </output>
        )}
      </SectionCard>
    </>
  )
}
