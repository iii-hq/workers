/**
 * Screen B — one installed kit: Overview (README), Profiles and Skills
 * (read-only previews of the files on disk, with "Edit in Directory"),
 * Workers (as Compose has them) and Files (intact / edited / missing, with
 * "View my changes" for edited ones). Remove and the registry link sit in
 * the footer.
 */

import {
  Badge,
  Button,
  type Host,
  MarkdownPreview,
  Skeleton,
  StatusPanel,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@iii-dev/console-ui'
import uiClasses from '@iii-dev/console-ui/ui-classes'
import { ArrowUpCircle, Cpu, FileText } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { ago } from '../lib/format'
import { TokenIcon } from '../page/agent-fields'
import { errorText, kitsApi, useKitsChange } from './api'
import { ContentRow } from './InstallReview'
import {
  BackButton,
  DiffDialog,
  ExternalLinkButton,
  FileStateChip,
  KitMark,
  ProfilePreview,
  SkillPreview,
} from './parts'
import type { InstalledFile, KitInfo } from './types'

type Tab = 'overview' | 'profiles' | 'skills' | 'workers' | 'files'

export function KitDetail({
  host,
  kit,
  onBack,
  onReviewUpdate,
  onRemove,
  onOpenEntry,
  busy,
}: {
  host: Host
  kit: string
  onBack: () => void
  onReviewUpdate: (kit: string) => void
  onRemove: (kit: string) => void
  onOpenEntry: (collection: 'agents' | 'skills', key: string) => void
  busy?: string | null
}) {
  const [info, setInfo] = useState<KitInfo | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [tab, setTab] = useState<Tab>('overview')
  const load = useCallback(() => {
    kitsApi(host)
      .get(kit)
      .then((next) => {
        setInfo(next)
        setError(null)
      })
      .catch((e) => setError(errorText(e)))
  }, [host, kit])
  useEffect(load, [load])
  useKitsChange(host, `detail-${kit}`, (change) => {
    if (!change.kit || change.kit === kit || change.op === 'updates') load()
  })

  if (error) {
    return (
      <div className="dir-ui-kit-screen">
        <div className="dir-ui-kit-scroll">
          <BackButton onClick={onBack} />
          <StatusPanel variant="alert" headline={`${kit} could not be loaded.`} detail={error} />
        </div>
      </div>
    )
  }
  if (!info) {
    return (
      <div className="dir-ui-kit-screen">
        <div className="dir-ui-kit-scroll dir-ui-loading" aria-hidden>
          <Skeleton style={{ width: '40%' }} />
          <Skeleton style={{ width: '65%' }} />
          <Skeleton style={{ width: '55%' }} />
        </div>
      </div>
    )
  }

  const agents = info.file_list.filter((f) => f.kind === 'agent')
  const skills = info.file_list.filter((f) => f.kind === 'skill')
  const update = info.update?.available ? info.update : null
  const detail = info.detail

  return (
    <div className="dir-ui-kit-screen">
      <header className="dir-ui-kit-head">
        <div className="dir-ui-kit-head-top">
          <BackButton onClick={onBack} />
          <h2 className="dir-ui-kit-title">
            {info.kit} <span className="dir-ui-kit-version">{info.version}</span>
          </h2>
          {update ? (
            <Button variant="primary" size="sm" onClick={() => onReviewUpdate(info.kit)} disabled={busy === info.kit}>
              <ArrowUpCircle aria-hidden />
              {busy === info.kit ? 'Planning…' : `Review update ${update.available}`}
            </Button>
          ) : null}
          <span className="dir-ui-kit-head-mark">
            <KitMark />
          </span>
        </div>
        <div className="dir-ui-kit-meta">
          {detail?.author ? <span>by {detail.author.name || detail.author.handle}</span> : null}
          {detail?.license ? <span>{detail.license}</span> : null}
          <span>
            installed {ago(info.installed_at)} · asked for <code>{info.requested}</code>
          </span>
          {update?.major ? <Badge variant="alert">major update</Badge> : null}
          {info.update?.ignored ? <span>ignoring {info.update.ignored}</span> : null}
        </div>
        {detail?.description ? <p className="dir-ui-kit-desc">{detail.description}</p> : null}
      </header>
      <Tabs value={tab} onValueChange={(v) => setTab(v as Tab)} className="dir-ui-kit-detail-tabs">
        <TabsList variant="line" className="dir-ui-kit-detail-tablist">
          <TabsTrigger value="overview" icon={false}>
            Overview
          </TabsTrigger>
          <TabsTrigger value="profiles" icon={false}>
            Profiles ({agents.length})
          </TabsTrigger>
          <TabsTrigger value="skills" icon={false}>
            Skills ({skills.length})
          </TabsTrigger>
          <TabsTrigger value="workers" icon={false}>
            Workers ({info.worker_status.length})
          </TabsTrigger>
          <TabsTrigger value="files" icon={false}>
            Files
            {info.files.edited + info.files.missing > 0 ? ` · ${info.files.edited + info.files.missing} changed` : ''}
          </TabsTrigger>
        </TabsList>
        <TabsContent value="overview" className="dir-ui-kit-scroll">
          {info.readme ? (
            <MarkdownPreview markdown={info.readme} className="dir-ui-preview dir-ui-kit-md" />
          ) : (
            <p className="dir-ui-kit-fine">The registry has no README for this version (or is unreachable).</p>
          )}
        </TabsContent>
        <TabsContent value="profiles" className="dir-ui-kit-split">
          <FileBrowser host={host} kit={info.kit} files={agents} detail={info} onOpenEntry={onOpenEntry} />
        </TabsContent>
        <TabsContent value="skills" className="dir-ui-kit-split">
          <FileBrowser host={host} kit={info.kit} files={skills} detail={info} onOpenEntry={onOpenEntry} />
        </TabsContent>
        <TabsContent value="workers" className="dir-ui-kit-scroll">
          <ul className="dir-ui-kit-table">
            {info.worker_status.map((w) => (
              <li key={w.name} className="dir-ui-kit-table-row">
                <Cpu aria-hidden className="dir-ui-kit-table-icon" />
                <span className="dir-ui-kit-path">{w.name}</span>
                <span className="dir-ui-kit-fine">
                  range <code>{w.range}</code>
                </span>
                <span className="dir-ui-kit-fine">
                  {w.installed ? (
                    <>
                      installed <code>{w.installed}</code>
                    </>
                  ) : (
                    'not installed'
                  )}
                  {w.declared && w.declared !== w.installed ? ` · declared ${w.declared}` : ''}
                  {w.path ? ' · local path worker' : ''}
                </span>
                {w.satisfied === false ? <Badge variant="warn">outside the range</Badge> : null}
                {w.satisfied === true ? <Badge variant="ok">ok</Badge> : null}
                <ExternalLinkButton href={w.registry_url}>registry</ExternalLinkButton>
              </li>
            ))}
          </ul>
        </TabsContent>
        <TabsContent value="files" className="dir-ui-kit-scroll">
          <FilesTable host={host} kit={info.kit} files={info.file_list} />
        </TabsContent>
      </Tabs>
      <footer className="dir-ui-kit-foot">
        <Button
          variant="ghost"
          size="sm"
          className="dir-ui-kit-danger-ghost"
          onClick={() => onRemove(info.kit)}
          disabled={busy === info.kit}
        >
          Remove kit
        </Button>
        <span className="dir-ui-kit-foot-gap" />
        <span className="dir-ui-kit-fine">
          {info.files.agents} profiles · {info.files.skills} skills · {Object.keys(info.workers).length} workers
        </span>
        <ExternalLinkButton href={info.registry_url}>Open in registry</ExternalLinkButton>
      </footer>
    </div>
  )
}

/** List + read-only preview of the kit's files as they are on disk. */
function FileBrowser({
  host,
  kit,
  files,
  detail,
  onOpenEntry,
}: {
  host: Host
  kit: string
  files: InstalledFile[]
  detail: KitInfo
  onOpenEntry: (collection: 'agents' | 'skills', key: string) => void
}) {
  const [selected, setSelected] = useState<string | null>(files[0]?.path ?? null)
  const [content, setContent] = useState<string | undefined>(undefined)
  const file = files.find((f) => f.path === selected)
  useEffect(() => {
    if (!file) return
    setContent(undefined)
    if (file.state === 'missing') {
      setContent('')
      return
    }
    const load =
      file.kind === 'agent'
        ? host.iii
            .trigger<{ raw?: string; system_prompt?: string }>('directory::agents::get', { id: file.id, raw: true })
            .then((o) => o.raw ?? o.system_prompt ?? '')
        : host.iii
            .trigger<{ raw?: string; body?: string }>('directory::skills::get', { id: file.id, raw: true })
            .then((o) => o.raw ?? o.body ?? '')
    load.then(setContent).catch(() => setContent(''))
  }, [host, file])
  if (files.length === 0) return <p className="dir-ui-kit-fine dir-ui-kit-pad">Nothing here.</p>
  const agentEntry = detail.detail?.agents?.find((a) => `agents/${a.id}.md` === file?.path)
  const skillEntry = detail.detail?.skills?.find((s) => s.id === file?.id)
  return (
    <>
      <div className={`${uiClasses.list} dir-ui-kit-split-list`}>
        {files.map((f) => (
          <ContentRow
            key={f.path}
            selected={f.path === selected}
            onClick={() => setSelected(f.path)}
            icon={f.kind === 'agent' ? <TokenIcon token="agent" /> : <FileText />}
            label={f.kind === 'agent' ? f.id : f.id.replace(`${kit}/`, '')}
            status={f.state === 'intact' ? undefined : f.state === 'skipped' ? 'kept yours' : f.state}
          />
        ))}
      </div>
      <div className="dir-ui-kit-split-main">
        {file ? (
          <>
            <div className="dir-ui-kit-detail-head">
              <span className="dir-ui-kit-path">{file.path}</span>
              <FileStateChip state={file.state} />
              {file.state !== 'missing' && file.state !== 'skipped' ? (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => onOpenEntry(file.kind === 'agent' ? 'agents' : 'skills', file.id)}
                >
                  Edit in Directory
                </Button>
              ) : null}
            </div>
            {file.state === 'missing' ? (
              <p className="dir-ui-kit-fine">
                This file was deleted. The next update restores it unless you keep it deleted.
              </p>
            ) : file.kind === 'agent' ? (
              <ProfilePreview kit={kit} agent={agentEntry} content={content} />
            ) : (
              <SkillPreview path={file.path} skill={skillEntry} content={content} />
            )}
          </>
        ) : null}
      </div>
    </>
  )
}

function FilesTable({ host, kit, files }: { host: Host; kit: string; files: InstalledFile[] }) {
  const [diff, setDiff] = useState<{ path: string; base: string; local: string } | null>(null)
  const [loadingPath, setLoadingPath] = useState<string | null>(null)
  const open = (path: string) => {
    setLoadingPath(path)
    kitsApi(host)
      .diff(kit, path)
      .then((d) => setDiff({ path, base: d.base ?? '', local: d.local ?? '' }))
      .catch(() => setDiff({ path, base: '', local: '' }))
      .finally(() => setLoadingPath(null))
  }
  return (
    <>
      <ul className="dir-ui-kit-table">
        {files.map((f) => (
          <li key={f.path} className="dir-ui-kit-table-row">
            {f.kind === 'agent' ? (
              <TokenIcon token="agent" className="dir-ui-kit-table-icon" />
            ) : (
              <FileText aria-hidden className="dir-ui-kit-table-icon" />
            )}
            <span className="dir-ui-kit-path">{f.path}</span>
            <FileStateChip state={f.state} />
            {f.state === 'edited' ? (
              <Button variant="ghost" size="sm" onClick={() => open(f.path)} disabled={loadingPath === f.path}>
                {loadingPath === f.path ? 'Loading…' : 'View my changes'}
              </Button>
            ) : null}
          </li>
        ))}
      </ul>
      <DiffDialog
        open={diff !== null}
        onOpenChange={(o) => (o ? null : setDiff(null))}
        title={diff?.path ?? ''}
        description="Left: what the kit installed. Right: the file on disk."
        left={diff ? { name: `kit/${diff.path}`, contents: diff.base } : null}
        right={diff ? { name: `local/${diff.path}`, contents: diff.local } : null}
      />
    </>
  )
}
