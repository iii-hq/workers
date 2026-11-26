import { Folder, X } from 'lucide-react'
import { useEffect, useId, useMemo, useRef, useState } from 'react'
import { DirectoryPicker } from '@/components/chat/DirectoryPicker'
import { Button } from '@/components/shadcn/button'
import { Checkbox } from '@/components/shadcn/checkbox'
import { Dialog } from '@/components/shadcn/dialog'
import {
  FormDialogContent,
  FormDialogDescription,
  FormDialogTitle,
} from '@/components/ui/FormDialog'
import { refreshChatProjects } from '@/hooks/use-chat-projects'
import { useChatSettings } from '@/hooks/use-chat-settings'
import {
  type AutocompactSetting,
  formatTokens,
  parseAutocompactSetting,
  parseTokens,
  sameSetting,
} from '@/lib/autocompact'
import { getDefaultBackend } from '@/lib/backend'
import { useClaudeAccounts } from '@/lib/backend/claude-accounts'
import {
  type ChatProject,
  createProject,
  describeProjectError,
  normalizeFolder,
  PROJECT_COLORS,
  PROJECT_FOLDERS_MAX,
  PROJECT_NAME_MAX,
  type ProjectAccount,
  type ProjectColor,
  parseProjectAccount,
  projectOf,
  tildePath,
  updateProject,
} from '@/lib/backend/projects'
import { useChatCapabilities } from '@/lib/chat-capabilities'
import { cn } from '@/lib/utils'
import type { Conversation } from '@/types/chat'
import { ProjectAccountSection } from './ProjectAccountSection'
import {
  type ProjectAutocompactMode,
  ProjectAutocompactSection,
} from './ProjectAutocompactSection'
import { ProjectDot } from './ProjectDot'

/**
 * A section the dialog can open on: the account (the Projects tab of Claude
 * Code accounts), and auto-compact (the overrides table, phase 3).
 */
export type ProjectDialogSection = 'account' | 'autocompact'

/** The size "Compact at" starts at when the project had none (§3.3.2). */
const AUTOCOMPACT_START = 160_000

export interface ProjectDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** The project to edit; absent for "New project". */
  project?: ChatProject | null
  /** Every project on the engine (names and folders already taken). */
  projects: readonly ChatProject[]
  /** The conversations the folder rows count (roots; side chats left out). */
  conversations: readonly Pick<
    Conversation,
    'id' | 'workingDir' | 'projectId' | 'parentId' | 'draft'
  >[]
  /** Folders a new project starts with. */
  initialFolders?: readonly string[]
  onSaved?: (project: ChatProject, moved: number) => void
  /** The engine call; tests and stories replace it. */
  save?: typeof saveProject
  /** Open scrolled to this section, its first control focused. */
  initialSection?: ProjectDialogSection
}

/** Create or update, then re-read the store. */
export async function saveProject(input: {
  id?: string
  name: string
  color: ProjectColor
  folders: string[]
  bringExisting: boolean
  /** The Claude Code account (§4.3.2); left out when the engine has no accounts. */
  account?: ProjectAccount
  /** Auto-compact (§3.3.2); left out when the engine has no phase 3. */
  autocompact?: AutocompactSetting
  /** Edit: what changed, so the update stays a patch. */
  changed?: {
    name: boolean
    color: boolean
    folders: boolean
    account?: boolean
    autocompact?: boolean
  }
}): Promise<{ project: ChatProject; moved: number }> {
  const result = input.id
    ? await updateProject({
        id: input.id,
        ...(input.changed?.name !== false ? { name: input.name } : {}),
        ...(input.changed?.color !== false ? { color: input.color } : {}),
        ...(input.changed?.folders !== false
          ? { folders: input.folders, bring_existing: input.bringExisting }
          : {}),
        ...(input.account && input.changed?.account !== false
          ? { account: input.account }
          : {}),
        ...(input.autocompact && input.changed?.autocompact !== false
          ? { autocompact: input.autocompact }
          : {}),
      })
    : await createProject({
        name: input.name,
        color: input.color,
        folders: input.folders,
        bring_existing: input.bringExisting,
        ...(input.account ? { account: input.account } : {}),
        // `inherit` is the harness's default: a new project sends a choice.
        ...(input.autocompact && input.autocompact.mode !== 'inherit'
          ? { autocompact: input.autocompact }
          : {}),
      })
  void refreshChatProjects()
  return result
}

function plural(n: number): string {
  return n === 0
    ? 'none yet'
    : n === 1
      ? '1 conversation'
      : `${n} conversations`
}

interface Errors {
  name?: string
  folders?: string
  account?: string
  autocompact?: string
  form?: string
}

/**
 * "New project" and "Edit project" (chat-projects §1.3.2): the name, one of
 * six colours, the folders (each with how many conversations it holds) and,
 * when some are there already, "Bring the N conversations…". The engine's
 * JSON errors land under the field they are about.
 */
export function ProjectDialog({
  open,
  onOpenChange,
  project,
  projects,
  conversations,
  initialFolders,
  onSaved,
  save = saveProject,
  initialSection,
}: ProjectDialogProps) {
  const editing = Boolean(project)
  // §4.3.6: the account section only on an engine with the whole feature.
  const accounts = useClaudeAccounts(getDefaultBackend().id === 'real' && open)
  const withAccounts = accounts.gate === 'full'
  const [account, setAccount] = useState<ProjectAccount>({ mode: 'global' })
  const accountRef = useRef<HTMLElement>(null)
  const accountSectionRef = useRef<HTMLDivElement>(null)
  // §3.3.2: the auto-compact section, on an engine with phase 3 (D168).
  const realBackend = getDefaultBackend().id === 'real'
  const caps = useChatCapabilities(realBackend && open)
  const chatSettings = useChatSettings(realBackend && open)
  const withAutocompact = caps.autocompact
  const [autocompactMode, setAutocompactMode] =
    useState<ProjectAutocompactMode>('inherit')
  const [autocompactText, setAutocompactText] = useState(
    formatTokens(AUTOCOMPACT_START),
  )
  const autocompactRef = useRef<HTMLElement>(null)
  const autocompactFieldRef = useRef<HTMLInputElement>(null)
  const autocompactSectionRef = useRef<HTMLDivElement>(null)
  const [name, setName] = useState('')
  const [color, setColor] = useState<ProjectColor>('blue')
  const [folders, setFolders] = useState<string[]>([])
  const [bring, setBring] = useState(true)
  const [errors, setErrors] = useState<Errors>({})
  const [saving, setSaving] = useState(false)
  const nameRef = useRef<HTMLInputElement>(null)
  const ids = useId()

  // biome-ignore lint/correctness/useExhaustiveDependencies: seeded once per open
  useEffect(() => {
    if (!open) return
    setName(project?.name ?? '')
    setColor(project?.color ?? firstFreeColor(projects))
    setFolders(
      project?.folders.map(normalizeFolder) ??
        (initialFolders ?? []).map(normalizeFolder).filter(Boolean),
    )
    setBring(true)
    setAccount(parseProjectAccount(project?.account))
    const autocompact = parseAutocompactSetting(project?.autocompact)
    setAutocompactMode(autocompact.mode)
    setAutocompactText(
      formatTokens(
        autocompact.mode === 'tokens' ? autocompact.tokens : AUTOCOMPACT_START,
      ),
    )
    setErrors({})
    setSaving(false)
  }, [open])

  // Opened on the auto-compact section: bring it into view.
  useEffect(() => {
    if (!open || initialSection !== 'autocompact' || !withAutocompact) return
    const frame = window.requestAnimationFrame(() =>
      autocompactSectionRef.current?.scrollIntoView?.({ block: 'nearest' }),
    )
    return () => window.cancelAnimationFrame(frame)
  }, [open, initialSection, withAutocompact])

  // Opened on the account section: bring it into view.
  useEffect(() => {
    if (!open || initialSection !== 'account' || !withAccounts) return
    const frame = window.requestAnimationFrame(() =>
      accountSectionRef.current?.scrollIntoView?.({ block: 'nearest' }),
    )
    return () => window.cancelAnimationFrame(frame)
  }, [open, initialSection, withAccounts])

  const roots = useMemo(
    () => conversations.filter((c) => !c.draft && !c.parentId && c.workingDir),
    [conversations],
  )
  const countIn = (folder: string) =>
    roots.filter((c) => projectOf(c.workingDir, [{ folders: [folder] }])).length
  /** Folders this save adds (all of them for a new project). */
  const added = folders.filter(
    (folder) => !project?.folders.some((f) => normalizeFolder(f) === folder),
  )
  const bringable = roots.filter(
    (c) =>
      (!c.projectId || c.projectId === 'none') &&
      projectOf(c.workingDir, [{ folders: added }]) !== null,
  ).length

  const others = projects.filter((p) => p.id !== project?.id)

  const addFolder = (raw: string) => {
    const folder = normalizeFolder(raw)
    if (!folder) return
    setErrors((current) => ({ ...current, folders: undefined }))
    if (folders.includes(folder)) return
    const holder = others.find((p) =>
      p.folders.some((f) => normalizeFolder(f) === folder),
    )
    if (holder) {
      setErrors((current) => ({
        ...current,
        folders: `${tildePath(folder)} is already in “${holder.name}”`,
      }))
      return
    }
    if (folders.length >= PROJECT_FOLDERS_MAX) {
      setErrors((current) => ({
        ...current,
        folders: `A project holds up to ${PROJECT_FOLDERS_MAX} folders`,
      }))
      return
    }
    setFolders((current) => [...current, folder])
  }

  const validate = (): Errors => {
    const next: Errors = {}
    const trimmed = name.trim()
    const taken = trimmed
      ? others.find((p) => p.name.toLowerCase() === trimmed.toLowerCase())
      : undefined
    if (!trimmed) next.name = 'Name is required'
    // The engine's copy: the name the existing project has, whatever case
    // was typed (§1.3.2, harness `name_taken`).
    else if (taken) next.name = `A project named “${taken.name}” already exists`
    if (folders.length === 0) next.folders = 'Add at least one folder'
    return next
  }

  const submit = async () => {
    const found = validate()
    setErrors(found)
    if (found.name) {
      nameRef.current?.focus()
      return
    }
    if (found.folders) return
    let autocompact: AutocompactSetting | undefined
    if (withAutocompact) {
      if (autocompactMode === 'tokens') {
        const parsed = parseTokens(autocompactText)
        if (!parsed.ok) {
          setErrors({ autocompact: parsed.error })
          autocompactFieldRef.current?.focus()
          return
        }
        autocompact = { mode: 'tokens', tokens: parsed.tokens }
      } else {
        autocompact = { mode: autocompactMode }
      }
    }
    setSaving(true)
    try {
      const trimmed = name.trim()
      const result = await save({
        id: project?.id,
        name: trimmed,
        color,
        folders,
        bringExisting: bring && bringable > 0,
        ...(withAccounts ? { account } : {}),
        ...(autocompact ? { autocompact } : {}),
        changed: project
          ? {
              name: trimmed !== project.name,
              color: color !== project.color,
              folders:
                folders.join('\n') !==
                project.folders.map(normalizeFolder).join('\n'),
              ...(withAccounts
                ? {
                    account:
                      JSON.stringify(account) !==
                      JSON.stringify(parseProjectAccount(project.account)),
                  }
                : {}),
              ...(autocompact
                ? {
                    autocompact: !sameSetting(
                      autocompact,
                      parseAutocompactSetting(project.autocompact),
                    ),
                  }
                : {}),
            }
          : undefined,
      })
      onSaved?.(result.project, result.moved)
      onOpenChange(false)
    } catch (err) {
      const shown = describeProjectError(err, { name, folders: added })
      setErrors({ [shown.field]: shown.message })
      if (shown.field === 'name') nameRef.current?.focus()
      if (shown.field === 'autocompact') autocompactFieldRef.current?.focus()
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <FormDialogContent
        className="w-[min(32rem,calc(100vw-2rem))] [--dialog-max-width:32rem]"
        initialFocus={
          initialSection === 'account' && withAccounts
            ? accountRef
            : initialSection === 'autocompact' && withAutocompact
              ? autocompactRef
              : nameRef
        }
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.stopPropagation()}
      >
        <form
          noValidate
          className="flex max-h-[min(44rem,calc(100dvh-2rem))] flex-col gap-5 overflow-y-auto p-5"
          onSubmit={(e) => {
            e.preventDefault()
            void submit()
          }}
        >
          <div className="flex flex-col gap-1 pr-8">
            <FormDialogTitle>
              {editing ? 'Edit project' : 'New project'}
            </FormDialogTitle>
            <FormDialogDescription>
              A project groups folders and the conversations started in them.
            </FormDialogDescription>
          </div>

          <div className="flex flex-col gap-1.5">
            <label
              htmlFor={`${ids}-name`}
              className="text-body font-medium text-foreground"
            >
              Name
            </label>
            <input
              ref={nameRef}
              id={`${ids}-name`}
              name="project-name"
              value={name}
              maxLength={PROJECT_NAME_MAX}
              autoComplete="off"
              spellCheck={false}
              aria-invalid={errors.name ? true : undefined}
              aria-describedby={errors.name ? `${ids}-name-error` : undefined}
              onChange={(e) => {
                setName(e.currentTarget.value)
                if (errors.name) setErrors((c) => ({ ...c, name: undefined }))
              }}
              className="h-10 w-full rounded-lg bg-muted px-3 text-sm text-foreground ring-1 ring-transparent outline-none focus-visible:ring-ring/50 aria-invalid:ring-alert/60"
            />
            {errors.name ? (
              <p
                id={`${ids}-name-error`}
                role="alert"
                className="text-xs text-alert"
              >
                {errors.name}
              </p>
            ) : null}
          </div>

          <div className="flex flex-col gap-2">
            <span id={`${ids}-color`} className="text-body font-medium">
              Colour
            </span>
            <div
              role="radiogroup"
              aria-labelledby={`${ids}-color`}
              className="flex items-center gap-2"
            >
              {PROJECT_COLORS.map((option) => {
                const selected = option.value === color
                return (
                  // biome-ignore lint/a11y/useSemanticElements: a swatch row, not a native radio list
                  <button
                    key={option.value}
                    type="button"
                    role="radio"
                    aria-checked={selected}
                    aria-label={
                      selected ? `${option.label}, selected` : option.label
                    }
                    onClick={() => setColor(option.value)}
                    className={cn(
                      'flex size-8 items-center justify-center rounded-full focus-visible:ring-3 focus-visible:outline-none focus-visible:ring-ring/50 pointer-coarse:size-11',
                      selected
                        ? 'bg-surface-selected'
                        : 'hover:bg-surface-hover',
                    )}
                  >
                    <ProjectDot color={option.value} size="swatch" />
                  </button>
                )
              })}
            </div>
          </div>

          <div className="flex flex-col gap-1.5">
            <span id={`${ids}-folders`} className="text-body font-medium">
              Folders
            </span>
            <ul
              aria-labelledby={`${ids}-folders`}
              className="flex flex-col rounded-lg bg-muted/60"
            >
              {folders.map((folder) => (
                <li
                  key={folder}
                  className="flex min-h-10 items-center gap-2 pr-1 pl-3"
                >
                  <Folder
                    aria-hidden
                    className="size-4 shrink-0 text-ink-faint"
                  />
                  <span
                    data-keep-mono=""
                    className="min-w-0 flex-1 truncate font-mono text-xs text-ink"
                    title={folder}
                  >
                    {tildePath(folder)}
                  </span>
                  <span className="shrink-0 text-xs text-ink-faint tabular-nums">
                    {plural(countIn(folder))}
                  </span>
                  <button
                    type="button"
                    aria-label="Remove folder"
                    title="Remove folder"
                    onClick={() =>
                      setFolders((current) =>
                        current.filter((f) => f !== folder),
                      )
                    }
                    className="flex size-8 shrink-0 items-center justify-center rounded-md text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:ring-3 focus-visible:outline-none focus-visible:ring-ring/50 pointer-coarse:size-11"
                  >
                    <X aria-hidden className="size-4" />
                  </button>
                </li>
              ))}
              <li className="flex min-h-10 items-center px-1">
                <DirectoryPicker
                  value={null}
                  onChange={addFolder}
                  triggerAppearance="chip"
                  emptyLabel="Add folder…"
                  popoverAlign="start"
                />
              </li>
            </ul>
            {errors.folders ? (
              <p role="alert" className="text-xs text-alert">
                {errors.folders}
              </p>
            ) : null}
          </div>

          {bringable > 0 ? (
            <div className="flex items-center gap-2.5 text-sm text-foreground">
              <Checkbox
                id={`${ids}-bring`}
                checked={bring}
                onCheckedChange={(checked) => setBring(checked === true)}
              />
              <label htmlFor={`${ids}-bring`}>
                {`Bring the ${bringable} ${bringable === 1 ? 'conversation' : 'conversations'} already in these folders`}
              </label>
            </div>
          ) : null}

          {withAccounts ? (
            <div ref={accountSectionRef}>
              <ProjectAccountSection
                value={account}
                onChange={(next) => {
                  setAccount(next)
                  if (errors.account) {
                    setErrors((c) => ({ ...c, account: undefined }))
                  }
                }}
                data={accounts}
                error={errors.account}
                firstRef={accountRef}
              />
            </div>
          ) : null}

          {withAutocompact ? (
            <div ref={autocompactSectionRef}>
              <ProjectAutocompactSection
                mode={autocompactMode}
                text={autocompactText}
                onModeChange={(next) => {
                  setAutocompactMode(next)
                  if (errors.autocompact) {
                    setErrors((c) => ({ ...c, autocompact: undefined }))
                  }
                }}
                onTextChange={(next) => {
                  setAutocompactText(next)
                  if (errors.autocompact) {
                    setErrors((c) => ({ ...c, autocompact: undefined }))
                  }
                }}
                global={chatSettings.autocompact}
                error={errors.autocompact}
                firstRef={autocompactRef}
                fieldRef={autocompactFieldRef}
              />
            </div>
          ) : null}

          {errors.form ? (
            <p role="alert" className="text-sm text-alert">
              {errors.form}
            </p>
          ) : null}

          <div className="flex items-center justify-end gap-2">
            <Button
              type="button"
              variant="secondary"
              size="lg"
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" size="lg" disabled={saving}>
              {editing ? 'Save' : 'Create project'}
            </Button>
          </div>
        </form>
      </FormDialogContent>
    </Dialog>
  )
}

/** A new project takes the first colour no project uses yet. */
function firstFreeColor(projects: readonly ChatProject[]): ProjectColor {
  const used = new Set(projects.map((p) => p.color))
  return PROJECT_COLORS.find((c) => !used.has(c.value))?.value ?? 'blue'
}
