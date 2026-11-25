/**
 * Chat card for artifact::create / update / get / share.
 *
 * A stored artifact renders as a live, sandboxed preview with the two things
 * a person does next: Share… (version, lifetime, audience) and Open (the
 * Artifacts page beside the chat). A share renders its links with copy
 * buttons. Errors and pending approvals fall through to the host.
 */

import {
  Button,
  type FunctionCallMessage,
  type FunctionCallRenderer,
  type Host,
  MetaRow,
  uiClasses,
} from '@iii-dev/console-ui'
import { unwrapEnvelope } from '@iii-dev/console-ui/format'
import { PanelRight, Share2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { ArtifactPreview } from '../components/ArtifactPreview'
import { ShareDialog, ShareLinks } from '../components/ShareDialog'
import { type ArtifactApi, type ArtifactRecord, createApi, isRecord, PAGE_ID, recordOf, shareOf } from '../lib/api'

const RENDERED = new Set(['artifact::create', 'artifact::update', 'artifact::get', 'artifact::share'])

function openPage(host: Host, id: string) {
  host.panels?.open({ pageId: PAGE_ID, context: { id } })
}

function ArtifactCard({
  host,
  api,
  record,
  version,
}: {
  host: Host
  api: ArtifactApi
  record: ArtifactRecord
  version: number
}) {
  const [sharing, setSharing] = useState(false)
  return (
    <div className="artifact-ui-card">
      <MetaRow
        items={[
          { label: 'artifact', value: record.title, tone: 'accent' },
          { label: 'kind', value: record.kind },
          { label: 'version', value: `v${version}` },
          { label: 'format', value: record.format },
        ]}
      />
      <ArtifactPreview api={api} id={record.id} version={version} size="card" />
      <div className="artifact-ui-card-actions">
        <Button variant="primary" size="sm" onClick={() => setSharing(true)}>
          <Share2 />
          Share…
        </Button>
        {host.panels ? (
          <Button variant="secondary" size="sm" onClick={() => openPage(host, record.id)}>
            <PanelRight />
            Open
          </Button>
        ) : null}
      </div>
      <ShareDialog
        api={api}
        artifact={record}
        version={version}
        open={sharing}
        onOpenChange={setSharing}
      />
    </div>
  )
}

function SettledView({ host, message }: { host: Host; message: FunctionCallMessage }) {
  const api = useMemo(() => createApi(host), [host])
  const output = unwrapEnvelope(message.output)

  if (message.functionId === 'artifact::share') {
    const share = shareOf(output)
    if (!share) return null
    return (
      <div className="artifact-ui-card">
        <MetaRow
          items={[
            { label: 'shared', value: share.artifact_id, tone: 'accent' },
            { label: 'version', value: `v${share.version}` },
            { label: 'expires', value: new Date(share.expires_at * 1000).toLocaleString() },
          ]}
        />
        <ShareLinks share={share} />
        {host.panels ? (
          <div className="artifact-ui-card-actions">
            <Button variant="secondary" size="sm" onClick={() => openPage(host, share.artifact_id)}>
              <PanelRight />
              Open
            </Button>
          </div>
        ) : null}
      </div>
    )
  }

  const record = recordOf(output)
  if (!record) return null
  const version =
    isRecord(output) && typeof output.version === 'number' ? output.version : record.latest_version
  return <ArtifactCard host={host} api={api} record={record} version={version} />
}

function isError(value: unknown): boolean {
  return isRecord(value) && 'error' in value
}

export function createArtifactCallRenderer(host: Host): FunctionCallRenderer {
  const settled = (message: FunctionCallMessage) => {
    if (!RENDERED.has(message.functionId)) return null
    if (message.pendingApproval || message.running) return null
    if (isError(message.output)) return null
    return <SettledView host={host} message={message} />
  }
  return {
    id: 'artifact/page.js#calls',
    isMatch: (functionId) => RENDERED.has(functionId),
    tryRender: settled,
    tryRenderRunning: (message) =>
      RENDERED.has(message.functionId) ? (
        <div className={`artifact-ui-muted ${uiClasses.pulse}`}>· working…</div>
      ) : null,
    metadata: { display: true },
  }
}
