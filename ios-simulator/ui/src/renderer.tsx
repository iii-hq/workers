import {
  ActionLine,
  Badge,
  Chip,
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  MetaRow,
} from '@iii-dev/console-ui'
import { Smartphone } from 'lucide-react'
import { NS } from './lib/api'

/**
 * Chat/trace rendering for `ios-simulator::screenshot`, shaped like the
 * browser worker's screenshot card: "capturing…" while it runs, then the
 * preview the model saw. Every other call falls through to the console's
 * generic card.
 */

type Obj = Record<string, unknown>

const obj = (value: unknown): Obj | null =>
  value && typeof value === 'object' && !Array.isArray(value) ? (value as Obj) : null
const str = (value: unknown) => (typeof value === 'string' && value ? value : null)
const num = (value: unknown) => (typeof value === 'number' && value > 0 ? value : null)

/** The worker's result, bare or inside the harness's `{ content, details }` envelope. */
function shotOf(output: unknown) {
  const outer = obj(output)
  const inner = obj(outer?.details)
  const value = Array.isArray(inner?.content) ? inner : outer
  const blocks = (Array.isArray(value?.content) ? value.content : []).map(obj)
  const image = blocks.find((b) => b?.type === 'image' && str(b.data))
  if (!image) return null
  const details = obj(value?.details)
  const media = obj(details?.media)
  return {
    src: `data:${str(image.mime) ?? 'image/jpeg'};base64,${image.data}`,
    device: str(details?.device),
    width: num(details?.width),
    height: num(details?.height),
    name: str(media?.name),
    bytes: num(media?.bytes),
  }
}

function Target({ udid }: { udid: string }) {
  return (
    <ActionLine icon={<Smartphone size={16} aria-hidden />} tone="ink">
      <span className="ios-ui-break">{udid}</span>
    </ActionLine>
  )
}

function ScreenshotView({ message }: { message: FunctionTriggerMessage }) {
  const udid = str(obj(message.input)?.udid)
  if (!udid) return null

  if (message.running) {
    return (
      <div className="ios-ui-shot">
        <MetaRow>
          <Badge variant="default">capturing…</Badge>
        </MetaRow>
        <Target udid={udid} />
        <div className="ios-ui-shot-more">· waiting for the simulator…</div>
      </div>
    )
  }

  const shot = shotOf(message.output)
  if (!shot) return null
  return (
    <div className="ios-ui-shot">
      <MetaRow>
        <Badge variant="accent">screenshot</Badge>
        <Chip>png</Chip>
        {shot.width && shot.height ? (
          <Chip>
            <span className="ios-ui-num">
              {shot.width}×{shot.height}
            </span>
          </Chip>
        ) : null}
        {shot.bytes ? (
          <Chip>
            <span className="ios-ui-num">{Math.max(1, Math.round(shot.bytes / 1024))} KB</span>
          </Chip>
        ) : null}
      </MetaRow>
      <Target udid={shot.device ?? udid} />
      <div className="ios-ui-shot-gallery">
        <img src={shot.src} alt={`screenshot of ${shot.device ?? udid}`} loading="lazy" className="ios-ui-shot-img" />
        {shot.name ? <div className="ios-ui-shot-caption ios-ui-break">{shot.name}</div> : null}
      </div>
    </div>
  )
}

function ScreenshotPreview({ message }: { message: FunctionTriggerMessage }) {
  const udid = str(obj(message.input)?.udid)
  if (!udid) return null
  return (
    <div className="ios-ui-shot">
      <MetaRow>
        <Badge variant="warn">permission to screenshot</Badge>
      </MetaRow>
      <Target udid={udid} />
    </div>
  )
}

// Called as functions, not elements, so a result that isn't a screenshot
// (an error, a capped result) returns null and falls through.
const render = (message: FunctionTriggerMessage) => (message.pendingApproval ? null : ScreenshotView({ message }))

export const screenshotRenderer: FunctionTriggerRenderer = {
  id: 'ios-simulator/page.js#screenshot',
  isMatch: (functionId) => functionId === `${NS}screenshot`,
  tryRender: render,
  tryRenderRunning: render,
  tryRenderPreview: (message) => ScreenshotPreview({ message }),
  metadata: { display: true },
}
