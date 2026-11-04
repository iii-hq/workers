/**
 * The voice page (page `voice`): the standard page chrome with a section
 * sidebar (Overview, Dictate, Transcribe, Models, Read aloud) over one
 * `voice::doctor` snapshot that every section reads. The header refreshes
 * the snapshot; notices and errors land as status panels above the section.
 * A palette command's `panelContext` (`{ action: 'dictate' | 'transcribe' }`)
 * opens the matching section and starts it.
 */

import {
  type Host,
  IconButton,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  StatusDot,
  StatusPanel,
  WorkerConfigurationDialog,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { Activity, FileMusic, Layers, type LucideIcon, Mic, RefreshCw, Volume2 } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { dictationList, doctor, modelsList } from '../lib/client'
import type { DictationController } from '../lib/dictation'
import { useDictation } from '../lib/dictation'
import { useModelProgress } from '../lib/progress'
import type { DictationListEntry, DoctorResponse, ModelsListResponse } from '../lib/types'
import { DictateSection } from './DictateSection'
import { ModelsSection } from './ModelsSection'
import { type Notice, Overview } from './Overview'
import { ReadAloudSection } from './ReadAloudSection'
import { SectionNav } from '../ui/section-nav'
import { LoadingRows } from './shared'
import { TranscribeSection } from './TranscribeSection'

type SectionId = 'overview' | 'dictate' | 'transcribe' | 'models' | 'speak'

const SECTIONS: { id: SectionId; label: string; icon: LucideIcon }[] = [
  { id: 'overview', label: 'Overview', icon: Activity },
  { id: 'dictate', label: 'Dictate', icon: Mic },
  { id: 'transcribe', label: 'Transcribe', icon: FileMusic },
  { id: 'models', label: 'Models', icon: Layers },
  { id: 'speak', label: 'Read aloud', icon: Volume2 },
]

/** The read-aloud engine in a person's words, for the header line. */
const TTS_NAMES: Record<string, string> = {
  piper: 'Piper voice',
  host: 'System voice',
  router: 'Provider voice',
  openai: 'OpenAI-compatible voice',
}

export function VoicePage({
  host,
  controller,
  panelSide,
  onRequestClose,
  panelContext,
}: { host: Host; controller: DictationController } & PageRenderProps) {
  const [section, setSection] = useState<SectionId>('overview')
  const [report, setReport] = useState<DoctorResponse | null>(null)
  const [reportError, setReportError] = useState<string | null>(null)
  const [models, setModels] = useState<ModelsListResponse | null>(null)
  const [sessions, setSessions] = useState<readonly DictationListEntry[]>([])
  const [refreshing, setRefreshing] = useState(false)
  const [notice, setNotice] = useState<Notice>(null)
  const [configuring, setConfiguring] = useState(false)
  const [focusTranscribe, setFocusTranscribe] = useState(0)
  const [autoDictate, setAutoDictate] = useState(0)
  const appliedContextRef = useRef(0)
  const { state: dictation } = useDictation(controller)
  const listening = dictation.status === 'listening' || dictation.status === 'starting'

  const refresh = useCallback(async () => {
    setRefreshing(true)
    try {
      const [nextReport, nextSessions] = await Promise.all([doctor(host.iii), dictationList(host.iii)])
      setReport(nextReport)
      setSessions(nextSessions.sessions)
      setReportError(null)
      // Dictation always uses a local live model, even with a remote final pass.
      setModels(await modelsList(host.iii))
    } catch (err) {
      setReportError(errorMessage(err))
    } finally {
      setRefreshing(false)
    }
  }, [host])

  useEffect(() => {
    void refresh()
  }, [refresh])

  useEffect(() => {
    if (dictation.status === 'idle' || dictation.status === 'listening') void refresh()
  }, [dictation.status, refresh])

  const onProgressDone = useCallback(() => {
    void refresh()
  }, [refresh])
  const progress = useModelProgress(host, onProgressDone)

  useEffect(() => {
    if (!panelContext || panelContext.id === appliedContextRef.current) return
    appliedContextRef.current = panelContext.id
    const context = panelContext.context
    const action =
      context && typeof context === 'object' && !Array.isArray(context)
        ? (context as Record<string, unknown>).action
        : null
    if (action === 'dictate') {
      setSection('dictate')
      setAutoDictate((n) => n + 1)
    } else if (action === 'transcribe') {
      setSection('transcribe')
      setFocusTranscribe((n) => n + 1)
    }
  }, [panelContext])

  // What listens and what speaks, by name: the transcript model (or the live
  // one alone) and the read-aloud engine, never raw model ids.
  const description = useMemo(() => {
    if (!report) return 'Speech to text and text to speech'
    const nameOf = (id: string) => models?.models.find((m) => m.id === id)?.name ?? id
    const { stt, tts } = report
    const finalReady = stt.final_state === 'loaded' || stt.final_state === 'installed'
    const listens =
      stt.backend === 'local'
        ? nameOf(finalReady && stt.final_model ? stt.final_model : stt.live_model)
        : nameOf(stt.model)
    const speaks = tts.backend === 'off' ? 'Read aloud off' : (TTS_NAMES[tts.backend] ?? tts.backend)
    return `${listens} · ${speaks}`
  }, [report, models])

  const sectionMeta = (id: SectionId) => {
    switch (id) {
      case 'dictate':
        return listening ? <StatusDot tone="accent" pulse /> : null
      case 'models':
        return models ? (
          <span className="voice-nav-count">{models.models.filter((m) => m.installed).length}</span>
        ) : null
      case 'speak':
        return report && !report.tts.available && report.tts.backend !== 'off' ? <StatusDot tone="warn" /> : null
      default:
        return null
    }
  }

  const content = (() => {
    if (!report) return reportError ? null : <LoadingRows rows={4} />
    switch (section) {
      case 'overview':
        return (
          <Overview
            host={host}
            report={report}
            models={models}
            sessions={sessions}
            progress={progress}
            onDictate={() => {
              setSection('dictate')
              setAutoDictate((n) => n + 1)
            }}
            onConfigure={() => setConfiguring(true)}
            onNotice={setNotice}
            onChanged={() => void refresh()}
          />
        )
      case 'dictate':
        return <DictateSection host={host} controller={controller} autoStartSignal={autoDictate} />
      case 'transcribe':
        return <TranscribeSection host={host} focusSignal={focusTranscribe} />
      case 'models':
        return (
          <ModelsSection
            host={host}
            report={report}
            models={models}
            progress={progress}
            onNotice={setNotice}
            onChanged={() => void refresh()}
          />
        )
      case 'speak':
        return <ReadAloudSection host={host} report={report} />
      default:
        return null
    }
  })()

  return (
    <PageShell className="voice-shell">
      <PageHeader
        icon={<Mic />}
        title="Voice"
        description={description}
        actions={
          <IconButton label="Refresh" variant="ghost" disabled={refreshing} onClick={() => void refresh()}>
            <RefreshCw className={refreshing ? host.uiClasses.spin : undefined} />
          </IconButton>
        }
        onClose={onRequestClose}
      />
      <PageBody side={panelSide}>
        <PageSidebar
          label="Sections"
          header={<span className="voice-side-label">Sections</span>}
          side={panelSide}
          collapsible
          resizable
          storageKey="voice:sections"
          defaultWidth={224}
          minWidth={176}
          maxWidth={320}
          narrowBelow={640}
          narrowMode="drawer"
        >
          <SectionNav
            aria-label="Voice sections"
            items={SECTIONS.map((item) => ({ ...item, meta: sectionMeta(item.id) }))}
            current={section}
            onSelect={setSection}
          />
        </PageSidebar>
        <PageMain className="voice-main">
          {reportError ? <StatusPanel variant="alert" headline="voice::doctor failed" detail={reportError} /> : null}
          {notice ? (
            <StatusPanel
              variant={notice.kind === 'error' ? 'alert' : 'success'}
              headline={notice.kind === 'error' ? 'Something failed' : 'Done'}
              detail={notice.text}
            />
          ) : null}
          <div key={section} className={`voice-section ${host.uiClasses.motionPanel}`}>
            {content}
          </div>
        </PageMain>
      </PageBody>
      <WorkerConfigurationDialog
        configurationId={configuring ? 'voice' : null}
        onClose={() => {
          setConfiguring(false)
          void refresh()
        }}
      />
    </PageShell>
  )
}
