/**
 * The Transcribe section: drop or choose a WAV file (read as base64, capped
 * at 10 MiB) or name a path on the worker's machine, run
 * `voice::transcribe`, and read the result with per-segment timestamps and
 * copy / send-to-chat actions.
 */

import {
  Button,
  Chip,
  type Host,
  IconButton,
  Input,
  StatusPanel,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@iii-dev/console-ui'
import type { ChangeEvent, DragEvent } from 'react'
import { useEffect, useRef, useState } from 'react'
import { transcribe } from '../lib/client'
import { errorMessage, formatSeconds } from '../lib/format'
import { CopyIcon, FileAudioIcon, SendIcon, TrashIcon, UploadIcon } from '../lib/icons'
import type { TranscribeResponse } from '../lib/types'
import { formatBytes, formatDuration, SectionCard } from './shared'

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
  const [dragging, setDragging] = useState(false)
  const [result, setResult] = useState<Result>({ phase: 'idle' })
  const [copied, setCopied] = useState(false)
  const fileInputRef = useRef<HTMLInputElement | null>(null)
  const dropRef = useRef<HTMLButtonElement | null>(null)

  useEffect(() => {
    if (focusSignal > 0) dropRef.current?.focus()
  }, [focusSignal])

  const acceptFile = (candidate: File | undefined) => {
    if (!candidate) return
    if (candidate.size > MAX_BYTES) {
      setFileError(
        `${candidate.name} is ${formatBytes(candidate.size)}; the inline limit is 10 MB. Pass a path instead.`,
      )
      setFile(null)
      return
    }
    setFileError(null)
    readFileAsBase64(candidate)
      .then((base64) => setFile({ name: candidate.name, size: candidate.size, base64 }))
      .catch((err: unknown) => setFileError(errorMessage(err)))
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

  const copyText = () => {
    if (result.phase !== 'ready') return
    navigator.clipboard
      .writeText(result.data.text)
      .then(() => {
        setCopied(true)
        window.setTimeout(() => setCopied(false), 2000)
      })
      .catch(() => {})
  }

  const sendToChat = () => {
    if (result.phase === 'ready') host.chat?.compose?.({ text: `${result.data.text} ` })
  }

  return (
    <>
      <SectionCard title="Audio">
        <input
          ref={fileInputRef}
          type="file"
          accept=".wav,audio/wav"
          onChange={onInput}
          className="voice-file-input"
          aria-label="choose a WAV file"
          tabIndex={-1}
        />
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
          <UploadIcon className="voice-dropzone-icon" />
          {file ? (
            <span className="voice-dropzone-file">
              <FileAudioIcon />
              <span className="voice-dropzone-name">{file.name}</span>
              <span className="voice-sub">{formatBytes(file.size)}</span>
            </span>
          ) : (
            <span className="voice-dropzone-copy">
              <span className="voice-strong">Drop a WAV file or click to choose</span>
              <span className="voice-sub">Any sample rate or channel count, up to 10 MB inline</span>
            </span>
          )}
        </button>
        {file ? (
          <div className="voice-inline-actions">
            <IconButton
              label="Remove file"
              variant="ghost"
              onClick={() => {
                setFile(null)
                setFileError(null)
              }}
            >
              <TrashIcon />
            </IconButton>
          </div>
        ) : null}
        <div className="voice-fields">
          <div className="voice-field">
            <span className="voice-field-label">Or a path on the worker's machine</span>
            <Input value={path} onChange={setPath} placeholder="/recordings/meeting.wav" aria-label="audio file path" />
          </div>
          <div className="voice-field-actions">
            <Button variant="primary" onClick={run} disabled={result.phase === 'loading'}>
              {result.phase === 'loading' ? 'transcribing…' : 'Transcribe'}
            </Button>
          </div>
        </div>
        {fileError ? <StatusPanel variant="warn" headline="Cannot use that input" detail={fileError} /> : null}
      </SectionCard>

      {result.phase === 'error' ? (
        <StatusPanel variant="alert" headline="Transcription failed" detail={result.message} />
      ) : null}

      {result.phase === 'ready' ? (
        <SectionCard
          title={
            <span className="voice-fact-line">
              <span>Transcript</span>
              <Chip tone="neutral">{result.data.backend === 'local' ? result.data.model : 'openai'}</Chip>
              <span className="voice-sub">
                {formatDuration(result.data.duration_secs)} · {result.source}
              </span>
            </span>
          }
          actions={
            <span className="voice-card-actions">
              {copied ? <Chip tone="success">copied</Chip> : null}
              <IconButton label="Copy transcript" variant="ghost" onClick={copyText}>
                <CopyIcon />
              </IconButton>
              {host.chat?.compose ? (
                <Button variant="primary" size="sm" onClick={sendToChat}>
                  <SendIcon />
                  Send to chat
                </Button>
              ) : null}
            </span>
          }
        >
          <p className="voice-transcript-text">{result.data.text || 'The recognizer heard no speech.'}</p>
          {result.data.segments.length > 1 ? (
            <TableViewport>
              <TableFrame>
                <Table density="compact">
                  <TableHeader>
                    <TableRow>
                      <TableHead>#</TableHead>
                      <TableHead>start</TableHead>
                      <TableHead>end</TableHead>
                      <TableHead>text</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {result.data.segments.map((segment) => (
                      <TableRow key={segment.segment}>
                        <TableCell>{segment.segment + 1}</TableCell>
                        <TableCell className="voice-mono">{formatSeconds(segment.start_secs)}</TableCell>
                        <TableCell className="voice-mono">{formatSeconds(segment.end_secs)}</TableCell>
                        <TableCell>{segment.text}</TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </TableFrame>
            </TableViewport>
          ) : null}
        </SectionCard>
      ) : null}
    </>
  )
}
