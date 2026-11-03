import {
  Badge,
  MetaRow,
  Table,
  TableBody,
  TableCell,
  TableFooter,
  TableHead,
  TableHeader,
  TableRow,
} from '@iii-dev/console-ui'
import { formatBytes } from '@iii-dev/console-ui/format'
import {
  contentRefSchema,
  type FsWriteRequest,
  type FsWriteResponse,
  fsWriteRequestSchema,
  fsWriteResponseSchema,
  safeParseResponse,
} from './parsers'
import { displayPath, items, kv, targetItem } from './shared'

interface FsWriteViewProps {
  input: unknown
  output: unknown
}

export function FsWriteView({ input, output }: FsWriteViewProps) {
  const req = fsWriteRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsWriteResponseSchema, output)
  if (!resp) return null

  /* Request-side discriminator: batch when `files` is a non-empty array.
     (The response-side `path === ''` heuristic is unreliable —
     sandbox-target single writes blank it too.) */
  if (req.data.files?.length) {
    return <BatchWrite req={req.data} resp={resp} />
  }

  const streamed =
    req.data.content != null &&
    contentRefSchema.safeParse(req.data.content).success

  return (
    <div className="shui-card">
      <MetaRow
        items={items(
          kv('mode', req.data.mode ?? '0644'),
          req.data.parents ? kv('parents', 'true') : null,
          targetItem(req.data.target),
        )}
      >
        {streamed ? <Badge>uploaded via channel</Badge> : null}
      </MetaRow>
      <div className="shui-card-body shui-line">
        <span className="t-accent">+ wrote</span>{' '}
        <span className="num">{formatBytes(resp.bytes_written)}</span>{' '}
        <span className="t-faint">to</span>{' '}
        <span className="shui-path">{displayPath(req.data.path ?? '', resp.path)}</span>
      </div>
    </div>
  )
}

interface BatchWriteProps {
  req: FsWriteRequest
  resp: FsWriteResponse
}

/** Batch layout — one row per written file. `resp.files` is authoritative
    (worker preserves order); each row joins back to its request spec by
    path (index fallback) for mode/content provenance. */
function BatchWrite({ req, resp }: BatchWriteProps) {
  const specs = req.files ?? []
  const specByPath = new Map(specs.map((s) => [s.path, s] as const))

  return (
    <div className="shui-card">
      <MetaRow items={items(kv('files', resp.files.length), targetItem(req.target))} />
      <Table density="compact">
        <TableHeader>
          <TableRow>
            <TableHead>path</TableHead>
            <TableHead className="r">bytes</TableHead>
            <TableHead>mode</TableHead>
            <TableHead>content</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {resp.files.map((r, i) => {
            const spec = specByPath.get(r.path) ?? specs[i]
            return (
              <TableRow key={r.path}>
                <TableCell className="shui-path t-ink">{r.path}</TableCell>
                <TableCell className="t-faint num r">{formatBytes(r.bytes_written)}</TableCell>
                <TableCell className="t-faint">
                  {spec ? (spec.mode ?? '0644') : '—'}
                  {spec?.parents ? <span className="t-ghost"> +parents</span> : null}
                </TableCell>
                <TableCell className="t-faint">
                  {spec ? (typeof spec.content === 'string' ? 'inline' : 'channel') : '—'}
                </TableCell>
              </TableRow>
            )
          })}
        </TableBody>
        <TableFooter>
          <TableRow>
            <TableCell className="t-faint">total</TableCell>
            <TableCell colSpan={3}>
              <Badge variant="accent">
                {`${formatBytes(resp.bytes_written)} · ${resp.files.length} ${resp.files.length === 1 ? 'file' : 'files'}`}
              </Badge>
            </TableCell>
          </TableRow>
        </TableFooter>
      </Table>
    </div>
  )
}
