import {
  Badge,
  Button,
  Card,
  CardBody,
  CardHeader,
  Chip,
  CodeHighlight,
  EmptyState,
  Eyebrow,
  Markdown,
  MetaRow,
} from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import type { Host } from '@iii-dev/console-ui'
import { FileCode } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { Client, DiagnosisRecord, Investigation } from '../api'
import { PreviousDiagnoses } from './history'
import { codeLocation } from './present.js'

interface Props {
  api: Client
  diagnosis?: DiagnosisRecord
  groupId: string
  host: Host
  investigation?: Investigation
  onAsk: () => void
  repositoryPath: string | null
}

/**
 * What the investigation concluded. The record is the agent's own words, so
 * it is shown as written — this page adds provenance and a way to open the
 * code it points at, and nothing else.
 */
export function DiagnosisCard({
  api,
  diagnosis,
  groupId,
  host,
  investigation,
  onAsk,
  repositoryPath,
}: Props) {
  const history = useHistory(api, groupId, diagnosis?.id)

  if (!diagnosis) {
    return (
      <EmptyState
        compact
        title="Nothing recorded yet"
        description={
          investigation
            ? 'The investigation is open. Ask it to write down what it has, or keep watching.'
            : 'Investigate this group and the agent records what it finds here.'
        }
        action={investigation ? { label: 'Ask for a diagnosis', onClick: onAsk } : undefined}
      />
    )
  }

  if (!diagnosis.valid) {
    return (
      <Card>
        <CardHeader>
          <Eyebrow>recorded, unparsable</Eyebrow>
        </CardHeader>
        <CardBody>
          <p>
            The stored payload no longer matches the diagnosis shape. It is kept verbatim rather
            than discarded.
          </p>
          <CodeHighlight code={diagnosis.raw_result ?? ''} language="json" wrap />
        </CardBody>
      </Card>
    )
  }

  const value = diagnosis.diagnosis
  if (!value) return null

  return (
    <Card>
      <CardHeader>
        <div className="sentinel-ui-diagnosis-head">
          <Badge variant={value.confidence === 'high' ? 'ok' : 'default'}>
            {value.confidence}
          </Badge>
          <Chip tone="neutral">{value.category}</Chip>
          <Eyebrow>confidence</Eyebrow>
          <Eyebrow>
            {diagnosis.source === 'first_pass' ? 'first pass' : 'conversation'} ·{' '}
            {formatRelative(diagnosis.created_ms)}
          </Eyebrow>
        </div>
      </CardHeader>
      <CardBody>
        <Markdown>{value.summary}</Markdown>

        <Eyebrow size="lg">root cause</Eyebrow>
        <Markdown>{value.root_cause.description}</Markdown>
        <ul className="sentinel-ui-evidence-list">
          {value.root_cause.evidence.map((item, index) => {
            const location = codeLocation(item, repositoryPath)
            return (
              <li key={index}>
                <div className="sentinel-ui-evidence-why">{item.why}</div>
                <CodeHighlight code={item.excerpt} language="text" wrap />
                {location ? (
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() =>
                      host.panels?.open({
                        pageId: 'ide',
                        context: { type: 'file', path: location.path, line: location.line },
                      })
                    }
                  >
                    <FileCode size={16} />
                    <span>
                      {item.path}:{item.line ?? 1}
                    </span>
                  </Button>
                ) : item.span_id ? (
                  <Eyebrow>span {item.span_id.slice(0, 10)}</Eyebrow>
                ) : null}
              </li>
            )
          })}
        </ul>

        {value.proposed_fix ? (
          <>
            <Eyebrow size="lg">proposed fix · {value.proposed_fix.risk} risk</Eyebrow>
            <Markdown>{value.proposed_fix.description}</Markdown>
            <ol className="sentinel-ui-steps">
              {value.proposed_fix.steps.map((step, index) => (
                <li key={index}>{step}</li>
              ))}
            </ol>
            {value.proposed_fix.files.length > 0 ? (
              <MetaRow items={[{ label: 'files', value: value.proposed_fix.files.join(', ') }]} />
            ) : null}
          </>
        ) : null}

        {value.missing_evidence && value.missing_evidence.length > 0 ? (
          <>
            <Eyebrow size="lg">what was missing</Eyebrow>
            <ul className="sentinel-ui-steps">
              {value.missing_evidence.map((item, index) => (
                <li key={index}>{item}</li>
              ))}
            </ul>
          </>
        ) : null}

        {value.version_note ? <p className="sentinel-ui-note">{value.version_note}</p> : null}

        <MetaRow
          items={[
            { label: 'model', value: diagnosis.model },
            { label: 'investigation', value: diagnosis.investigation_id.slice(0, 12) },
            ...(investigation?.checkout_ref
              ? [{ label: 'checkout', value: investigation.checkout_ref }]
              : []),
          ]}
        >
          <Button size="sm" variant="ghost" onClick={onAsk}>
            Ask for a new diagnosis
          </Button>
        </MetaRow>

        <PreviousDiagnoses records={history.records} total={history.total} />
      </CardBody>
    </Card>
  )
}

/**
 * Every reading of this group except the one on screen. Re-read whenever the
 * current one changes, which is how a new recording appears here without a
 * reload.
 */
function useHistory(api: Client, groupId: string, currentId: string | undefined) {
  const [state, setState] = useState<{ records: DiagnosisRecord[]; total: number }>({
    records: [],
    total: 0,
  })

  useEffect(() => {
    let live = true
    if (!currentId) {
      setState({ records: [], total: 0 })
      return
    }
    api
      .diagnoses(groupId)
      .then((response) => {
        if (!live) return
        setState({
          records: response.diagnoses.filter((record) => record.id !== currentId),
          total: response.total,
        })
      })
      .catch((cause) => {
        // The history is context, not the answer: losing it must not take the
        // diagnosis down with it.
        if (live) {
          setState({ records: [], total: 0 })
          console.warn('sentinel: could not read the diagnosis history', errorMessage(cause))
        }
      })
    return () => {
      live = false
    }
  }, [api, groupId, currentId])

  return state
}
