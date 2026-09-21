import { Badge, Chip, CollapsibleCard, CollapsibleCardContent, CollapsibleCardTrigger, Eyebrow, Markdown } from '@iii-dev/console-ui'
import { formatRelative } from '@iii-dev/console-ui/format'
import type { DiagnosisRecord } from '../api'

/**
 * The readings that came before the current one.
 *
 * Every call to `sentinel::diagnosis::record` is a version and none are
 * overwritten, which is most of the reason for keeping them: the same failure
 * diagnosed twice, before and after a release, is the comparison somebody
 * actually wants. The current one is rendered in full above; these are
 * collapsed, because a page that opened with five of them would bury it.
 */
export function PreviousDiagnoses({
  records,
  total,
}: {
  records: DiagnosisRecord[]
  total: number
}) {
  if (records.length === 0) return null
  return (
    <CollapsibleCard className="sentinel-ui-history">
      <CollapsibleCardTrigger>
        <Eyebrow>
          {records.length} earlier reading{records.length === 1 ? '' : 's'}
          {total > records.length + 1 ? ` of ${total}` : ''}
        </Eyebrow>
      </CollapsibleCardTrigger>
      <CollapsibleCardContent>
        <ol className="sentinel-ui-history-list">
          {records.map((record) => (
            <li key={record.id}>
              <div className="sentinel-ui-history-head">
                <Badge variant={record.diagnosis?.confidence === 'high' ? 'ok' : 'default'}>
                  {record.diagnosis?.confidence ?? 'unparsable'}
                </Badge>
                {record.diagnosis ? <Chip tone="neutral">{record.diagnosis.category}</Chip> : null}
                <Eyebrow>
                  {record.source === 'first_pass' ? 'first pass' : 'conversation'} ·{' '}
                  {formatRelative(record.created_ms)} · {record.model}
                </Eyebrow>
              </div>
              {record.diagnosis ? (
                <Markdown>{record.diagnosis.summary}</Markdown>
              ) : (
                <p className="sentinel-ui-note">
                  The stored payload no longer parses; it is kept verbatim rather than discarded.
                </p>
              )}
            </li>
          ))}
        </ol>
      </CollapsibleCardContent>
    </CollapsibleCard>
  )
}
