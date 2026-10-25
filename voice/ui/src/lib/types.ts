/**
 * TypeScript types for the `voice` worker's wire contract: every
 * `voice::*` function's request/response shape and the trigger payloads a
 * page or chip may bind. Mirrors the Rust side exactly; kept
 * dependency-free so client.ts and every consumer can import it directly.
 */
import type { ExtensionIii } from '@iii-dev/console-ui'
import type { ComponentType } from 'react'

export type { ExtensionIii }

export interface Segment {
  segment: number
  text: string
  start_secs?: number
  end_secs?: number
}

/** A `type` alias (not `interface`) so it structurally satisfies the
    `Record<string, unknown>` payload parameter of `host.iii.trigger`
    without a cast — TS gives object-literal-shaped `type`s an implicit
    index signature, but not declared `interface`s. */
export type DictationStartRequest = {
  output_function_id: string
  sample_rate?: number
}

export interface DictationStartResponse {
  session_id: string
  model: string
  sample_rate: number
}

export type DictationPushRequest = {
  session_id: string
  seq: number
  pcm16_base64: string
}

export interface DictationPushResponse {
  accepted: boolean
  seq: number
  queued_ms: number
}

export type DictationStopRequest = {
  session_id: string
  discard?: boolean
}

export interface DictationStopResponse {
  session_id: string
  /** The transcript, cleaned up by the configured language model when that pass is on. */
  text: string
  segments: Segment[]
  duration_secs: number
  /** The recognizer's words before the clean-up pass changed them. */
  raw_text?: string
  /** The `provider::model` that cleaned the text up. */
  cleanup_model?: string
}

export interface DictationListEntry {
  session_id: string
  model: string
  started_at_ms: number
  duration_secs: number
  segments: number
  idle_secs: number
}

export interface DictationListResponse {
  sessions: DictationListEntry[]
}

export type TranscribeRequest = {
  path?: string
  audio_base64?: string
  language?: string
}

export interface TranscribeResponse {
  text: string
  segments: Segment[]
  duration_secs: number
  model: string
  backend: 'local' | 'whisper_cpp' | 'openai' | 'router'
}

export type SpeakRequest = {
  text: string
  /** Markdown prose by default; plain preserves already-rendered selections. */
  text_format?: 'markdown' | 'plain'
  voice?: string
  rate_wpm?: number
}

/** Every backend returns audio to the caller; speech_id identifies the synthesis result.
 * Playback and Stop are local to the requesting browser. */
export interface SpeakResponse {
  backend: 'host' | 'piper' | 'openai' | 'router'
  speech_id: string
  played: boolean
  audio_base64?: string
  mime?: string
}

export type SpeakStopRequest = {
  speech_id?: string
}

export interface SpeakStopResponse {
  stopped: number
}

export interface ModelInfo {
  id: string
  name: string
  kind?: 'streaming_transducer' | 'offline_nemo_transducer' | 'whisper_ggml' | 'piper_onnx'
  languages: string[]
  license?: string
  author?: string
  source?: string
  size_bytes: number
  installed: boolean
}

export interface ModelsListResponse {
  active: string
  models_dir: string
  models: ModelInfo[]
}

export type ModelsDownloadRequest = {
  id?: string
}

export interface ModelsDownloadResponse {
  id: string
  installed: true
  bytes: number
}

export type ModelsRemoveRequest = {
  id: string
}

export interface ModelsRemoveResponse {
  id: string
  removed: boolean
}

export interface DoctorResponse {
  stt: {
    backend: 'local' | 'whisper_cpp' | 'openai' | 'router'
    model: string
    installed: boolean
    loaded: boolean
    live_model: string
    live_installed: boolean
    live_loaded: boolean
    load_ms?: number
    models_dir: string
    problem?: string
    final_model: string
    final_state: 'off' | 'missing' | 'downloading' | 'installed' | 'loaded' | 'unknown'
    final_load_ms?: number
  }
  tts: {
    backend: 'host' | 'piper' | 'openai' | 'router' | 'off'
    command?: string
    /** Requested Piper policy, not a report of actual GPU availability. */
    device?: 'auto' | 'cpu'
    model?: string
    problem?: string | null
    available: boolean
    /** Legacy server playback count, now zero; browsers own playback. */
    playing: number
  }
  /** The clean-up pass over finished dictations; absent on workers before 0.2. */
  cleanup?: {
    enabled: boolean
    model: string
    problem?: string
  }
  /** Whether Read aloud / Voice chat show above the composer; absent (shown) on workers before 0.2. */
  chat_controls?: boolean
  sessions: number
  version: string
}

export type TranscriptEventKind = 'partial' | 'final' | 'closed' | 'error'

export interface TranscriptEvent {
  session_id: string
  seq: number
  kind: TranscriptEventKind
  text: string
  segment: number
  timestamp_ms: number
  reason?: string
}

export interface SessionStartedEvent {
  session_id: string
  timestamp_ms: number
}

export interface SessionStoppedEvent {
  session_id: string
  reason: string
  timestamp_ms: number
}

/** Legacy event retained for compatibility; browser playback uses local audio events. */
export interface SpeechEndedEvent {
  speech_id: string
  reason: 'ended' | 'stopped' | 'failed' | string
  timestamp_ms: number
}

export interface ModelProgressEvent {
  id: string
  file: string
  received_bytes: number
  total_bytes: number
  done: boolean
  error?: string
}

/* ── session-manager shapes used only to read the last assistant turn ──── */

export interface SessionContentBlock {
  type: string
  text?: string
}

export interface SessionMessageEntry {
  entry_id: string
  origin?: { turn_id?: string }
  elided?: boolean
  message?: {
    role?: string
    content?: SessionContentBlock[]
  }
}

export interface SessionMessagesResponse {
  messages: SessionMessageEntry[]
  next_cursor?: string
}

/** Props a composer toolbar action receives from consoles that ship the slot. */
export interface ComposerActionProps {
  sessionId: string | null
  isStreaming: boolean
}

/** A key in the console's shortcut syntax, one chord or one per platform. */
export type ComposerShortcut = string | { mac: readonly string[]; other: readonly string[] }

/** The palette row and key a console may give a composer action (ADE 2026-10 and later). */
export interface ComposerActionCommand {
  title: string
  detail?: string
  keywords?: readonly string[]
  shortcut?: ComposerShortcut
  firesWhileTyping?: boolean
  enabled?: () => boolean
  run: () => void
}

export interface ComposerActionRegistration {
  id: string
  render: ComponentType<ComposerActionProps>
  command?: ComposerActionCommand
}

/** `host.chat` on a console that ships the composer toolbar slot; feature-detect the method. */
export interface ComposerCapableChat {
  registerComposerAction?(action: ComposerActionRegistration): () => void
}

/* ── llm-router speech models, as `router::models::list` returns them ─── */

export interface RouterSpeechModel {
  id: string
  provider: string
  display_name?: string
  speech?: {
    modality: 'stt' | 'tts'
    languages?: string[]
    streaming?: boolean
  }
}

export interface RouterModelsListResponse {
  models: RouterSpeechModel[]
}
