/**
 * The voice page (`#/ext/voice`): standard `PageShell`/`PageHeader` chrome
 * over three stacked sections — worker status, a one-off file/path
 * transcription panel, and a dictation test panel that exercises
 * `useDictation` without depending on a chat composer. A palette command's
 * `panelContext` (`{ action: 'dictate' | 'transcribe' }`, set up in
 * `page.tsx`) auto-starts dictation or focuses the file picker on arrival.
 */

import { type Host, PageBody, PageHeader, PageMain, type PageRenderProps, PageShell } from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import type { DictationController } from '../lib/dictation'
import { MicIcon } from '../lib/icons'
import { DictationSection } from './DictationSection'
import { StatusSection } from './StatusSection'
import { TranscribeSection } from './TranscribeSection'

export function VoicePage({
  host,
  controller,
  panelSide,
  onRequestClose,
  panelContext,
}: { host: Host; controller: DictationController } & PageRenderProps) {
  const [focusTranscribe, setFocusTranscribe] = useState(0)
  const [autoDictate, setAutoDictate] = useState(0)
  const appliedContextRef = useRef(0)

  useEffect(() => {
    if (!panelContext || panelContext.id === appliedContextRef.current) return
    appliedContextRef.current = panelContext.id
    const context = panelContext.context
    const action =
      context && typeof context === 'object' && !Array.isArray(context)
        ? (context as Record<string, unknown>).action
        : null
    if (action === 'dictate') setAutoDictate((n) => n + 1)
    else if (action === 'transcribe') setFocusTranscribe((n) => n + 1)
  }, [panelContext])

  return (
    <PageShell className="voice-ui">
      <PageHeader
        icon={<MicIcon />}
        title="Voice"
        description="Speech to text and text to speech"
        onClose={onRequestClose}
      />
      <PageBody side={panelSide}>
        <PageMain className="voice-page-main">
          <StatusSection host={host} />
          <TranscribeSection host={host} focusSignal={focusTranscribe} />
          <DictationSection controller={controller} autoStartSignal={autoDictate} />
        </PageMain>
      </PageBody>
    </PageShell>
  )
}
