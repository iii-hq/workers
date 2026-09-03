/**
 * The `voice-read-aloud` turn-summary registration: a compact row above the
 * composer with a "Read aloud" action. It walks `session::messages` for the
 * last assistant text, strips markdown code fences, and calls
 * `voice::speak`. The `openai` backend returns `audio_base64`, played
 * through an `<audio>` element with a real `onended`. The `host` backend
 * returns as soon as playback STARTS (`played: true`, no audio) — "Stop" is
 * hidden by whichever comes first: an estimated duration (~150 wpm, capped
 * at 60s) or `voice::doctor` reporting `tts.playing === 0`, polled no
 * faster than every 2s. "Stop" calls `voice::speak::stop` with the
 * utterance's `speech_id`. Hidden while the turn is streaming; disabled
 * with a tooltip when the doctor reports the tts backend `off`.
 */

import type { Host, SessionTurnSummaryProps, SessionTurnSummaryRegistration } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import { doctor, speak, speakStop } from '../lib/client'
import { errorMessage, stripCodeFences } from '../lib/format'
import { SpeakerIcon } from '../lib/icons'
import type { SessionContentBlock, SessionMessageEntry, SessionMessagesResponse } from '../lib/types'

const WORDS_PER_MINUTE = 150
const MAX_ESTIMATE_SECS = 60
const POLL_INTERVAL_MS = 2000
const MAX_PAGES = 20

function extractAssistantText(entry: SessionMessageEntry): string | null {
  const blocks = entry.message?.content
  if (!blocks) return null
  const text = blocks
    .filter((block): block is SessionContentBlock & { text: string } => block.type === 'text' && !!block.text)
    .map((block) => block.text)
    .join('\n')
    .trim()
  return text ? stripCodeFences(text) : null
}

async function fetchLastAssistantText(host: Host, sessionId: string): Promise<string | null> {
  let cursor: string | undefined
  let lastText: string | null = null
  for (let page = 0; page < MAX_PAGES; page++) {
    const res = await host.iii.trigger<SessionMessagesResponse>('session::messages', {
      session_id: sessionId,
      roles: ['assistant'],
      limit: 100,
      cursor,
    })
    for (const entry of res.messages) {
      const text = extractAssistantText(entry)
      if (text) lastText = text
    }
    if (!res.next_cursor) break
    cursor = res.next_cursor
  }
  return lastText
}

function estimateSpeechSeconds(text: string): number {
  const words = text.trim().split(/\s+/).filter(Boolean).length
  if (words === 0) return 1
  return Math.min(MAX_ESTIMATE_SECS, Math.max(1, (words / WORDS_PER_MINUTE) * 60))
}

type SpeakState =
  | { phase: 'idle' }
  | { phase: 'loading' }
  | { phase: 'speaking'; speechId?: string }
  | { phase: 'error'; message: string }

export function createVoiceTurnSummary(host: Host): SessionTurnSummaryRegistration {
  function VoiceTurnSummary({ sessionId, isStreaming }: SessionTurnSummaryProps) {
    const [ttsOff, setTtsOff] = useState(false)
    const [speakState, setSpeakState] = useState<SpeakState>({ phase: 'idle' })
    const audioRef = useRef<HTMLAudioElement | null>(null)
    const estimateTimerRef = useRef<number | null>(null)
    const pollTimerRef = useRef<number | null>(null)
    const [hasReply, setHasReply] = useState(false)

    useEffect(() => {
      if (isStreaming) return
      let cancelled = false
      fetchLastAssistantText(host, sessionId)
        .then((text) => {
          if (!cancelled) setHasReply(Boolean(text))
        })
        .catch(() => {
          if (!cancelled) setHasReply(false)
        })
      return () => {
        cancelled = true
      }
    }, [sessionId, isStreaming])

    useEffect(() => {
      let cancelled = false
      doctor(host.iii)
        .then((res) => {
          if (!cancelled) setTtsOff(res.tts.backend === 'off')
        })
        .catch(() => {
          if (!cancelled) setTtsOff(false)
        })
      return () => {
        cancelled = true
      }
    }, [])

    const clearWatch = useCallback(() => {
      if (estimateTimerRef.current !== null) {
        window.clearTimeout(estimateTimerRef.current)
        estimateTimerRef.current = null
      }
      if (pollTimerRef.current !== null) {
        window.clearInterval(pollTimerRef.current)
        pollTimerRef.current = null
      }
    }, [])

    const armHostPlaybackWatch = useCallback(
      (estimateSecs: number) => {
        clearWatch()
        estimateTimerRef.current = window.setTimeout(() => {
          clearWatch()
          setSpeakState({ phase: 'idle' })
        }, estimateSecs * 1000)
        pollTimerRef.current = window.setInterval(() => {
          doctor(host.iii)
            .then((res) => {
              if (res.tts.playing === 0) {
                clearWatch()
                setSpeakState({ phase: 'idle' })
              }
            })
            .catch(() => {})
        }, POLL_INTERVAL_MS)
      },
      [clearWatch],
    )

    useEffect(
      () => () => {
        clearWatch()
        audioRef.current?.pause()
      },
      [clearWatch],
    )

    const onReadAloud = useCallback(async () => {
      setSpeakState({ phase: 'loading' })
      try {
        const text = await fetchLastAssistantText(host, sessionId)
        if (!text) {
          setSpeakState({ phase: 'idle' })
          return
        }
        const res = await speak(host.iii, { text })
        if (res.audio_base64) {
          const mime = res.mime ?? 'audio/mpeg'
          const audio = new Audio(`data:${mime};base64,${res.audio_base64}`)
          audioRef.current = audio
          audio.onended = () => setSpeakState({ phase: 'idle' })
          audio.onerror = () => setSpeakState({ phase: 'error', message: 'playback failed' })
          await audio.play()
          setSpeakState({ phase: 'speaking', speechId: res.speech_id })
        } else if (res.played) {
          setSpeakState({ phase: 'speaking', speechId: res.speech_id })
          armHostPlaybackWatch(estimateSpeechSeconds(text))
        } else {
          setSpeakState({ phase: 'idle' })
        }
      } catch (err) {
        setSpeakState({ phase: 'error', message: errorMessage(err) })
      }
    }, [sessionId, armHostPlaybackWatch])

    const onStop = useCallback(() => {
      audioRef.current?.pause()
      audioRef.current = null
      clearWatch()
      const speechId = speakState.phase === 'speaking' ? speakState.speechId : undefined
      speakStop(host.iii, speechId ? { speech_id: speechId } : {}).catch(() => {})
      setSpeakState({ phase: 'idle' })
    }, [clearWatch, speakState])

    if (isStreaming || !hasReply) return null

    const busy = speakState.phase === 'speaking' || speakState.phase === 'loading'

    return (
      <div className="voice-turn-summary">
        <button
          type="button"
          className="voice-turn-action"
          disabled={ttsOff}
          title={ttsOff ? 'text-to-speech is off' : busy ? 'Stop reading' : 'Read the last reply aloud'}
          aria-label={busy ? 'Stop reading aloud' : 'Read aloud'}
          onClick={busy ? onStop : onReadAloud}
        >
          <SpeakerIcon />
          <span>{busy ? 'Stop' : 'Read aloud'}</span>
        </button>
        {speakState.phase === 'error' ? <span className="voice-turn-error">{speakState.message}</span> : null}
      </div>
    )
  }

  return { id: 'voice-read-aloud', render: VoiceTurnSummary }
}
