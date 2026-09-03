/**
 * The voice page (`#/ext/voice`): the standard page chrome with a section
 * sidebar (Overview, Dictate, Transcribe, Models, Read aloud) over one
 * `voice::doctor` snapshot that every section reads. The header refreshes
 * the snapshot; notices and errors land as status panels above the section.
 * A palette command's `panelContext` (`{ action: 'dictate' | 'transcribe' }`)
 * opens the matching section and starts it.
 */

import {
  Chip,
  type Host,
  IconButton,
  List,
  ListItem,
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
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { dictationList, doctor, modelsList } from '../lib/client'
import type { DictationController } from '../lib/dictation'
import { useDictation } from '../lib/dictation'
import { errorMessage } from '../lib/format'
import { ActivityIcon, FileAudioIcon, LayersIcon, MicIcon, RefreshIcon, SpeakerIcon } from '../lib/icons'
import { useModelProgress } from '../lib/progress'
import type { DictationListEntry, DoctorResponse, ModelsListResponse } from '../lib/types'
import { DictateSection } from './DictateSection'
import { ModelsSection } from './ModelsSection'
import { type Notice, Overview } from './Overview'
import { ReadAloudSection } from './ReadAloudSection'
import { LoadingRows } from './shared'
import { TranscribeSection } from './TranscribeSection'

type SectionId = 'overview' | 'dictate' | 'transcribe' | 'models' | 'speak'

const SECTIONS: { id: SectionId; label: string; description: string; icon: typeof MicIcon }[] = [
  { id: 'overview', label: 'Overview', description: 'Engines and sessions', icon: ActivityIcon },
  { id: 'dictate', label: 'Dictate', description: 'Speak, then send', icon: MicIcon },
  { id: 'transcribe', label: 'Transcribe', description: 'A recording to text', icon: FileAudioIcon },
  { id: 'models', label: 'Models', description: 'What is on disk', icon: LayersIcon },
  { id: 'speak', label: 'Read aloud', description: 'Text to speech', icon: SpeakerIcon },
]

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
      if (nextReport.stt.backend === 'local') {
        setModels(await modelsList(host.iii))
      } else {
        setModels(null)
      }
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

  const description = useMemo(() => {
    if (!report) return 'Speech to text and text to speech'
    const parts = [report.stt.model]
    if (report.stt.final_state === 'loaded' || report.stt.final_state === 'installed')
      parts.push(report.stt.final_model)
    if (report.tts.available && report.tts.command) parts.push(report.tts.command)
    return <span className="voice-mono">{parts.join(' + ')}</span>
  }, [report])

  const sectionMeta = (id: SectionId) => {
    switch (id) {
      case 'overview':
        return report ? <StatusDot tone={report.stt.loaded ? 'accent' : 'ink'} /> : null
      case 'dictate':
        return listening ? <Chip tone="accent">live</Chip> : null
      case 'models': {
        const installed = models?.models.filter((m) => m.installed).length
        return models ? (
          <Chip tone="neutral">
            {installed}/{models.models.length}
          </Chip>
        ) : null
      }
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
        icon={<MicIcon />}
        title="Voice"
        description={description}
        actions={
          <IconButton label="Refresh" variant="ghost" disabled={refreshing} onClick={() => void refresh()}>
            <RefreshIcon className={refreshing ? 'voice-spin' : undefined} />
          </IconButton>
        }
        onClose={onRequestClose}
      />
      <PageBody side={panelSide}>
        <PageSidebar
          label="sections"
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
          <List className="voice-side-list" aria-label="Voice sections">
            {SECTIONS.map((item) => {
              const Icon = item.icon
              return (
                <ListItem
                  key={item.id}
                  selected={section === item.id}
                  aria-current={section === item.id ? 'page' : undefined}
                  leading={<Icon className={host.uiClasses.icon} />}
                  label={item.label}
                  description={item.description}
                  trailing={sectionMeta(item.id)}
                  onClick={() => setSection(item.id)}
                />
              )
            })}
          </List>
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
          <div key={section} className="voice-section">
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
