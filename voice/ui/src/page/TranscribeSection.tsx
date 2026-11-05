/**
 * The Transcribe section: drop or choose a WAV file (read as base64, capped
 * at 10 MiB) or name a path on the worker's machine, run
 * `voice::transcribe`, and read the result with per-segment timestamps and
 * copy / send-to-chat actions.
 */

import {
  type Host,
  IconButton,
  ShadcnButton,
  ShadcnInput,
  ShadcnTable,
  ShadcnTableBody,
  ShadcnTableCell,
  ShadcnTableHead,
  ShadcnTableHeader,
  ShadcnTableRow,
  StatusPanel,
} from '@iii-dev/console-ui'
import { errorMessage, formatBytes } from '@iii-dev/console-ui/format'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { Copy, FileMusic, Send, Trash2, Upload } from 'lucide-react'
import type { ChangeEvent, DragEvent } from 'react'
import { useEffect, useRef, useState } from 'react'
import { transcribe } from '../lib/client'
import { formatSeconds } from '../lib/format'
import type { TranscribeResponse } from '../lib/types'
import { StatusBadge } from '../ui/status'
import { formatDuration, PanelBody, SectionCard } from './shared'

const MAX_BYTES = 10 * 1024 * 1024

type Result =
  | { phase: 'idle' }
  | { phase: 'loading' }
  | { phase: 'ready'; data: TranscribeResponse; source: string }
  | { phase: 'error'; message: string }

function readFileAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const result = reader.result as string
      const comma = result.indexOf(',')
      resolve(comma >= 0 ? result.slice(comma + 1) : result)
    }
    reader.onerror = () => reject(reader.error ?? new Error('could not read the file'))
    reader.readAsDataURL(file)
  })
}

export function TranscribeSection({ host, focusSignal }: { host: Host; focusSignal: number }) {
  const [path, setPath] = useState('')
  const [file, setFile] = useState<{ name: string; size: number; base64: string } | null>(null)
  const [fileError, setFileError] = useState<string | null>(null)
  const [reading, setReading] = useState(false)
  const [dragging, setDragging] = useState(false)
  const [result, setResult] = useState<Result>({ phase: 'idle' })
  const fileInputRef = useRef<HTMLInputElement | null>(null)
  const dropRef = useRef<HTMLButtonElement | null>(null)
  const readGenerationRef = useRef(0)
  const { state: copyState, copy } = useCopyFlash(result.phase === 'ready' ? result.data.text : '', 2000)

  useEffect(() => {
    if (focusSignal > 0) dropRef.current?.focus()
  }, [focusSignal])

  const acceptFile = (candidate: File | undefined) => {
    if (!candidate) return
    readGenerationRef.current += 1
    const generation = readGenerationRef.current
    if (candidate.size > MAX_BYTES) {
      setFileError(
        `${candidate.name} is ${formatBytes(candidate.size)}; the inline limit is 10 MiB. Pass a path instead.`,
      )
      setFile(null)
      setReading(false)
      return
    }
    setFileError(null)
    setFile(null)
    setReading(true)
    readFileAsBase64(candidate)
      .then((base64) => {
        if (generation !== readGenerationRef.current) return
        setFile({ name: candidate.name, size: candidate.size, base64 })
        setReading(false)
      })
      .catch((err: unknown) => {
        if (generation !== readGenerationRef.current) return
        setFileError(errorMessage(err))
        setReading(false)
      })
  }

  const onInput = (event: ChangeEvent<HTMLInputElement>) => {
    acceptFile(event.target.files?.[0])
    event.target.value = ''
  }

  const onDrop = (event: DragEvent<HTMLButtonElement>) => {
    event.preventDefault()
    setDragging(false)
    acceptFile(event.dataTransfer.files?.[0])
  }

  const run = () => {
    if (reading) return
    if (!file && !path.trim()) {
      setFileError('Choose a WAV file or enter a path.')
      return
    }
    setFileError(null)
    setResult({ phase: 'loading' })
    const source = file ? file.name : path.trim()
    transcribe(host.iii, file ? { audio_base64: file.base64 } : { path: path.trim() })
      .then((data) => setResult({ phase: 'ready', data, source }))
      .catch((err: unknown) => setResult({ phase: 'error', message: errorMessage(err) }))
  }

  const sendToChat = () => {
    if (result.phase === 'ready') host.chat?.compose?.({ text: `${result.data.text} ` })
  }

  return (
    <>
      <SectionCard title="Audio" description="A WAV recording, from this browser or a path on the worker's machine.">
        <PanelBody>
          <input
            ref={fileInputRef}
            type="file"
            accept=".wav,audio/wav"
            onChange={onInput}
            className="voice-file-input"
            aria-label="choose a WAV file"
            tabIndex={-1}
          />
          <div className="voice-drop-row">
            <button
              ref={dropRef}
              type="button"
              className={dragging ? 'voice-dropzone dragging' : 'voice-dropzone'}
              onClick={() => fileInputRef.current?.click()}
              onDragOver={(event) => {
                event.preventDefault()
                setDragging(true)
              }}
              onDragLeave={() => setDragging(false)}
              onDrop={onDrop}
              aria-label="choose or drop a WAV file"
            >
              {file ? (
                <span className="voice-dropzone-file">
                  <FileMusic size={16} className="voice-dropzone-icon" />
                  <span className="voice-dropzone-name">{file.name}</span>
                  <span className="voice-row-hint">{formatBytes(file.size)}</span>
                </span>
              ) : (
                <span className="voice-dropzone-file">
                  <Upload size={16} className="voice-dropzone-icon" />
                  <span className="voice-dropzone-copy">
                    <span className="voice-row-label">Drop a WAV file or click to choose</span>
                    <span className="voice-row-hint">Any sample rate or channel count, up to 10 MiB inline</span>
                  </span>
                </span>
              )}
            </button>
            {file ? (
              <IconButton
                label="Remove file"
                variant="ghost"
                onClick={() => {
                  readGenerationRef.current += 1
                  setFile(null)
                  setFileError(null)
                  setReading(false)
                }}
              >
                <Trash2 />
              </IconButton>
            ) : null}
          </div>
          <div className="voice-field">
            <span className="voice-row-label">Or a path on the worker's machine</span>
            <span className="voice-field-line">
              <ShadcnInput
                value={path}
                onChange={(event) => setPath(event.target.value)}
                placeholder="/recordings/meeting.wav"
                aria-label="audio file path"
              />
              <ShadcnButton onClick={run} disabled={result.phase === 'loading' || reading}>
                {result.phase === 'loading' ? 'Transcribing…' : reading ? 'Reading file…' : 'Transcribe'}
              </ShadcnButton>
            </span>
          </div>
          {fileError ? <StatusPanel variant="warn" headline="Cannot use that input" detail={fileError} /> : null}
        </PanelBody>
      </SectionCard>

      {result.phase === 'error' ? (
        <StatusPanel variant="alert" headline="Transcription failed" detail={result.message} />
      ) : null}

      {result.phase === 'ready' ? (
        <SectionCard
          title="Transcript"
          description={
            <>
              {result.data.backend === 'local' ? result.data.model : 'OpenAI-compatible'} ·{' '}
              {formatDuration(result.data.duration_secs)} · {result.source}
            </>
          }
          actions={
            <>
              {copyState === 'copied' ? <StatusBadge tone="ok">Copied</StatusBadge> : null}
              <IconButton label="Copy transcript" variant="ghost" onClick={copy}>
                <Copy />
              </IconButton>
              {host.chat?.compose ? (
                <ShadcnButton variant="secondary" size="sm" onClick={sendToChat}>
                  <Send />
                  Send to chat
                </ShadcnButton>
              ) : null}
            </>
          }
        >
          <PanelBody>
            <p className="voice-transcript-text">{result.data.text || 'The recognizer heard no speech.'}</p>
          </PanelBody>
          {result.data.segments.length > 1 ? (
            <ShadcnTable className="voice-table">
              <ShadcnTableHeader>
                <ShadcnTableRow>
                  <ShadcnTableHead>#</ShadcnTableHead>
                  <ShadcnTableHead>Start</ShadcnTableHead>
                  <ShadcnTableHead>End</ShadcnTableHead>
                  <ShadcnTableHead>Text</ShadcnTableHead>
                </ShadcnTableRow>
              </ShadcnTableHeader>
              <ShadcnTableBody>
                {result.data.segments.map((segment) => (
                  <ShadcnTableRow key={segment.segment}>
                    <ShadcnTableCell>{segment.segment + 1}</ShadcnTableCell>
                    <ShadcnTableCell className="voice-mono">{formatSeconds(segment.start_secs)}</ShadcnTableCell>
                    <ShadcnTableCell className="voice-mono">{formatSeconds(segment.end_secs)}</ShadcnTableCell>
                    <ShadcnTableCell className="voice-wrap">{segment.text}</ShadcnTableCell>
                  </ShadcnTableRow>
                ))}
              </ShadcnTableBody>
            </ShadcnTable>
          ) : null}
        </SectionCard>
      ) : null}
    </>
  )
}
