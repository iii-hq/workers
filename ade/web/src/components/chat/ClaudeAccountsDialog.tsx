import { Plus, RefreshCw } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { ProjectDialog } from '@/components/projects/ProjectDialog'
import { ProjectDot } from '@/components/projects/ProjectDot'
import { Button } from '@/components/shadcn/button'
import { Dialog } from '@/components/shadcn/dialog'
import {
  PanelDialogContent,
  PanelDialogHeader,
  PanelDialogTitle,
} from '@/components/ui/FormDialog'
import { Hint } from '@/components/ui/Hint'
import { QuietIconButton } from '@/components/ui/QuietIconButton'
import {
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/Table'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/Tabs'
import { useChatProjects } from '@/hooks/use-chat-projects'
import { MOBILE_LAYOUT_QUERY, useMediaQuery } from '@/hooks/use-media-query'
import { getDefaultBackend } from '@/lib/backend'
import {
  type ClaudeAccounts,
  type ClaudeAccountsGate,
  refreshClaudeAccounts,
  TMUX_WORKER,
  useClaudeAccounts,
} from '@/lib/backend/claude-accounts'
import {
  type ChatProject,
  parseProjectAccount,
  tildePath,
} from '@/lib/backend/projects'
import { refreshChatCapabilities } from '@/lib/chat-capabilities'
import {
  ADD_ACCOUNT,
  ALWAYS_GLOBAL,
  ANY_OTHER_FOLDER,
  COLUMN_ACCOUNT,
  COLUMN_FOLDERS,
  COLUMN_LOW,
  COLUMN_PROJECT,
  COLUMN_SESSIONS,
  foldersCell,
  INSTALL_TMUX,
  INTRO_PHONE,
  INTRO_WIDE,
  NEEDS_NEWER_TMUX,
  NEEDS_TMUX,
  NO_PROJECT_ROW,
  OPEN_IN_SETTINGS,
  oldestReading,
  PROJECTS_HEADING,
  PROJECTS_HINT,
  projectAccountCell,
  readAgo,
  SCREEN_TITLE,
  STAYS_PINNED,
  SWITCHES_WITH_GLOBAL,
  sessionsCell,
  TAB_ACCOUNTS_PHONE,
  TAB_ACCOUNTS_WIDE,
  TAB_PROJECTS,
  UPDATE_TMUX,
  usageError,
} from '@/lib/claude-accounts-words'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { addWorkersWithProgress } from '@/lib/onboarding/api'
import type { Conversation } from '@/types/chat'
import { AccountDot } from './AccountCard'
import {
  AccountCards,
  AccountsStatus,
  useAccountActions,
  useUsageNow,
} from './AccountCards'
import {
  type ClaudeAccountsTab,
  onClaudeAccountsDialogRequest,
  requestClaudeAccountsDialog,
} from './claude-accounts-dialog-request'

/**
 * §4.3.6 short of the whole feature: "Update tmux" (a tmux without
 * `tmux::accounts::credential`) or "Install tmux" (no tmux); nothing for the
 * full feature or an engine without `harness::retry`.
 */
export function ClaudeAccountsGateNotice({
  gate,
}: {
  gate: ClaudeAccountsGate
}) {
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  if (gate !== 'update' && gate !== 'install') return null
  const install = async () => {
    setBusy(true)
    setError(null)
    try {
      await addWorkersWithProgress([TMUX_WORKER], ({ note: line }) =>
        setNote(line ?? null),
      )
      setNote(null)
      void refreshChatCapabilities()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
      setNote(null)
    } finally {
      setBusy(false)
    }
  }
  return (
    <div
      data-claude-accounts-gate={gate}
      className="flex flex-col gap-2 rounded-lg bg-muted px-3 py-2.5"
    >
      <div className="flex flex-wrap items-center gap-3">
        <p className="min-w-0 flex-1 text-sm text-ink">
          {gate === 'update' ? NEEDS_NEWER_TMUX : NEEDS_TMUX}
        </p>
        {gate === 'update' ? (
          <Button
            size="sm"
            variant="secondary"
            onClick={() => {
              // The workers page, where tmux updates; the capabilities are
              // read again once it is back.
              window.location.hash = '#/workers'
            }}
          >
            {UPDATE_TMUX}
          </Button>
        ) : (
          <Button
            size="sm"
            variant="secondary"
            disabled={busy}
            onClick={() => void install()}
          >
            {INSTALL_TMUX}
          </Button>
        )}
      </div>
      {note ? (
        <p role="status" className="text-xs text-ink-faint">
          {note}
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="text-xs text-alert">
          {error}
        </p>
      ) : null}
    </div>
  )
}

interface ProjectRow {
  key: string
  project: ChatProject | null
  name: string
  folders: string
  account: string
  accountColor: string | null
  sessions: string
  low: string
}

function projectRows(
  data: ClaudeAccounts,
  projects: readonly ChatProject[],
  conversations: readonly Conversation[],
): ProjectRow[] {
  const claude = data.claude
  const global = claude?.global ?? null
  const globalName = global ? data.nameOf(global) : null
  const globalColor = data.account(global)?.color ?? null
  const roots = conversations.filter((c) => !c.draft && !c.parentId)
  const sessions = (list: Conversation[]) =>
    sessionsCell(list.length, list.filter((c) => c.status === 'working').length)
  const rows: ProjectRow[] = projects.map((project) => {
    const choice = parseProjectAccount(project.account)
    const pinned = choice.mode === 'pinned' ? choice.account_id : null
    return {
      key: project.id,
      project,
      name: project.name,
      folders: foldersCell(project.folders, tildePath),
      account: projectAccountCell(choice, globalName, data.nameOf),
      accountColor: pinned
        ? (data.account(pinned)?.color ?? 'gray')
        : globalColor,
      sessions: sessions(roots.filter((c) => c.projectId === project.id)),
      low: pinned ? STAYS_PINNED : SWITCHES_WITH_GLOBAL,
    }
  })
  const known = new Set(projects.map((p) => p.id))
  rows.push({
    key: 'none',
    project: null,
    name: NO_PROJECT_ROW,
    folders: ANY_OTHER_FOLDER,
    account: ALWAYS_GLOBAL,
    accountColor: globalColor,
    sessions: sessions(
      roots.filter((c) => !c.projectId || !known.has(c.projectId)),
    ),
    low: SWITCHES_WITH_GLOBAL,
  })
  return rows
}

/** "Accounts by project" (§4.3.1): a table, a list of rows on a phone. */
function ProjectsTab({
  data,
  narrow,
  onOpenProject,
}: {
  data: ClaudeAccounts
  narrow: boolean
  onOpenProject: (project: ChatProject) => void
}) {
  const realBackend = getDefaultBackend().id === 'real'
  const chatProjects = useChatProjects(realBackend)
  const ctx = useConversationsCtxOptional()
  const rows = useMemo(
    () => projectRows(data, chatProjects.projects, ctx?.conversations ?? []),
    [data, chatProjects.projects, ctx?.conversations],
  )
  return (
    <section className="flex flex-col gap-3" aria-label={PROJECTS_HEADING}>
      <div className="flex flex-col gap-1">
        <h3 className="text-sm font-semibold text-ink">{PROJECTS_HEADING}</h3>
        <p className="text-xs text-ink-faint">{PROJECTS_HINT}</p>
      </div>
      {narrow ? (
        <ul className="flex flex-col gap-2">
          {rows.map((row) => (
            <li key={row.key}>
              <button
                type="button"
                data-project-account-row={row.key}
                disabled={!row.project}
                onClick={() => row.project && onOpenProject(row.project)}
                className="flex w-full flex-col gap-1 rounded-lg border border-border px-3 py-2.5 text-left text-xs text-ink-faint enabled:hover:bg-surface-hover focus-visible:ring-3 focus-visible:outline-none focus-visible:ring-ring/50"
              >
                <span className="flex items-center gap-2 text-sm font-medium text-ink">
                  {row.project ? (
                    <ProjectDot color={row.project.color} />
                  ) : null}
                  <span className="min-w-0 flex-1 truncate">{row.name}</span>
                  <span className="text-xs font-normal text-ink-faint">
                    {row.sessions}
                  </span>
                </span>
                <span className="truncate font-mono">{row.folders}</span>
                <span className="flex items-center gap-1.5 text-ink">
                  {row.accountColor ? (
                    <AccountDot color={row.accountColor} />
                  ) : null}
                  {row.account}
                </span>
                <span>{row.low}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <TableFrame>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{COLUMN_PROJECT}</TableHead>
                <TableHead>{COLUMN_FOLDERS}</TableHead>
                <TableHead>{COLUMN_ACCOUNT}</TableHead>
                <TableHead>{COLUMN_SESSIONS}</TableHead>
                <TableHead>{COLUMN_LOW}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((row) => (
                <TableRow
                  key={row.key}
                  data-project-account-row={row.key}
                  interactive={Boolean(row.project)}
                  aria-label={
                    row.project ? `Edit ${row.name}'s account` : undefined
                  }
                  onClick={
                    row.project
                      ? () => row.project && onOpenProject(row.project)
                      : undefined
                  }
                >
                  <TableCell>
                    <span className="flex items-center gap-2 font-medium text-ink">
                      {row.project ? (
                        <ProjectDot color={row.project.color} />
                      ) : null}
                      {row.name}
                    </span>
                  </TableCell>
                  <TableCell>
                    <span className="block max-w-56 truncate font-mono text-xs text-ink-faint">
                      {row.folders}
                    </span>
                  </TableCell>
                  <TableCell>
                    <span className="flex items-center gap-1.5 text-ink">
                      {row.accountColor ? (
                        <AccountDot color={row.accountColor} />
                      ) : null}
                      {row.account}
                    </span>
                  </TableCell>
                  <TableCell>
                    <span className="text-ink-faint tabular-nums">
                      {row.sessions}
                    </span>
                  </TableCell>
                  <TableCell>
                    <span className="text-ink-faint">{row.low}</span>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </TableFrame>
      )}
    </section>
  )
}

/** "Accounts and usage": the top line, the intro, the cards. */
function AccountsTab({
  data,
  narrow,
}: {
  data: ClaudeAccounts
  narrow: boolean
}) {
  const actions = useAccountActions(data)
  const now = useUsageNow()
  const accounts = data.claude?.accounts ?? []
  const readAt = oldestReading(accounts) ?? data.usage?.generatedAtMs ?? null
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-xs text-ink-faint" data-accounts-read="">
          {readAt ? readAgo(readAt, now) : null}
        </span>
        <Hint label="Read the usage again">
          <QuietIconButton
            aria-label="Read the usage again"
            disabled={data.refreshing}
            onClick={() => void refreshClaudeAccounts(true)}
          >
            <RefreshCw
              aria-hidden
              className={data.refreshing ? 'size-4 animate-spin' : 'size-4'}
            />
          </QuietIconButton>
        </Hint>
        <span className="flex-1" />
        <Button size="sm" onClick={actions.openAdd}>
          <Plus aria-hidden />
          {ADD_ACCOUNT}
        </Button>
      </div>
      <p className="text-body text-ink-faint">
        {narrow ? INTRO_PHONE : INTRO_WIDE}
      </p>
      {data.error && !data.claude ? (
        <p role="alert" className="text-sm text-alert">
          {usageError(null)}: {data.error}
        </p>
      ) : null}
      <AccountsStatus data={data} actions={actions} />
      <AccountCards
        data={data}
        actions={actions}
        splits
        layout={narrow ? 'stack' : 'grid'}
        time={{ now }}
      />
      {actions.dialogs}
    </div>
  )
}

/**
 * "Claude Code accounts" (§4.3.1, D136): the full screen in a `PanelDialog`,
 * from Settings › Chat, the picker's "Open in Settings", the account chip and
 * the limit card's "Other…". Tabs "Accounts and usage" ("Accounts" on a
 * phone) and "Projects"; a project's row opens its dialog on the account.
 */
export function ClaudeAccountsDialog({
  open,
  onOpenChange,
  initialTab = 'accounts',
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  initialTab?: ClaudeAccountsTab
}) {
  const realBackend = getDefaultBackend().id === 'real'
  const data = useClaudeAccounts(realBackend && open, { splits: open })
  const narrow = useMediaQuery(MOBILE_LAYOUT_QUERY)
  const [tab, setTab] = useState<ClaudeAccountsTab>(initialTab)
  const [editing, setEditing] = useState<ChatProject | null>(null)
  useEffect(() => {
    if (open) setTab(initialTab)
  }, [open, initialTab])
  // A fresh reading on every opening.
  useEffect(() => {
    if (open && data.gate === 'full') void refreshClaudeAccounts()
  }, [open, data.gate])
  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <PanelDialogContent
          data-claude-accounts-dialog=""
          className="flex max-h-[calc(100dvh-2rem)] w-[min(64rem,calc(100vw-2rem))] flex-col [--dialog-max-width:64rem]"
        >
          <PanelDialogHeader>
            <PanelDialogTitle>{SCREEN_TITLE}</PanelDialogTitle>
          </PanelDialogHeader>
          {data.gate !== 'full' ? (
            <div className="p-5">
              <ClaudeAccountsGateNotice gate={data.gate} />
            </div>
          ) : (
            <Tabs
              value={tab}
              onValueChange={(value) => setTab(value as ClaudeAccountsTab)}
              className="flex min-h-0 flex-1 flex-col"
            >
              <div className="px-5 pt-2">
                <TabsList>
                  <TabsTrigger value="accounts" icon={false}>
                    {narrow ? TAB_ACCOUNTS_PHONE : TAB_ACCOUNTS_WIDE}
                  </TabsTrigger>
                  <TabsTrigger value="projects" icon={false}>
                    {TAB_PROJECTS}
                  </TabsTrigger>
                </TabsList>
              </div>
              <div className="min-h-0 flex-1 overflow-y-auto p-5">
                <TabsContent value="accounts">
                  <AccountsTab data={data} narrow={narrow} />
                </TabsContent>
                <TabsContent value="projects">
                  <ProjectsTab
                    data={data}
                    narrow={narrow}
                    onOpenProject={setEditing}
                  />
                </TabsContent>
              </div>
            </Tabs>
          )}
        </PanelDialogContent>
      </Dialog>
      {editing ? (
        <ProjectAccountEditor
          project={editing}
          onClose={() => setEditing(null)}
        />
      ) : null}
    </>
  )
}

/** A project's dialog, opened from the Projects tab on its account section. */
function ProjectAccountEditor({
  project,
  onClose,
}: {
  project: ChatProject
  onClose: () => void
}) {
  const chatProjects = useChatProjects(getDefaultBackend().id === 'real')
  const ctx = useConversationsCtxOptional()
  return (
    <ProjectDialog
      open
      onOpenChange={(next) => {
        if (!next) onClose()
      }}
      project={project}
      projects={chatProjects.projects}
      conversations={ctx?.conversations ?? []}
      initialSection="account"
    />
  )
}

/** The chat pane's one "Claude Code accounts" dialog, answering `requestClaudeAccountsDialog`. */
export function ClaudeAccountsDialogHost() {
  const [open, setOpen] = useState(false)
  // Mounted from the first request on: a chat pane that never opens it
  // reads nothing.
  const [mounted, setMounted] = useState(false)
  const [tab, setTab] = useState<ClaudeAccountsTab>('accounts')
  useEffect(
    () =>
      onClaudeAccountsDialogRequest((next) => {
        setTab(next)
        setMounted(true)
        setOpen(true)
      }),
    [],
  )
  return mounted ? (
    <ClaudeAccountsDialog open={open} onOpenChange={setOpen} initialTab={tab} />
  ) : null
}

/**
 * The model picker's Claude Code page (§4.3.1): the account cards above
 * today's content, then "Open in Settings" (the full screen). Short of the
 * whole feature, the §4.3.6 notice; nothing on an engine without
 * `harness::retry`.
 */
export function ClaudeCodeAccountsSection() {
  const realBackend = getDefaultBackend().id === 'real'
  const data = useClaudeAccounts(realBackend, { splits: true })
  const actions = useAccountActions(data)
  const now = useUsageNow()
  const [ownOpen, setOwnOpen] = useState(false)
  if (data.gate === 'none') return null
  if (data.gate !== 'full') {
    return (
      <div className="pt-2 pb-4">
        <ClaudeAccountsGateNotice gate={data.gate} />
      </div>
    )
  }
  return (
    <section
      aria-label={SCREEN_TITLE}
      data-claude-code-accounts=""
      className="flex flex-col gap-3 pt-2 pb-5"
    >
      <AccountsStatus data={data} actions={actions} />
      <AccountCards
        data={data}
        actions={actions}
        splits
        layout="stack"
        time={{ now }}
      />
      <div>
        <Button
          size="sm"
          variant="secondary"
          onClick={() => {
            // The chat pane's dialog when there is one; else this page's.
            if (!requestClaudeAccountsDialog()) setOwnOpen(true)
          }}
        >
          {OPEN_IN_SETTINGS}
        </Button>
      </div>
      {actions.dialogs}
      {ownOpen ? <ClaudeAccountsDialog open onOpenChange={setOwnOpen} /> : null}
    </section>
  )
}
