import { getIiiClient, type IiiClient } from '@/lib/iii-client'

export const HARNESS_PROJECTS_LIST_FUNCTION_ID = 'harness::projects::list'
export const HARNESS_PROJECTS_UPSERT_FUNCTION_ID = 'harness::projects::upsert'
export const HARNESS_PROJECTS_DELETE_FUNCTION_ID = 'harness::projects::delete'

export interface HarnessProject {
  path: string
  name: string
  last_used_at: number
}

interface ProjectListResponse {
  projects?: HarnessProject[]
}

interface ProjectUpsertResponse {
  project: HarnessProject
}

interface ProjectDeleteResponse {
  deleted: boolean
}

let cachedProjects: HarnessProject[] = []

function byMostRecent(projects: HarnessProject[]): HarnessProject[] {
  return [...projects].sort((a, b) => b.last_used_at - a.last_used_at)
}

/** Synchronous compatibility view for injected UI's existing Host contract. */
export function recentHarnessProjectPaths(): string[] {
  return cachedProjects.map((project) => project.path)
}

export async function listHarnessProjects(
  providedClient?: IiiClient,
): Promise<HarnessProject[]> {
  const client = providedClient ?? (await getIiiClient())
  const response = await client.trigger<ProjectListResponse>(
    HARNESS_PROJECTS_LIST_FUNCTION_ID,
    {},
  )
  cachedProjects = byMostRecent(response?.projects ?? [])
  return cachedProjects
}

/** Omit `name` to preserve a custom name, or pass blank to reset to basename. */
export async function upsertHarnessProject(
  path: string,
  name?: string,
): Promise<HarnessProject> {
  const client = await getIiiClient()
  const response = await client.trigger<ProjectUpsertResponse>(
    HARNESS_PROJECTS_UPSERT_FUNCTION_ID,
    name === undefined ? { path } : { path, name },
  )
  cachedProjects = byMostRecent([
    response.project,
    ...cachedProjects.filter(
      (project) => project.path !== response.project.path,
    ),
  ])
  return response.project
}

export async function deleteHarnessProject(path: string): Promise<boolean> {
  const client = await getIiiClient()
  const response = await client.trigger<ProjectDeleteResponse>(
    HARNESS_PROJECTS_DELETE_FUNCTION_ID,
    { path },
  )
  cachedProjects = cachedProjects.filter((project) => project.path !== path)
  return response.deleted
}
