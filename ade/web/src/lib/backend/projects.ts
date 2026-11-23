/**
 * The Chat's projects (`harness::projects::*`, chat-projects §1.2.2): a named,
 * coloured group of folders and the conversations started in them. Also the
 * harness's chat settings (`harness::chat-settings::*`, §1.2.5) and the
 * merged metadata write the new features use (`session::patch-meta`, D9).
 *
 * Only on engines whose catalogue lists `harness::projects::create`
 * (`ChatCapabilities.projects`); the saved-folder catalogue that used to
 * answer `harness::projects::*` is `./folders.ts`.
 *
 * A write never sends `account` or `autocompact` (phases 4 and 3): `update`
 * is a patch, so a phase-1 client cannot erase what a later one set.
 */
import { errorCode } from '@iii-dev/console-ui/format'
import { errText } from '@/lib/errors'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'

export { patchSessionMeta } from '@/lib/sessions/api'

export const PROJECTS_LIST_FUNCTION_ID = 'harness::projects::list'
export const PROJECTS_CREATE_FUNCTION_ID = 'harness::projects::create'
export const PROJECTS_UPDATE_FUNCTION_ID = 'harness::projects::update'
export const PROJECTS_DELETE_FUNCTION_ID = 'harness::projects::delete'
export const PROJECTS_ASSIGN_FUNCTION_ID = 'harness::projects::assign'
export const PROJECTS_CHANGED_FUNCTION_ID = 'harness::projects::on-changed'
export const CHAT_SETTINGS_GET_FUNCTION_ID = 'harness::chat-settings::get'
export const CHAT_SETTINGS_SET_FUNCTION_ID = 'harness::chat-settings::set'
export const CHAT_SETTINGS_CHANGED_FUNCTION_ID =
  'harness::chat-settings::on-changed'

/** `metadata.project_id` of a conversation kept out of every project on purpose. */
export const NO_PROJECT = 'none'

export type ProjectColor =
  | 'blue'
  | 'purple'
  | 'teal'
  | 'green'
  | 'amber'
  | 'rose'

/** The six hues (§0.6), in swatch order, with their accessible names (§1.3.2). */
export const PROJECT_COLORS: readonly { value: ProjectColor; label: string }[] =
  [
    { value: 'blue', label: 'Blue' },
    { value: 'purple', label: 'Violet' },
    { value: 'teal', label: 'Teal' },
    { value: 'green', label: 'Green' },
    { value: 'amber', label: 'Amber' },
    { value: 'rose', label: 'Rose' },
  ]

export function isProjectColor(value: unknown): value is ProjectColor {
  return PROJECT_COLORS.some((color) => color.value === value)
}

export const PROJECT_NAME_MAX = 40
export const PROJECT_FOLDERS_MAX = 20

export interface ChatProject {
  id: string
  name: string
  color: ProjectColor
  folders: string[]
  created_at_ms: number
  updated_at_ms: number
  /** Phase 4 and 3 fields, kept as the engine sent them and never written here. */
  account?: unknown
  autocompact?: unknown
}

export interface ProjectList {
  revision: number
  projects: ChatProject[]
}

export interface ProjectChange {
  revision: number
  project_id: string
  change: 'created' | 'updated' | 'deleted'
}

/** No trailing slash (except `/` itself), the way the engine stores folders. */
export function normalizeFolder(path: string): string {
  const trimmed = path.trim()
  if (!trimmed) return ''
  return trimmed.replace(/\/+$/, '') || '/'
}

/**
 * The project a path belongs to: the one whose folder is the longest prefix
 * of it, equal or followed by `/` (§1.2.2 folder rule). The same pure
 * function, with the same test table, on every surface.
 */
export function projectOf<P extends Pick<ChatProject, 'folders'>>(
  path: string | null | undefined,
  projects: readonly P[],
): P | null {
  const target = normalizeFolder(path ?? '')
  if (!target) return null
  let best: P | null = null
  let bestLength = -1
  for (const project of projects) {
    for (const raw of project.folders) {
      const folder = normalizeFolder(raw)
      if (!folder) continue
      const inside =
        target === folder ||
        (folder === '/'
          ? target.startsWith('/')
          : target.startsWith(`${folder}/`))
      if (inside && folder.length > bestLength) {
        best = project
        bestLength = folder.length
      }
    }
  }
  return best
}

/** `/home/u/x` and `/Users/u/x` read `~/x`, as the folder rows show them. */
export function tildePath(path: string): string {
  return path.replace(/^\/(?:Users|home)\/[^/]+(?=\/|$)/, '~')
}

function str(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined
}

function num(value: unknown, fallback = 0): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}

/** One project out of untrusted wire JSON; null when it has no id. */
export function parseProject(raw: unknown): ChatProject | null {
  if (!raw || typeof raw !== 'object') return null
  const record = raw as Record<string, unknown>
  const id = str(record.id)
  if (!id) return null
  const project: ChatProject = {
    id,
    name: str(record.name) ?? id,
    color: isProjectColor(record.color) ? record.color : 'blue',
    folders: Array.isArray(record.folders)
      ? record.folders.filter((f): f is string => typeof f === 'string')
      : [],
    created_at_ms: num(record.created_at_ms),
    updated_at_ms: num(record.updated_at_ms),
  }
  if ('account' in record) project.account = record.account
  if ('autocompact' in record) project.autocompact = record.autocompact
  return project
}

export function parseProjectList(raw: unknown): ProjectList {
  const record = (raw ?? {}) as { revision?: unknown; projects?: unknown }
  const projects = Array.isArray(record.projects)
    ? record.projects.flatMap((p) => {
        const parsed = parseProject(p)
        return parsed ? [parsed] : []
      })
    : []
  return { revision: num(record.revision), projects }
}

async function client(provided?: IiiClient): Promise<IiiClient> {
  return provided ?? (await getIiiClient())
}

/** Every project (`version: 2`: without it a D26 shim answers the folder list). */
export async function listProjects(provided?: IiiClient): Promise<ProjectList> {
  const c = await client(provided)
  return parseProjectList(
    await c.call(PROJECTS_LIST_FUNCTION_ID, { version: 2 }),
  )
}

export interface ProjectWrite {
  project: ChatProject
  moved: number
}

function parseWrite(raw: unknown): ProjectWrite {
  const record = (raw ?? {}) as { project?: unknown; moved?: unknown }
  const project = parseProject(record.project)
  if (!project) throw new Error('the harness answered without a project')
  return { project, moved: num(record.moved) }
}

export async function createProject(input: {
  name: string
  color: ProjectColor
  folders: string[]
  bring_existing?: boolean
}): Promise<ProjectWrite> {
  const c = await client()
  return parseWrite(await c.call(PROJECTS_CREATE_FUNCTION_ID, { ...input }))
}

/** A patch: only the fields given change; `account`/`autocompact` are never sent. */
export async function updateProject(input: {
  id: string
  name?: string
  color?: ProjectColor
  folders?: string[]
  bring_existing?: boolean
}): Promise<ProjectWrite> {
  const c = await client()
  const payload: Record<string, unknown> = { id: input.id }
  if (input.name !== undefined) payload.name = input.name
  if (input.color !== undefined) payload.color = input.color
  if (input.folders !== undefined) payload.folders = input.folders
  if (input.bring_existing !== undefined)
    payload.bring_existing = input.bring_existing
  return parseWrite(await c.call(PROJECTS_UPDATE_FUNCTION_ID, payload))
}

export async function deleteProject(id: string): Promise<{ released: number }> {
  const c = await client()
  const res = await c.call<{ released?: unknown }>(
    PROJECTS_DELETE_FUNCTION_ID,
    { id },
  )
  return { released: num(res?.released) }
}

/**
 * Move conversations (with their subagents and sidechats, which the harness
 * follows) to a project, or out of every project with `NO_PROJECT`.
 */
export async function assignProject(
  sessionIds: string[],
  project: string,
): Promise<{ updated: number }> {
  const c = await client()
  const res = await c.call<{ updated?: unknown }>(PROJECTS_ASSIGN_FUNCTION_ID, {
    session_ids: sessionIds,
    project,
  })
  return { updated: num(res?.updated) }
}

/* ── errors ── */

export type ProjectErrorCode =
  | 'name_taken'
  | 'folder_taken'
  | 'bad_color'
  | 'too_many_folders'
  | 'folder_not_absolute'
  | 'not_found'

export interface ProjectError {
  code: string
  message?: string
  /** `folder_taken`: the project that holds the folder. */
  project_id?: string
  project_name?: string
  /** The folder the error is about, when the engine names it. */
  folder?: string
}

function texts(err: unknown, depth = 0): string[] {
  if (depth > 3 || err == null) return []
  if (typeof err === 'string') return [err]
  if (err instanceof Error) return [err.message]
  if (typeof err !== 'object') return []
  const record = err as Record<string, unknown>
  return [
    ...['message', 'error', 'reason', 'detail'].flatMap((key) =>
      typeof record[key] === 'string'
        ? [record[key] as string]
        : record[key] && typeof record[key] === 'object'
          ? texts(record[key], depth + 1)
          : [],
    ),
  ]
}

function jsonIn(text: string): Record<string, unknown> | null {
  for (
    let brace = text.indexOf('{');
    brace !== -1;
    brace = text.indexOf('{', brace + 1)
  ) {
    try {
      const parsed: unknown = JSON.parse(text.slice(brace))
      if (parsed && typeof parsed === 'object') {
        const record = parsed as Record<string, unknown>
        // A transport envelope wraps the handler's own JSON one level down.
        const inner =
          typeof record.message === 'string' ? jsonIn(record.message) : null
        return inner && typeof inner.code === 'string' ? inner : record
      }
    } catch {
      // not JSON from here; try the next brace
    }
  }
  return null
}

/**
 * The engine's JSON error (`{code, message, …}` in the error message, the
 * way tmux reports them), or the bare code when only that survived.
 */
export function parseProjectError(err: unknown): ProjectError | null {
  if (err && typeof err === 'object' && !(err instanceof Error)) {
    const record = err as Record<string, unknown>
    if (typeof record.code === 'string' && 'project_name' in record) {
      return record as unknown as ProjectError
    }
  }
  for (const text of texts(err)) {
    const parsed = jsonIn(text)
    if (parsed && typeof parsed.code === 'string') {
      return {
        code: parsed.code,
        message: str(parsed.message),
        project_id: str(parsed.project_id),
        project_name: str(parsed.project_name),
        folder: str(parsed.folder) ?? str(parsed.path),
      }
    }
  }
  const code = errorCode(err)
  return code ? { code } : null
}

/** Where a project error belongs in the dialog, with its copy (§1.3.2). */
export interface ProjectErrorDisplay {
  field: 'name' | 'folders' | 'form'
  message: string
}

export function describeProjectError(
  err: unknown,
  context: { name: string; folders: readonly string[] },
): ProjectErrorDisplay {
  const parsed = parseProjectError(err)
  switch (parsed?.code) {
    case 'name_taken':
      return {
        field: 'name',
        message: `A project named “${context.name.trim()}” already exists`,
      }
    case 'folder_taken': {
      const folder = parsed.folder ?? context.folders[0] ?? ''
      return {
        field: 'folders',
        message: parsed.project_name
          ? `${tildePath(folder)} is already in “${parsed.project_name}”`
          : (parsed.message ?? errText(err)),
      }
    }
    case 'too_many_folders':
      return {
        field: 'folders',
        message: `A project holds up to ${PROJECT_FOLDERS_MAX} folders`,
      }
    case 'folder_not_absolute':
      return {
        field: 'folders',
        message: parsed.folder
          ? `${parsed.folder} is not an absolute path`
          : 'Folders need an absolute path',
      }
    case 'bad_color':
      return { field: 'form', message: 'Pick one of the six colours' }
    case 'not_found':
      return { field: 'form', message: 'This project no longer exists' }
    default:
      return { field: 'form', message: parsed?.message ?? errText(err) }
  }
}

/* ── chat settings ── */

export interface ChatSettings {
  descriptions: { agent_writes: boolean }
  /** Other keys (phase 3's `autocompact`) pass through untouched. */
  [key: string]: unknown
}

export function parseChatSettings(raw: unknown): ChatSettings {
  const record = (raw && typeof raw === 'object' ? raw : {}) as Record<
    string,
    unknown
  >
  const settings =
    record.settings && typeof record.settings === 'object'
      ? (record.settings as Record<string, unknown>)
      : record
  const descriptions = (settings.descriptions ?? {}) as {
    agent_writes?: unknown
  }
  return {
    ...settings,
    descriptions: { agent_writes: descriptions.agent_writes !== false },
  }
}

export async function getChatSettings(
  provided?: IiiClient,
): Promise<ChatSettings> {
  const c = await client(provided)
  return parseChatSettings(await c.call(CHAT_SETTINGS_GET_FUNCTION_ID, {}))
}

/** The server reads, merges `patch` and writes; the answer is the result. */
export async function setChatSettings(
  patch: Record<string, unknown>,
): Promise<ChatSettings> {
  const c = await client()
  return parseChatSettings(
    await c.call(CHAT_SETTINGS_SET_FUNCTION_ID, { patch }),
  )
}
