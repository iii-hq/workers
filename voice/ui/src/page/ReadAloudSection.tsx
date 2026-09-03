/**
 * The Read aloud section: type or paste text and hear it through the
 * configured backend (the host speech command, or an OpenAI-compatible
 * speech endpoint played in the browser), with a Stop control.
 */

import { Button, Chip, type Host, StatusPanel } from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import { speak, speakStop } from '../lib/client'
import { errorMessage } from '../lib/format'
import { SpeakerIcon, StopIcon } from '../lib/icons'
import type { DoctorResponse } from '../lib/types'
import { Fact, Facts, SectionCard } from './shared'

const SAMPLE = 'Build finished. Three tests failed in the billing module; the rest passed.'

type SpeakState =
  | { phase: 'idle' }
  | { phase: 'loading' }
  | { phase: 'speaking'; speechId?: string }
  | { phase: 'error'; message: string }

export function ReadAloudSection({ host, report }: { host: Host; report: DoctorResponse }) {
  const [text, setText] = useState(SAMPLE)
  const [state, setState] = useState<SpeakState>({ phase: 'idle' })
  const audioRef = useRef<HTMLAudioElement | null>(null)
  const timerRef = useRef<number | null>(null)
  const { tts } = report
  const disabled = tts.backend === 'off' || !tts.available

  const clearTimer = () => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current)
      timerRef.current = null
    }
  }

  useEffect(
    () => () => {
      audioRef.current?.pause()
      audioRef.current = null
      if (timerRef.current !== null) window.clearTimeout(timerRef.current)
    },
    [],
  )

  const onSpeak = async () => {
    const body = text.trim()
    if (!body) return
    setState({ phase: 'loading' })
    try {
      const res = await speak(host.iii, { text: body })
      if (res.audio_base64) {
        const audio = new Audio(`data:${res.mime ?? 'audio/mpeg'};base64,${res.audio_base64}`)
        audioRef.current = audio
        audio.onended = () => setState({ phase: 'idle' })
        audio.onerror = () => setState({ phase: 'error', message: 'playback failed' })
        await audio.play()
        setState({ phase: 'speaking', speechId: res.speech_id })
      } else if (res.played) {
        setState({ phase: 'speaking', speechId: res.speech_id })
        const words = body.split(/\s+/).length
        clearTimer()
        timerRef.current = window.setTimeout(
          () => setState({ phase: 'idle' }),
          Math.min(60_000, Math.max(1500, (words / 150) * 60_000)),
        )
      } else {
        setState({ phase: 'idle' })
      }
    } catch (err) {
      setState({ phase: 'error', message: errorMessage(err) })
    }
  }

  const onStop = () => {
    audioRef.current?.pause()
    audioRef.current = null
    clearTimer()
    const speechId = state.phase === 'speaking' ? state.speechId : undefined
    speakStop(host.iii, speechId ? { speech_id: speechId } : {}).catch(() => {})
    setState({ phase: 'idle' })
  }

  return (
    <>
      <SectionCard
        title="Read aloud"
        actions={
          <Chip tone={tts.available ? 'success' : 'neutral'}>
            {tts.backend === 'host' ? (tts.command ?? 'host') : tts.backend}
          </Chip>
        }
      >
        <textarea
          className="voice-textarea"
          value={text}
          onChange={(event) => setText(event.target.value)}
          rows={4}
          aria-label="text to read aloud"
          placeholder="Type something to hear it…"
        />
        <div className="voice-inline-actions">
          {state.phase === 'speaking' ? (
            <Button variant="primary" onClick={onStop}>
              <StopIcon />
              Stop
            </Button>
          ) : (
            <Button
              variant="primary"
              onClick={onSpeak}
              disabled={disabled || state.phase === 'loading' || !text.trim()}
            >
              <SpeakerIcon />
              {state.phase === 'loading' ? 'preparing…' : 'Speak'}
            </Button>
          )}
          <Button variant="ghost" onClick={() => setText(SAMPLE)} disabled={text === SAMPLE}>
            Reset sample
          </Button>
        </div>
        {state.phase === 'error' ? (
          <StatusPanel variant="alert" headline="Could not speak" detail={state.message} />
        ) : null}
        {disabled ? (
          <StatusPanel
            variant="warn"
            headline={tts.backend === 'off' ? 'Read-aloud is off' : 'No speech command found'}
            detail={
              tts.backend === 'off'
                ? 'Set tts.backend to host or openai in the voice configuration.'
                : 'Install say (macOS) or espeak-ng (Linux) on the worker machine, or switch tts.backend to openai.'
            }
          />
        ) : null}
      </SectionCard>
      <SectionCard title="How replies are read">
        <Facts>
          <Fact label="In chat">
            <span className="voice-sub">Each finished turn shows a Read aloud action above the composer.</span>
          </Fact>
          <Fact label="Where it plays">
            <span className="voice-sub">
              {tts.backend === 'openai'
                ? 'Audio comes back to this browser.'
                : 'On the machine running the worker; use the openai backend for remote setups.'}
            </span>
          </Fact>
        </Facts>
      </SectionCard>
    </>
  )
}
