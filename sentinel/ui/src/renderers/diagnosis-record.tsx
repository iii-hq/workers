import { Badge, Chip, MetaRow } from '@iii-dev/console-ui'
import type { FunctionTriggerMessage, FunctionTriggerRenderer, Host } from '@iii-dev/console-ui'
import { FN } from '../shared'

interface RecordInput {
  group_id?: string
  diagnosis?: {
    summary?: string
    category?: string
    confidence?: string
  }
}

/**
 * How a recorded diagnosis reads in the transcript.
 *
 * The call is visible there like any other `triggered ƒ`, and unrendered it
 * is a wall of JSON in the middle of a conversation somebody is following.
 * This turns it into the one line that matters plus its confidence.
 */
export function diagnosisRecordRenderer(_host: Host): FunctionTriggerRenderer {
  const render = (message: FunctionTriggerMessage) => {
    const input = (message.input ?? {}) as RecordInput
    const diagnosis = input.diagnosis ?? {}
    if (!diagnosis.summary) return null
    return (
      <div className="sentinel-ui-record">
        <MetaRow
          items={[
            { label: 'group', value: (input.group_id ?? '').slice(0, 16) },
            { label: 'category', value: diagnosis.category ?? 'unknown' },
          ]}
        >
          <Badge variant={diagnosis.confidence === 'high' ? 'ok' : 'default'}>
            {diagnosis.confidence ?? 'unknown'} confidence
          </Badge>
          <Chip tone="accent">diagnosis recorded</Chip>
        </MetaRow>
        <p className="sentinel-ui-record-summary">{diagnosis.summary}</p>
      </div>
    )
  }

  return {
    id: 'sentinel/page.js#diagnosis-record',
    isMatch: (functionId) => functionId === FN.record,
    tryRender: render,
    // Worth seeing in the feed without opening the call: this is the moment
    // the investigation reached a conclusion.
    tryRenderDisplay: render,
    metadata: { display: true, displayAction: 'expand' },
  }
}
