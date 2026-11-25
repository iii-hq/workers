/**
 * The Artifacts section of the chat's Context panel: the most recently
 * updated artifacts with their live-link count. A row opens the artifact on
 * the Artifacts page beside the chat. The console draws the header from
 * `setSummary`.
 */

import { type ContextSectionProps, type Host, List, ListItem } from '@iii-dev/console-ui'
import { formatRelative } from '@iii-dev/console-ui/format'
import { useWorkerLive } from '@iii-dev/console-ui/hooks'
import { FileCode2, FileText, Image } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { useShareRefresh } from '../lib/refresh'
import { type ArtifactFormat, createApi, type IndexEntry, MUTATING_FUNCTIONS, PAGE_ID, type ShareView } from '../lib/api'

/** Rows shown before "Show all". */
const SECTION_CAP = 5

const FORMAT_ICON: Record<ArtifactFormat, typeof FileCode2> = {
  html: FileCode2,
  markdown: FileText,
  svg: Image,
}

interface Snapshot {
  artifacts: IndexEntry[]
  total: number
  shares: ShareView[]
}

/** Adds the section where the console has the slot; `undefined` on an older console. */
export function registerArtifactSection(host: Host): (() => void) | undefined {
  function Render(props: ContextSectionProps) {
    return <ArtifactSection {...props} host={host} />
  }
  return host.chat?.registerContextSection?.({
    id: 'artifact-recent',
    title: 'Artifacts',
    order: 650,
    render: Render,
  })
}

function ArtifactSection({ host, setSummary }: ContextSectionProps & { host: Host }) {
  const api = useMemo(() => createApi(host), [host])
  const [showAll, setShowAll] = useState(false)
  const live = useWorkerLive<Snapshot>({
    iii: host.iii,
    sources: MUTATING_FUNCTIONS,
    handlerId: 'iii::artifact-ui::context',
    fetch: async () => {
      const [list, shares] = await Promise.all([api.list(), api.shares()])
      return { artifacts: list.artifacts, total: list.total, shares: shares.shares }
    },
  })
  useShareRefresh(live.data?.shares, live.refresh)

  const artifacts = live.data?.artifacts ?? []
  const shares = live.data?.shares ?? []
  // The header counts every artifact, not just the listed page.
  const count = live.data?.total ?? artifacts.length
  const linkCount = shares.length
  const brief = linkCount > 0 ? `${linkCount} live ${linkCount === 1 ? 'link' : 'links'}` : undefined
  useEffect(() => {
    setSummary({ count, empty: count === 0, brief, tone: linkCount > 0 ? 'accent' : undefined })
  }, [setSummary, count, brief, linkCount])

  const shown = showAll ? artifacts : artifacts.slice(0, SECTION_CAP)
  const open = (id: string) => host.panels?.open({ pageId: PAGE_ID, context: { id } })

  return (
    <List aria-label="Artifacts" className="artifact-ui-context">
      {shown.map((a) => {
        const Icon = FORMAT_ICON[a.format] ?? FileCode2
        const links = shares.filter((s) => s.artifact_id === a.id).length
        return (
          <ListItem
            key={a.id}
            leading={<Icon aria-hidden />}
            label={a.title}
            description={`${a.kind} · v${a.latest_version} · ${formatRelative(a.updated_at * 1000)}`}
            trailing={links ? <span className="artifact-ui-muted">{links} live</span> : undefined}
            aria-label={`Open ${a.title}`}
            onClick={() => open(a.id)}
          />
        )
      })}
      {artifacts.length > SECTION_CAP ? (
        <ListItem
          label={showAll ? 'Show fewer' : `Show all ${artifacts.length}`}
          onClick={() => setShowAll(!showAll)}
        />
      ) : null}
    </List>
  )
}
