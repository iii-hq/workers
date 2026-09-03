/**
 * The voice page's "Transcribe a file" panel: a `.wav` file picked through
 * a shared `Button` (the native input stays hidden; read as base64, capped
 * at 10 MiB) or a path text field, calling `voice::transcribe` and
 * rendering the text plus a segments table with copy and "send to chat"
 * actions. Inputs stack in narrow panes.
 */

import {
  Button,
  type Host,
  IconButton,
  Input,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@iii-dev/console-ui'
import type { ChangeEvent } from 'react'
import { useEffect, useRef, useState } from 'react'
import { transcribe } from '../lib/client'
import { errorMessage, formatSeconds } from '../lib/format'
import { CopyIcon, SendIcon } from '../lib/icons'
import type { Segment, TranscribeResponse } from '../lib/types'

const MAX_BYTES = 10 * 1024 * 1024

type Result =
  | { phase: 'idle' }
  | { phase: 'loading' }
  | { phase: 'ready'; data: TranscribeResponse }
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
  const [fileName, setFileName] = useState<string | null>(null)
  const [fileError, setFileError] = useState<string | null>(null)
  const [result, setResult] = useState<Result>({ phase: 'idle' })
  const audioBase64Ref = useRef<string | null>(null)
  const fileInputRef = useRef<HTMLInputElement | null>(null)
  const chooseRef = useRef<HTMLButtonElement | null>(null)

  useEffect(() => {
    if (focusSignal > 0) chooseRef.current?.focus()
  }, [focusSignal])

  const onFile = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0]
    if (!file) return
    if (file.size > MAX_BYTES) {
      setFileError(`file is ${(file.size / (1024 * 1024)).toFixed(1)} MiB; the limit is 10 MiB`)
      event.target.value = ''
      audioBase64Ref.current = null
      setFileName(null)
      return
    }
    setFileError(null)
    setFileName(file.name)
    readFileAsBase64(file)
      .then((b64) => {
        audioBase64Ref.current = b64
      })
      .catch((err: unknown) => setFileError(errorMessage(err)))
  }

  const clearFile = () => {
    audioBase64Ref.current = null
    setFileName(null)
    if (fileInputRef.current) fileInputRef.current.value = ''
  }

  const run = () => {
    const audioBase64 = audioBase64Ref.current
    if (!audioBase64 && !path.trim()) {
      setFileError('choose a .wav file or enter a path')
      return
    }
    setFileError(null)
    setResult({ phase: 'loading' })
    transcribe(host.iii, audioBase64 ? { audio_base64: audioBase64 } : { path: path.trim() })
      .then((data) => setResult({ phase: 'ready', data }))
      .catch((err: unknown) => setResult({ phase: 'error', message: errorMessage(err) }))
  }

  const copyText = () => {
    if (result.phase === 'ready') navigator.clipboard.writeText(result.data.text).catch(() => {})
  }

  const sendToChat = () => {
    if (result.phase === 'ready') host.chat?.compose?.({ text: `${result.data.text} ` })
  }

  return (
    <section className="voice-section">
      <h3 className="voice-section-title">Transcribe a file</h3>
      <div className="voice-transcribe-inputs">
        <input
          ref={fileInputRef}
          type="file"
          accept=".wav,audio/wav"
          onChange={onFile}
          className="voice-file-input"
          aria-label="choose a WAV file"
          tabIndex={-1}
        />
        <div className="voice-transcribe-file">
          <Button ref={chooseRef} variant="ghost" size="sm" onClick={() => fileInputRef.current?.click()}>
            Choose WAV file
          </Button>
          {fileName ? (
            <span className="voice-file-name" title={fileName}>
              {fileName}
              <button type="button" className="voice-file-clear" onClick={clearFile} aria-label="clear chosen file">
                ×
              </button>
            </span>
          ) : (
            <span className="voice-transcribe-or">or a path on the worker's machine</span>
          )}
        </div>
        <Input
          value={path}
          onChange={setPath}
          placeholder="/path/to/audio.wav"
          className="voice-path-input"
          aria-label="audio file path"
        />
        <Button variant="primary" size="sm" onClick={run} disabled={result.phase === 'loading'}>
          {result.phase === 'loading' ? 'transcribing…' : 'Transcribe'}
        </Button>
      </div>
      {fileError ? <div className="voice-note warn">{fileError}</div> : null}
      {result.phase === 'error' ? <div className="voice-note warn">{result.message}</div> : null}
      {result.phase === 'ready' ? (
        <div className="voice-transcribe-result">
          <div className="voice-transcribe-text-row">
            <p className="voice-transcribe-text">{result.data.text}</p>
            <div className="voice-transcribe-actions">
              <IconButton label="Copy transcript" onClick={copyText}>
                <CopyIcon />
              </IconButton>
              {host.chat?.compose ? (
                <IconButton label="Send to chat" onClick={sendToChat}>
                  <SendIcon />
                </IconButton>
              ) : null}
            </div>
          </div>
          {result.data.segments.length > 0 ? (
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
                    {result.data.segments.map((segment: Segment) => (
                      <TableRow key={segment.segment}>
                        <TableCell>{segment.segment}</TableCell>
                        <TableCell>{formatSeconds(segment.start_secs)}</TableCell>
                        <TableCell>{formatSeconds(segment.end_secs)}</TableCell>
                        <TableCell>{segment.text}</TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </TableFrame>
            </TableViewport>
          ) : null}
        </div>
      ) : null}
    </section>
  )
}
