/**
 * The harness's catalogue of saved folders (one path, a name, `last_used_at`):
 * the folder pickers, the new-chat dialog and the default folder read it.
 *
 * chat-projects D25: the catalogue moved from `harness::projects::*` to
 * `harness::folders::*` with the same shapes, and `harness::projects` now
 * means the Chat's projects (`./projects.ts`). An engine whose harness
 * predates the move still answers the old ids, so every call here uses
 * `harness::folders::*` when the engine has it and the old ids otherwise.
 */
import {
  getChatCapabilities,
  refreshChatCapabilities,
} from '@/lib/chat-capabilities'
import { isFunctionNotFound } from '@/lib/errors'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'

export const HARNESS_FOLDERS_LIST_FUNCTION_ID = 'harness::folders::list'
export const HARNESS_FOLDERS_UPSERT_FUNCTION_ID = 'harness::folders::upsert'
export const HARNESS_FOLDERS_DELETE_FUNCTION_ID = 'harness::folders::delete'

/** The folder catalogue's ids on a harness from before D25. */
export const LEGACY_FOLDERS_LIST_FUNCTION_ID = 'harness::projects::list'
export const LEGACY_FOLDERS_UPSERT_FUNCTION_ID = 'harness::projects::upsert'
export const LEGACY_FOLDERS_DELETE_FUNCTION_ID = 'harness::projects::delete'

export interface SavedFolder {
  path: string
  name: string
  last_used_at: number
}

interface FolderListResponse {
  folders?: SavedFolder[]
  /** The old catalogue's (and the D26 shim's) key. */
  projects?: SavedFolder[]
}

interface FolderUpsertResponse {
  folder?: SavedFolder
  project?: SavedFolder
}

interface FolderDeleteResponse {
  deleted: boolean
}

let cachedFolders: SavedFolder[] = []

function byMostRecent(folders: SavedFolder[]): SavedFolder[] {
  return [...folders].sort((a, b) => b.last_used_at - a.last_used_at)
}

/** Synchronous compatibility view for injected UI's existing Host contract. */
export function recentFolderPaths(): string[] {
  return cachedFolders.map((folder) => folder.path)
}

/** Whether to call `harness::folders::*` (else the pre-D25 ids). */
async function shouldUseFolderIds(): Promise<boolean> {
  const caps = getChatCapabilities()
  if (caps.loaded) return caps.folders
  // `harness::folders::*` is internal (D37): the console's capability read
  // (with internal functions) decides, not the default catalogue. Unread,
  // the new id is tried and a missing one falls back below.
  const read = await refreshChatCapabilities()
  return read.loaded ? read.folders : true
}

/**
 * Call the new id when the engine has it, and the old one otherwise. A new
 * id that turns out to be missing (a catalogue read that failed open, a
 * harness downgraded under us) falls back to the old one once.
 */
async function callCatalog<T>(
  client: IiiClient,
  ids: { current: string; legacy: string },
  payload: Record<string, unknown>,
): Promise<T> {
  if (await shouldUseFolderIds()) {
    try {
      return await client.call<T>(ids.current, payload)
    } catch (err) {
      if (!isFunctionNotFound(err)) throw err
    }
  }
  return client.call<T>(ids.legacy, payload)
}

export async function listSavedFolders(
  providedClient?: IiiClient,
): Promise<SavedFolder[]> {
  const client = providedClient ?? (await getIiiClient())
  const response = await callCatalog<FolderListResponse>(
    client,
    {
      current: HARNESS_FOLDERS_LIST_FUNCTION_ID,
      legacy: LEGACY_FOLDERS_LIST_FUNCTION_ID,
    },
    {},
  )
  cachedFolders = byMostRecent(
    response?.folders ?? response?.projects ?? [],
  ).filter((folder) => typeof folder?.path === 'string' && folder.path)
  return cachedFolders
}

/** Omit `name` to preserve a custom name, or pass blank to reset to basename. */
export async function upsertSavedFolder(
  path: string,
  name?: string,
): Promise<SavedFolder> {
  const client = await getIiiClient()
  const response = await callCatalog<FolderUpsertResponse>(
    client,
    {
      current: HARNESS_FOLDERS_UPSERT_FUNCTION_ID,
      legacy: LEGACY_FOLDERS_UPSERT_FUNCTION_ID,
    },
    name === undefined ? { path } : { path, name },
  )
  const folder = response.folder ?? response.project
  if (!folder) throw new Error('the folder catalogue answered without a folder')
  cachedFolders = byMostRecent([
    folder,
    ...cachedFolders.filter((entry) => entry.path !== folder.path),
  ])
  return folder
}

export async function deleteSavedFolder(path: string): Promise<boolean> {
  const client = await getIiiClient()
  const response = await callCatalog<FolderDeleteResponse>(
    client,
    {
      current: HARNESS_FOLDERS_DELETE_FUNCTION_ID,
      legacy: LEGACY_FOLDERS_DELETE_FUNCTION_ID,
    },
    { path },
  )
  cachedFolders = cachedFolders.filter((folder) => folder.path !== path)
  return response.deleted
}
