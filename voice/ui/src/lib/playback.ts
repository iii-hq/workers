/**
 * Host read-aloud ends on the worker's side, so the worker says so: this
 * hook binds a browser function to the `voice::speech-ended` trigger for
 * as long as the component lives, the same way the shell page listens for
 * workspace changes. No timers, no doctor polling.
 */

import type { Host } from '@iii-dev/console-ui'
import { useEffect, useRef } from 'react'
import type { SpeechEndedEvent } from './types'

const EVENTS_FN = 'iii::voice-ui::speech-ended'

export function useSpeechEnded(host: Host, onEnded: (event: SpeechEndedEvent) => void): void {
  const handlerRef = useRef(onEnded)
  handlerRef.current = onEnded
  useEffect(() => {
    const offHandler = host.iii.on<SpeechEndedEvent>(EVENTS_FN, (event) => {
      if (typeof event?.speech_id !== 'string') return
      handlerRef.current(event)
    })
    const offTrigger = host.iii.registerTrigger({
      type: 'voice::speech-ended',
      function_id: `${EVENTS_FN}::${host.iii.browserId}`,
      config: {},
    })
    return () => {
      offTrigger()
      offHandler()
    }
  }, [host])
}
