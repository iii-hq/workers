/**
 * Thin typed wrappers over `host.iii.call` for every `voice::*`
 * function. Each wrapper takes the tab's `ExtensionIii` client directly
 * (not the whole `Host`), so callers stay decoupled from the console
 * runtime and every wrapper is trivially testable with a fake.
 */

import type { ExtensionIii } from '@iii-dev/console-ui'
import type {
  DictationListResponse,
  DictationPushRequest,
  DictationPushResponse,
  DictationStartRequest,
  DictationStartResponse,
  DictationStopRequest,
  DictationStopResponse,
  DoctorResponse,
  ModelsDownloadRequest,
  ModelsDownloadResponse,
  ModelsListResponse,
  ModelsRemoveRequest,
  ModelsRemoveResponse,
  RouterModelsListResponse,
  SpeakRequest,
  SpeakResponse,
  SpeakStopRequest,
  SpeakStopResponse,
  TranscribeRequest,
  TranscribeResponse,
} from './types'

const TRANSCRIBE_TIMEOUT_MS = 10 * 60 * 1000
const DOWNLOAD_TIMEOUT_MS = 60 * 60 * 1000

export function dictationStart(iii: ExtensionIii, req: DictationStartRequest): Promise<DictationStartResponse> {
  return iii.call<DictationStartResponse>('voice::dictation::start', req)
}

export function dictationPush(iii: ExtensionIii, req: DictationPushRequest): Promise<DictationPushResponse> {
  return iii.call<DictationPushResponse>('voice::dictation::push', req)
}

export function dictationStop(iii: ExtensionIii, req: DictationStopRequest): Promise<DictationStopResponse> {
  return iii.call<DictationStopResponse>('voice::dictation::stop', req)
}

export function dictationList(iii: ExtensionIii): Promise<DictationListResponse> {
  return iii.call<DictationListResponse>('voice::dictation::list', {})
}

export function transcribe(iii: ExtensionIii, req: TranscribeRequest): Promise<TranscribeResponse> {
  return iii.call<TranscribeResponse>('voice::transcribe', req, { timeoutMs: TRANSCRIBE_TIMEOUT_MS })
}

export function speak(iii: ExtensionIii, req: SpeakRequest): Promise<SpeakResponse> {
  return iii.call<SpeakResponse>('voice::speak', req, { timeoutMs: 90_000 })
}

export function speakStop(iii: ExtensionIii, req: SpeakStopRequest = {}): Promise<SpeakStopResponse> {
  return iii.call<SpeakStopResponse>('voice::speak::stop', req)
}

export function modelsList(iii: ExtensionIii): Promise<ModelsListResponse> {
  return iii.call<ModelsListResponse>('voice::models::list', {})
}

export function modelsDownload(iii: ExtensionIii, req: ModelsDownloadRequest = {}): Promise<ModelsDownloadResponse> {
  return iii.call<ModelsDownloadResponse>('voice::models::download', req, { timeoutMs: DOWNLOAD_TIMEOUT_MS })
}

export function modelsRemove(iii: ExtensionIii, req: ModelsRemoveRequest): Promise<ModelsRemoveResponse> {
  return iii.call<ModelsRemoveResponse>('voice::models::remove', req)
}

export function doctor(iii: ExtensionIii): Promise<DoctorResponse> {
  return iii.call<DoctorResponse>('voice::doctor', {})
}

/** Speech models llm-router knows for one family; empty when no speech provider is registered. */
export function routerSpeechModels(iii: ExtensionIii, modality: 'stt' | 'tts'): Promise<RouterModelsListResponse> {
  return iii.call<RouterModelsListResponse>('router::models::list', { modality })
}
