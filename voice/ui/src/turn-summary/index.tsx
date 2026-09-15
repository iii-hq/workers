/** Read the last assistant reply in this browser, never on the worker's host. */

import type { Host, SessionTurnSummaryProps, SessionTurnSummaryRegistration } from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import { speak } from '../lib/client'
import { errorMessage } from '../lib/format'
import { SpeakerIcon } from '../lib/icons'
import { useBrowserPlayback } from '../lib/playback'
import { fetchSpokenReply, selectedChatText, subscribeAutoReplies, type SpokenReply } from '../lib/voice-chat'

export function createVoiceTurnSummary(host: Host): SessionTurnSummaryRegistration {
  function VoiceTurnSummary({ sessionId, isStreaming }: SessionTurnSummaryProps) {
    const { state: speakState, play, enqueue, stop: onStop } = useBrowserPlayback()
    const [lastReply, setLastReply] = useState<SpokenReply | null>(null)
    const [selected, setSelected] = useState('')
    const [autoSession, setAutoSession] = useState<string | null>(null)
    const [autoError, setAutoError] = useState<string | null>(null)
    const autoRead = autoSession === sessionId
    const replyRef = useRef(lastReply)
    replyRef.current = lastReply
    const hasReply = lastReply !== null

    useEffect(() => {
      setSelected('')
      setAutoSession(null)
      setAutoError(null)
      setLastReply(null)
      const update = () => setSelected(selectedChatText(window.getSelection(), sessionId))
      document.addEventListener('selectionchange', update)
      return () => document.removeEventListener('selectionchange', update)
    }, [sessionId])

    useEffect(() => {
      if (!autoRead) return
      try {
        return subscribeAutoReplies(host.iii, sessionId, {
          initialReplyId: replyRef.current?.id,
          readReply: (turnId) => fetchSpokenReply(host.iii, sessionId, turnId),
          onStarted: onStop,
          streaming: {
            prepare: (text, complete) => host.iii.trigger('voice::speech::prepare', { text, complete }),
            onChunk: (text) => enqueue(() => speak(host.iii, { text, text_format: 'plain' })),
          },
          onReply: setLastReply,
          onError: (error) => { setAutoError(errorMessage(error)); setAutoSession(null); onStop() },
        })
      } catch (error) {
        setAutoError(errorMessage(error))
      }
    }, [sessionId, autoRead, enqueue, onStop])

    useEffect(() => {
      if (isStreaming) return
      let cancelled = false
      fetchSpokenReply(host.iii, sessionId)
        .then((text) => {
          if (!cancelled) setLastReply(text)
        })
        .catch(() => {
          if (!cancelled) setLastReply(null)
        })
      return () => {
        cancelled = true
      }
    }, [sessionId, isStreaming])

    // Never freeze availability from a mount-time doctor snapshot. Each
    // explicit attempt is validated by voice::speak against the current config
    // and installed models; failures remain actionable and the buttons retryable.

    // A different chat must not inherit playback or a pending response.
    useEffect(() => () => onStop(), [sessionId, onStop])

    // Surface autoplay policy failures and stop automatic retries until the user opts in again.
    useEffect(() => { if (speakState.phase === 'error') setAutoSession(null) }, [speakState.phase])

    const onReadAloud = () => {
      if (lastReply) void play(() => speak(host.iii, { text: lastReply.text, text_format: 'markdown' }))
    }


    const busy = speakState.phase === 'speaking' || speakState.phase === 'loading'

    return (
      <div className="voice-turn-summary">
        <button type="button" className="voice-turn-action" aria-pressed={autoRead}
          title="Read assistant text as it arrives, sentence by sentence, in this browser. Does not send messages or keep the microphone open."
          onClick={() => { setAutoError(null); setAutoSession(autoRead ? null : sessionId); if (autoRead) onStop() }}>
          <SpeakerIcon />
          {autoRead ? 'Voice chat on' : 'Voice chat'}
        </button>
        {selected && !busy ? <button type="button" className="voice-turn-action"
          onPointerDown={(event) => event.preventDefault()}
          onClick={() => { const text = selected; void play(() => speak(host.iii, { text, text_format: 'plain' })) }}
          title={`Read only the selected passage (${selected.length} characters)`}>
          <SpeakerIcon />Read selection
        </button> : null}
        <button
          type="button"
          className="voice-turn-action"
          disabled={!busy && (!hasReply || isStreaming)}
          title={busy ? 'Stop reading' : 'Read the last reply aloud; uses the current voice configuration'}
          aria-label={busy ? 'Stop reading aloud' : 'Read aloud'}
          onClick={busy ? () => { setAutoSession(null); onStop() } : onReadAloud}
        >
          <SpeakerIcon />
          <span>{busy ? 'Stop' : 'Read aloud'}</span>
        </button>
        {autoError ? <span className="voice-turn-error" role="alert">{autoError}</span> : null}
        {autoRead ? <span className="voice-sub">Reading new text as it arrives, in short phrases.</span> : null}
        {speakState.phase === 'error' ? <span className="voice-turn-error">{speakState.message}</span> : null}
      </div>
    )
  }

  return { id: 'voice-read-aloud', render: VoiceTurnSummary }
}
