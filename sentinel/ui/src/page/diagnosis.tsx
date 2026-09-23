import {
  Button,
  Card,
  CardBody,
  CardHeader,
  Chip,
  CodeHighlight,
  EmptyState,
  Markdown,
  Panel,
  StatusPanel,
  uiClasses,
} from '@iii-dev/console-ui'
import type { ChipTone, Host } from '@iii-dev/console-ui'
import { MessageSquare, ScanSearch } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { Diagnosis, DiagnosisRecord, GroupSummary, Investigation } from '../api'
import { Dot } from './marks'
import { ago, codeLocation, versionRange } from './present.js'

interface Props {
  group: GroupSummary
  host: Host
  now: number
  /** Every recording, newest first. */
  records: DiagnosisRecord[]
  investigation?: Investigation
  running: boolean
  sessionInView: boolean
  onAsk?: () => void
  /** Present when the group can be investigated: the empty tab's next step. */
  onInvestigate?: () => void
  onOpenSession?: () => void
  repositoryPath: string | null
}

const CONFIDENCE_TONE: Record<Diagnosis['confidence'], ChipTone> = {
  high: 'success',
  medium: 'warning',
  low: 'neutral',
}

function categoryTone(category: Diagnosis['category']): ChipTone {
  if (category === 'bug') return 'danger'
  if (category === 'unknown') return 'neutral'
  return 'warning'
}

/**
 * What the investigation concluded. The record is the agent's own words, so
 * it is shown as written — this page adds provenance, the versions that came
 * before, and a way to open the code it points at.
 */
export function DiagnosisTab({
  group,
  host,
  now,
  records,
  investigation,
  running,
  sessionInView,
  onAsk,
  onInvestigate,
  onOpenSession,
  repositoryPath,
}: Props) {
  const [shownId, setShownId] = useState<string | null>(null)
  useEffect(() => setShownId(null), [records[0]?.id])
  const shownIndex = Math.max(0, records.findIndex((record) => record.id === shownId))
  const shown = records[shownIndex]
  const version = (index: number) => records.length - index

  return (
    <div className="sentinel-ui-stack">
      {running && investigation ? (
        <StatusPanel
          variant="info"
          icon={<Dot tone="accent" pulse />}
          headline={`First pass running ${sessionInView ? 'in the session beside' : 'in the background — the session column is closed'}`}
          detail={`${investigation.model} · started ${ago(investigation.created_ms, now)}. When the agent has a probable cause it records it with sentinel::diagnosis::record — the cards appear here the moment it does. Type in the session at any time to steer it.`}
          action={
            !sessionInView && onOpenSession ? (
              <Button size="sm" variant="pill" onClick={onOpenSession}>
                <MessageSquare size={16} />
                Open session
              </Button>
            ) : undefined
          }
        />
      ) : null}

      {!shown && !running ? (
        <EmptyState
          icon={ScanSearch}
          title="No diagnosis yet"
          description="Investigate opens a harness session beside this page with the frozen evidence and read-only access to the mapped repository. You watch it work and can steer it; when it has a probable cause it records it with sentinel::diagnosis::record, and the cards land here."
          // The way forward from here: ask the session that exists, or start one.
          action={
            onAsk
              ? { label: 'Ask for a diagnosis', onClick: onAsk }
              : onInvestigate
                ? { label: 'Investigate', onClick: onInvestigate }
                : undefined
          }
        />
      ) : null}

      {shown ? (
        <>
          <Panel className="sentinel-ui-provenance">
            <div className="sentinel-ui-provenance-copy">
              <div className="sentinel-ui-provenance-line">
                <Dot tone={shownIndex === 0 ? 'ok' : 'ghost'} />
                <b>
                  Diagnosis v{version(shownIndex)} · recorded by the agent {ago(shown.created_ms, now)}
                  {shown.source === 'first_pass' ? ', first pass' : ' in the conversation'}
                </b>
                <span className="sentinel-ui-quiet-mono">{shown.model}</span>
              </div>
              <div className="sentinel-ui-provenance-line sentinel-ui-quiet">
                {onOpenSession ? (
                  <button type="button" className="sentinel-ui-link" onClick={onOpenSession}>
                    <MessageSquare size={16} aria-hidden="true" />
                    Sentinel: {group.exception_type ?? group.service_name}
                  </button>
                ) : null}
                <span className="sentinel-ui-mono">
                  investigated {investigation?.investigated_version ?? versionRange(group.first_version, group.last_version)}
                </span>
                <span className="sentinel-ui-mono">
                  {investigation?.checkout_ref ? `checkout ${investigation.checkout_ref}` : 'no checkout — evidence only'}
                </span>
                <span className="sentinel-ui-mono sentinel-ui-accent">via sentinel::diagnosis::record</span>
              </div>
            </div>
            {onAsk ? (
              <Button size="sm" variant="pill" onClick={onAsk}>
                <MessageSquare size={16} />
                Ask for a diagnosis
              </Button>
            ) : null}
          </Panel>

          {shown.valid && shown.diagnosis ? (
            <Reading diagnosis={shown.diagnosis} host={host} repositoryPath={repositoryPath} />
          ) : (
            <Card>
              <CardHeader>Recorded, unparsable</CardHeader>
              <CardBody>
                <p className="sentinel-ui-prose">
                  The stored payload no longer matches the diagnosis shape. It is kept verbatim rather than
                  discarded.
                </p>
                <CodeHighlight code={shown.raw_result ?? ''} language="json" wrap />
              </CardBody>
            </Card>
          )}

          <Card>
            <CardHeader>
              <span>Diagnoses</span>
              <span className="sentinel-ui-card-note">every recording is a version; none are overwritten</span>
            </CardHeader>
            <CardBody>
              <ol className="sentinel-ui-versions">
                {records.map((record, index) => (
                  <li key={record.id}>
                    <Dot tone={index === 0 ? 'ok' : 'ghost'} />
                    <span className="sentinel-ui-quiet-mono sentinel-ui-versions-when">{ago(record.created_ms, now)}</span>
                    <span>
                      v{version(index)} · {record.source === 'first_pass' ? 'first pass' : 'conversation'}
                    </span>
                    {record.diagnosis ? (
                      <Chip tone={CONFIDENCE_TONE[record.diagnosis.confidence]}>
                        confidence {record.diagnosis.confidence}
                      </Chip>
                    ) : (
                      <Chip>unparsable</Chip>
                    )}
                    <span className="sentinel-ui-quiet sentinel-ui-versions-note">
                      {record.diagnosis?.missing_evidence?.[0] ?? 'no open questions'}
                    </span>
                    {record.id === shown.id ? (
                      <span className="sentinel-ui-accent sentinel-ui-versions-tag">
                        {index === 0 ? 'current' : 'shown'}
                      </span>
                    ) : (
                      <button type="button" className="sentinel-ui-link sentinel-ui-versions-tag" onClick={() => setShownId(record.id)}>
                        view
                      </button>
                    )}
                  </li>
                ))}
              </ol>
            </CardBody>
          </Card>
        </>
      ) : null}
    </div>
  )
}

function Reading({
  diagnosis,
  host,
  repositoryPath,
}: {
  diagnosis: Diagnosis
  host: Host
  repositoryPath: string | null
}) {
  const fix = diagnosis.proposed_fix
  const missing = diagnosis.missing_evidence ?? []
  return (
    <>
      <Card>
        <CardHeader>
          <span>Summary</span>
          <Chip tone={categoryTone(diagnosis.category)} className="sentinel-ui-card-end">
            {diagnosis.category}
          </Chip>
          <Chip tone={CONFIDENCE_TONE[diagnosis.confidence]}>confidence {diagnosis.confidence}</Chip>
        </CardHeader>
        <CardBody className="sentinel-ui-prose">
          <Markdown>{diagnosis.summary}</Markdown>
        </CardBody>
      </Card>

      <Card>
        <CardHeader>
          <span>Root cause</span>
          <span className="sentinel-ui-card-note">evidence the agent cited</span>
        </CardHeader>
        <CardBody className="sentinel-ui-stack">
          <div className="sentinel-ui-prose">
            <Markdown>{diagnosis.root_cause.description}</Markdown>
          </div>
          {diagnosis.root_cause.evidence.map((item, index) => {
            const location = codeLocation(item, repositoryPath)
            return (
              <Panel key={index} className="sentinel-ui-cited">
                <Chip>{item.kind}</Chip>
                <div className="sentinel-ui-cited-body">
                  {location ? (
                    <button
                      type="button"
                      className="sentinel-ui-link sentinel-ui-mono"
                      onClick={() =>
                        host.panels?.open({
                          pageId: 'ide',
                          context: { type: 'file', path: location.path, line: location.line },
                        })
                      }
                    >
                      {item.path}:{item.line ?? 1}
                    </button>
                  ) : item.path ? (
                    <span className="sentinel-ui-mono">
                      {item.path}
                      {item.line ? `:${item.line}` : ''}
                    </span>
                  ) : item.span_id ? (
                    <span className="sentinel-ui-mono">span {item.span_id}</span>
                  ) : null}
                  {item.excerpt ? <CodeHighlight code={item.excerpt} language="text" wrap /> : null}
                  <p className="sentinel-ui-quiet">{item.why}</p>
                </div>
              </Panel>
            )
          })}
          {missing.length > 0 ? (
            <div className="sentinel-ui-missing">
              <span className={uiClasses.eyebrow}>What is missing</span>
              <ul>
                {missing.map((item, index) => (
                  <li key={index}>{item}</li>
                ))}
              </ul>
              <p className="sentinel-ui-quiet">
                Tell the agent in the session what it could not see, then ask for the diagnosis again.
              </p>
            </div>
          ) : null}
        </CardBody>
      </Card>

      {fix ? (
        <Card>
          <CardHeader>
            <span>Proposed fix</span>
            <Chip tone={fix.risk === 'low' ? 'success' : fix.risk === 'medium' ? 'warning' : 'danger'} className="sentinel-ui-card-end">
              risk {fix.risk}
            </Chip>
          </CardHeader>
          <CardBody className="sentinel-ui-fix">
            <div className="sentinel-ui-prose">
              <Markdown>{fix.description}</Markdown>
              {fix.steps.length > 0 ? (
                <ol>
                  {fix.steps.map((step, index) => (
                    <li key={index}>{step}</li>
                  ))}
                </ol>
              ) : null}
              <p className="sentinel-ui-quiet">
                Want the patch? Continue in the session — it keeps the transcript, and the read-only policy
                until you widen it.
              </p>
            </div>
            {fix.files.length > 0 ? (
              <div className="sentinel-ui-fix-files">
                <span className={uiClasses.eyebrow}>Files</span>
                {fix.files.map((file) => (
                  <Chip key={file} className="sentinel-ui-mono">
                    {file}
                  </Chip>
                ))}
              </div>
            ) : null}
          </CardBody>
        </Card>
      ) : null}

      {diagnosis.version_note ? <p className="sentinel-ui-foot-note">{diagnosis.version_note}</p> : null}
    </>
  )
}
