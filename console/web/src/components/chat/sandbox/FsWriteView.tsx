import { formatBytes } from './format'
import {
  fsWriteRequestSchema,
  fsWriteResponseSchema,
  safeParseResponse,
  streamChannelRefSchema,
} from './parsers'
import { Chip, FooterPill } from './terminal/Terminal'

interface FsWriteViewProps {
  input: unknown
  output: unknown
}

export function FsWriteView({ input, output }: FsWriteViewProps) {
  const req = fsWriteRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsWriteResponseSchema, output)
  if (!resp) return null
  const streamed = req.data.content
    ? streamChannelRefSchema.safeParse(req.data.content).success
    : false
  const usedB64 = !!req.data.content_b64

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="px-3 py-3 flex flex-col gap-2">
        <div className="font-mono text-[12.5px] text-ink">
          <span className="text-accent">+ wrote</span>{' '}
          <span className="tabular-nums">
            {formatBytes(resp.bytes_written)}
          </span>{' '}
          <span className="text-ink-faint">to</span> <span>{resp.path}</span>
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          <Chip label="mode">{req.data.mode ?? '0644'}</Chip>
          {req.data.parents ? <Chip label="parents">true</Chip> : null}
          {streamed ? (
            <FooterPill tone="default">uploaded via channel</FooterPill>
          ) : null}
          {usedB64 ? (
            <FooterPill tone="default">base64 inline</FooterPill>
          ) : null}
        </div>
      </div>
    </div>
  )
}
