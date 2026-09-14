/** Read the last assistant reply in this browser, never on the worker's host. */

import type { Host, SessionTurnSummaryProps, SessionTurnSummaryRegistration } from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import { doctor, speak } from '../lib/client'
import { errorMessage } from '../lib/format'
import { SpeakerIcon } from '../lib/icons'
import { useBrowserPlayback } from '../lib/playback'
import { fetchSpokenReply, selectedChatText, subscribeAutoReplies, type SpokenReply } from '../lib/voice-chat'

export function createVoiceTurnSummary(host: Host): SessionTurnSummaryRegistration {
  function VoiceTurnSummary({ sessionId, isStreaming }: SessionTurnSummaryProps) {
    const [ttsOff, setTtsOff] = useState(false)
    const { state: speakState, play, stop: onStop } = useBrowserPlayback()
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
      if (!autoRead || ttsOff) return
      try {
        return subscribeAutoReplies(host.iii, sessionId, {
          initialReplyId: replyRef.current?.id,
          readReply: (turnId) => fetchSpokenReply(host.iii, sessionId, turnId),
          onStarted: onStop,
          onReply: (reply) => {
            setLastReply(reply)
            void play(() => speak(host.iii, { text: reply.text }))
          },
          onError: (error) => setAutoError(errorMessage(error)),
        })
      } catch (error) {
        setAutoError(errorMessage(error))
      }
    }, [sessionId, autoRead, ttsOff, play, onStop])

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

    useEffect(() => {
      let cancelled = false
      doctor(host.iii)
        .then((res) => {
          if (!cancelled) setTtsOff(res.tts.backend === 'off' || !res.tts.available)
        })
        .catch(() => {
          if (!cancelled) setTtsOff(false)
        })
      return () => {
        cancelled = true
      }
    }, [])

    // A different chat must not inherit playback or a pending response.
    useEffect(() => () => onStop(), [sessionId, onStop])

    // Surface autoplay policy failures and stop automatic retries until the user opts in again.
    useEffect(() => { if (speakState.phase === 'error') setAutoSession(null) }, [speakState.phase])

    const onReadAloud = () => {
      if (lastReply) void play(() => speak(host.iii, { text: lastReply.text }))
    }


    const busy = speakState.phase === 'speaking' || speakState.phase === 'loading'

    return (
      <div className="voice-turn-summary">
        <button type="button" className="voice-turn-action" aria-pressed={autoRead}
          disabled={ttsOff} title="Read new completed replies automatically in this browser. Does not send messages or keep the microphone open."
          onClick={() => { setAutoError(null); setAutoSession(autoRead ? null : sessionId); if (autoRead) onStop() }}>
          <SpeakerIcon />
          {autoRead ? 'Voice chat on' : 'Voice chat'}
        </button>
        {selected && !busy ? <button type="button" className="voice-turn-action" disabled={ttsOff}
          onPointerDown={(event) => event.preventDefault()}
          onClick={() => { const text = selected; void play(() => speak(host.iii, { text })) }}
          title={`Read only the selected passage (${selected.length} characters)`}>
          <SpeakerIcon />Read selection
        </button> : null}
        <button
          type="button"
          className="voice-turn-action"
          disabled={ttsOff || (!busy && (!hasReply || isStreaming))}
          title={ttsOff ? 'text-to-speech is off' : busy ? 'Stop reading' : 'Read the last reply aloud'}
          aria-label={busy ? 'Stop reading aloud' : 'Read aloud'}
          onClick={busy ? () => { setAutoSession(null); onStop() } : onReadAloud}
        >
          <SpeakerIcon />
          <span>{busy ? 'Stop' : 'Read aloud'}</span>
        </button>
        {autoError ? <span className="voice-turn-error" role="alert">{autoError}</span> : null}
        {autoRead ? <span className="voice-sub">New replies will be read automatically here.</span> : null}
        {speakState.phase === 'error' ? <span className="voice-turn-error">{speakState.message}</span> : null}
      </div>
    )
  }

  return { id: 'voice-read-aloud', render: VoiceTurnSummary }
}
