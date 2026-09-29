import type { ExtensionIii } from '@iii-dev/console-ui'
import { unwrapEnvelope } from '@iii-dev/console-ui/format'

/**
 * Typed wrappers over the worker's own `ios-simulator::*` surface. Wire
 * source: `ios-simulator/src/functions.rs` (ids and payloads verbatim).
 * Every call carries the page's tenant.
 */

export const NS = 'ios-simulator::'
export const DEVICE_CHANGED = 'ios-simulator::device-changed'
export const MEDIA_CHANGED = 'ios-simulator::media-changed'
export const FRAME_EVENT = 'ios-simulator::frame-event'
export const ACTIVITY = 'ios-simulator::activity'
export const OPEN = 'ios-simulator::open'
export const DEFAULT_TENANT = 'default'

export interface Device {
  udid: string
  name: string
  state: string
  runtime: string
  device_type: string
  available: boolean
  streaming: boolean
  recording: boolean
}

export interface DeviceType {
  identifier: string
  name: string
  product_family: string
}

export interface Runtime {
  identifier: string
  name: string
  version: string
  device_types: DeviceType[]
}

export interface Media {
  name: string
  udid: string
  kind: 'screenshot' | 'recording'
  mime: string
  bytes: number
  created_ms: number
}

export interface FrameEvent {
  udid: string
  data: string
  width: number
  height: number
  device_width: number
  device_height: number
  seq: number
}

type Payload = Record<string, unknown>

export class SimApi {
  constructor(
    private readonly iii: ExtensionIii,
    readonly tenant: string,
  ) {}

  async call<T>(fn: string, payload: Payload = {}, timeoutMs?: number): Promise<T> {
    const res = await this.iii.trigger<unknown>(
      NS + fn,
      { ...payload, tenant: this.tenant },
      timeoutMs ? { timeoutMs } : undefined,
    )
    return unwrapEnvelope(res) as T
  }

  tenants = () => this.call<{ tenants: string[] }>('tenants::list').then((r) => r.tenants)
  devices = () => this.call<{ devices: Device[] }>('devices::list').then((r) => r.devices)
  runtimes = () => this.call<{ runtimes: Runtime[] }>('runtimes').then((r) => r.runtimes)
  create = (name: string, device_type: string, runtime?: string) =>
    this.call<{ device: Device }>('devices::create', { name, device_type, runtime }, 60_000)
  /** The page shows the phone itself: no live preview for this boot. */
  boot = (udid: string) => this.call('devices::boot', { udid, preview: false }, 180_000)
  shutdown = (udid: string) => this.call('devices::shutdown', { udid }, 120_000)
  erase = (udid: string) => this.call('devices::erase', { udid }, 180_000)
  remove = (udid: string) => this.call('devices::delete', { udid }, 120_000)
  screenshot = (udid: string) =>
    this.call<{ details: { media: Media } }>('screenshot', { udid, include_image: false }, 30_000)
  recordStart = (udid: string) => this.call<{ name: string }>('recording::start', { udid }, 30_000)
  recordStop = (udid: string) => this.call<{ media?: Media }>('recording::stop', { udid }, 60_000)
  media = (udid?: string) => this.call<{ media: Media[] }>('media::list', { udid }).then((r) => r.media)
  deleteMedia = (name: string) => this.call('media::delete', { name })
  watch = (udid: string) =>
    this.call<{ device_width: number; device_height: number; lease_ms: number }>('watch', { udid }, 60_000)
  frame = (udid: string) =>
    this.call<{ frame?: string; width: number; height: number; device_width: number; device_height: number; seq: number }>(
      'frame',
      { udid },
    )
  touch = (udid: string, t: TouchPoint) => this.call('touch', { udid, ...t })
  button = (udid: string, button: string, phase: 'press' | 'down' | 'up' = 'press') =>
    this.call('button', { udid, button, phase })
  type = (udid: string, text: string) => this.call('type', { udid, text })
  key = (udid: string, keys: string[]) => this.call('key', { udid, keys })

  /** A whole media file, read in 8 MiB slices, as a blob URL. */
  async mediaUrl(item: Media): Promise<string> {
    const parts: BlobPart[] = []
    let offset = 0
    for (;;) {
      const slice = await this.call<{ data: string; eof: boolean }>('media::read', { name: item.name, offset }, 60_000)
      const bytes = Uint8Array.from(atob(slice.data), (c) => c.charCodeAt(0))
      parts.push(bytes)
      offset += bytes.length
      if (slice.eof || bytes.length === 0) break
    }
    return URL.createObjectURL(new Blob(parts, { type: item.mime }))
  }
}

export interface TouchPoint {
  phase: 'down' | 'move' | 'up'
  x: number
  y: number
  x2?: number
  y2?: number
}

/** An `ios-simulator::frame-event` payload, or null for anything else. */
export function frameEvent(raw: unknown): FrameEvent | null {
  if (!raw || typeof raw !== 'object') return null
  const f = raw as Partial<FrameEvent>
  if (typeof f.udid !== 'string' || typeof f.data !== 'string' || typeof f.seq !== 'number') return null
  return f as FrameEvent
}

export function isBooted(d: Pick<Device, 'state'>) {
  return d.state === 'Booted'
}

/** `com.apple.CoreSimulator.SimRuntime.iOS-26-5` → `iOS 26.5`. */
export function runtimeLabel(runtime: string) {
  const tail = runtime.split('.').pop() ?? runtime
  const [os, ...version] = tail.split('-')
  return version.length ? `${os} ${version.join('.')}` : tail
}
