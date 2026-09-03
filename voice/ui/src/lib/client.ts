/**
 * Thin typed wrappers over `host.iii.trigger` for every `voice::*`
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
  SpeakRequest,
  SpeakResponse,
  SpeakStopRequest,
  SpeakStopResponse,
  TranscribeRequest,
  TranscribeResponse,
} from './types'

export function dictationStart(iii: ExtensionIii, req: DictationStartRequest): Promise<DictationStartResponse> {
  return iii.trigger<DictationStartResponse>('voice::dictation::start', req)
}

export function dictationPush(iii: ExtensionIii, req: DictationPushRequest): Promise<DictationPushResponse> {
  return iii.trigger<DictationPushResponse>('voice::dictation::push', req)
}

export function dictationStop(iii: ExtensionIii, req: DictationStopRequest): Promise<DictationStopResponse> {
  return iii.trigger<DictationStopResponse>('voice::dictation::stop', req)
}

export function dictationList(iii: ExtensionIii): Promise<DictationListResponse> {
  return iii.trigger<DictationListResponse>('voice::dictation::list', {})
}

export function transcribe(iii: ExtensionIii, req: TranscribeRequest): Promise<TranscribeResponse> {
  return iii.trigger<TranscribeResponse>('voice::transcribe', req)
}

export function speak(iii: ExtensionIii, req: SpeakRequest): Promise<SpeakResponse> {
  return iii.trigger<SpeakResponse>('voice::speak', req)
}

export function speakStop(iii: ExtensionIii, req: SpeakStopRequest = {}): Promise<SpeakStopResponse> {
  return iii.trigger<SpeakStopResponse>('voice::speak::stop', req)
}

export function modelsList(iii: ExtensionIii): Promise<ModelsListResponse> {
  return iii.trigger<ModelsListResponse>('voice::models::list', {})
}

export function modelsDownload(iii: ExtensionIii, req: ModelsDownloadRequest = {}): Promise<ModelsDownloadResponse> {
  return iii.trigger<ModelsDownloadResponse>('voice::models::download', req)
}

export function modelsRemove(iii: ExtensionIii, req: ModelsRemoveRequest): Promise<ModelsRemoveResponse> {
  return iii.trigger<ModelsRemoveResponse>('voice::models::remove', req)
}

export function doctor(iii: ExtensionIii): Promise<DoctorResponse> {
  return iii.trigger<DoctorResponse>('voice::doctor', {})
}
