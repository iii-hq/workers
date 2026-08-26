/**
 * Zod schemas for the `directory::*` namespace — moved out of the console
 * SPA (formerly console/web/src/components/chat/directory/parsers.ts) into
 * the worker's own injected UI.
 *
 * Wire source: `iii-directory/src/functions/*.rs`
 *   - skills.rs        — directory::skills::list / get / index
 *   - download.rs      — directory::skills::download (+ _from_registry / _from_repo)
 *   - update.rs        — directory::skills::update / create and
 *                        directory::system-prompts::* updates
 *   - registry.rs      — directory::registry::workers::list / info
 *
 * The `::delete` ids (skills, system-prompts) are deliberately
 * absent: their `{ name }` / `{ id }` in/out shape has no dedicated view,
 * and the console's generic JSON card is the right rendering for a delete
 * confirmation.
 */
import { z } from 'zod'
import { unwrapEnvelope } from '../lib/envelope'

export const DIRECTORY_FUNCTION_IDS = [
  'directory::skills::list',
  'directory::skills::get',
  'directory::skills::index',
  'directory::skills::download',
  'directory::skills::download_from_registry',
  'directory::skills::download_from_repo',
  'directory::skills::update',
  'directory::skills::create',
  'directory::system-prompts::list',
  'directory::system-prompts::get',
  'directory::system-prompts::update',
  'directory::system-prompts::create',
  'directory::agents::list',
  'directory::agents::get',
  'directory::agents::update',
  'directory::agents::create',
  'directory::registry::workers::list',
  'directory::registry::workers::info',
] as const

export type DirectoryFunctionId = (typeof DIRECTORY_FUNCTION_IDS)[number]

const DIRECTORY_FUNCTION_ID_SET: ReadonlySet<string> = new Set<string>(
  DIRECTORY_FUNCTION_IDS,
)

export function isDirectoryFunction(id: string): id is DirectoryFunctionId {
  return DIRECTORY_FUNCTION_ID_SET.has(id)
}

/* ---------------- skills::list ---------------- */

export const skillsListRequestSchema = z.object({
  search: z.string().optional(),
  prefix: z.string().optional(),
  type: z.string().optional(),
  include_description: z.boolean().optional(),
})
export type SkillsListRequest = z.infer<typeof skillsListRequestSchema>

export const skillEntrySchema = z.object({
  id: z.string(),
  title: z.string(),
  type: z.string().nullable().optional(),
  function_id: z.string().nullable().optional(),
  description: z.string(),
  bytes: z.number(),
  modified_at: z.string(),
})
export type SkillEntry = z.infer<typeof skillEntrySchema>

export const skillsListResponseSchema = z.object({
  skills: z.array(skillEntrySchema),
})
export type SkillsListResponse = z.infer<typeof skillsListResponseSchema>

/* ---------------- skills::get ---------------- */

export const skillsGetRequestSchema = z.object({
  id: z.string(),
  raw: z.boolean().optional(),
})
export type SkillsGetRequest = z.infer<typeof skillsGetRequestSchema>

export const skillsGetResponseSchema = z.object({
  id: z.string(),
  title: z.string(),
  type: z.string().nullable().optional(),
  function_id: z.string().nullable().optional(),
  path: z.string().optional(),
  body: z.string(),
  raw: z.string().nullable().optional(),
  modified_at: z.string(),
})
export type SkillsGetResponse = z.infer<typeof skillsGetResponseSchema>

/* ---------------- skills::index ---------------- */

export const skillsIndexRequestSchema = z.object({})
export type SkillsIndexRequest = z.infer<typeof skillsIndexRequestSchema>

export const skillsIndexResponseSchema = z.object({
  body: z.string(),
  workers_count: z.number(),
})
export type SkillsIndexResponse = z.infer<typeof skillsIndexResponseSchema>

/* ---------------- skills::download (all three variants) ---------------- */

export const skillsDownloadRequestSchema = z.object({
  repo: z.string().nullable().optional(),
  skill: z.string().nullable().optional(),
  branch: z.string().nullable().optional(),
  worker: z.string().nullable().optional(),
  version: z.string().nullable().optional(),
  tag: z.string().nullable().optional(),
})
export type SkillsDownloadRequest = z.infer<typeof skillsDownloadRequestSchema>

export const skillsDownloadResponseSchema = z.object({
  namespace: z.string(),
  skills_written: z.array(z.string()),
  system_prompts_written: z.array(z.string()),
  agents_written: z.array(z.string()),
  source: z.unknown(),
})
export type SkillsDownloadResponse = z.infer<
  typeof skillsDownloadResponseSchema
>

/* ---------------- skills::update ---------------- */

export const skillsUpdateRequestSchema = z.object({
  id: z.string(),
  content: z.string(),
})
export type SkillsUpdateRequest = z.infer<typeof skillsUpdateRequestSchema>

export const skillsUpdateResponseSchema = z.object({
  id: z.string(),
  title: z.string(),
  type: z.string().nullable().optional(),
  function_id: z.string().nullable().optional(),
  bytes: z.number(),
  modified_at: z.string(),
})
export type SkillsUpdateResponse = z.infer<typeof skillsUpdateResponseSchema>

/* ---------------- system-prompts::list ---------------- */

export const systemPromptsListRequestSchema = z.object({})
export type SystemPromptsListRequest = z.infer<
  typeof systemPromptsListRequestSchema
>

export const systemPromptEntrySchema = z.object({
  name: z.string(),
  description: z.string(),
  modified_at: z.string(),
})
export type SystemPromptEntry = z.infer<typeof systemPromptEntrySchema>

export const systemPromptsListResponseSchema = z.object({
  prompts: z.array(systemPromptEntrySchema),
})
export type SystemPromptsListResponse = z.infer<
  typeof systemPromptsListResponseSchema
>

/* ---------------- system-prompts::get ---------------- */

export const systemPromptsGetRequestSchema = z.object({
  name: z.string(),
  raw: z.boolean().optional(),
})
export type SystemPromptsGetRequest = z.infer<typeof systemPromptsGetRequestSchema>

export const systemPromptsGetResponseSchema = z.object({
  name: z.string(),
  description: z.string(),
  body: z.string(),
  raw: z.string().nullable().optional(),
  modified_at: z.string(),
})
export type SystemPromptsGetResponse = z.infer<
  typeof systemPromptsGetResponseSchema
>

/* ---------------- system-prompts::update ---------------- */

export const systemPromptsUpdateRequestSchema = z.object({
  name: z.string(),
  content: z.string(),
})
export type SystemPromptsUpdateRequest = z.infer<
  typeof systemPromptsUpdateRequestSchema
>

export const systemPromptsUpdateResponseSchema = z.object({
  name: z.string(),
  description: z.string(),
  bytes: z.number(),
  modified_at: z.string(),
})
export type SystemPromptsUpdateResponse = z.infer<
  typeof systemPromptsUpdateResponseSchema
>

/* ---------------- agents::list ---------------- */

export const agentEntrySchema = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string(),
  logo: z.string().nullable().optional(),
  skill_count: z.number().nullable().optional(),
  model: z.string().nullable().optional(),
  icon: z.string().nullable().optional(),
  leaf: z.boolean().optional(),
  modified_at: z.string(),
})
export type AgentEntry = z.infer<typeof agentEntrySchema>

export const agentsListResponseSchema = z.object({
  agents: z.array(agentEntrySchema),
})
export type AgentsListResponse = z.infer<typeof agentsListResponseSchema>

/* ---------------- agents::get ---------------- */

export const agentsGetRequestSchema = z.object({
  id: z.string(),
  raw: z.boolean().optional(),
})
export type AgentsGetRequest = z.infer<typeof agentsGetRequestSchema>

export const agentsGetResponseSchema = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string(),
  logo: z.string().nullable().optional(),
  system_prompt: z.string(),
  skills: z.array(z.string()),
  unknown_skills: z.array(z.string()),
  delegates_to: z.array(z.string()).nullable().optional(),
  leaf: z.boolean().optional(),
  unknown_delegates: z.array(z.string()).optional(),
  model: z.string().nullable().optional(),
  icon: z.string().nullable().optional(),
  raw: z.string().nullable().optional(),
  modified_at: z.string(),
})
export type AgentsGetResponse = z.infer<typeof agentsGetResponseSchema>

/* ---------------- agents::update / create ---------------- */

export const agentsUpdateRequestSchema = z.object({
  id: z.string(),
  content: z.string(),
})
export type AgentsUpdateRequest = z.infer<typeof agentsUpdateRequestSchema>

export const agentsUpdateResponseSchema = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string(),
  logo: z.string().nullable().optional(),
  bytes: z.number(),
  modified_at: z.string(),
})
export type AgentsUpdateResponse = z.infer<typeof agentsUpdateResponseSchema>

/* ---------------- registry::workers::list ---------------- */

export const registryWorkersListRequestSchema = z.object({
  search: z.string().optional(),
  cursor: z.string().nullable().optional(),
})
export type RegistryWorkersListRequest = z.infer<
  typeof registryWorkersListRequestSchema
>

export const registryWorkerAuthorSchema = z.object({
  name: z.string().nullable().optional(),
  pfp: z.string().nullable().optional(),
  verified: z.boolean().optional(),
})
export type RegistryWorkerAuthor = z.infer<typeof registryWorkerAuthorSchema>

export const registryDependencySchema = z.object({
  name: z.string(),
  version: z.string(),
})
export type RegistryDependency = z.infer<typeof registryDependencySchema>

export const registryWorkerSchema = z.object({
  name: z.string(),
  description: z.string().nullable().optional(),
  type: z.string().nullable().optional(),
  version: z.string().nullable().optional(),
  repo: z.string().nullable().optional(),
  config: z.unknown().optional(),
  supported_targets: z.array(z.string()).optional(),
  image: z.string().nullable().optional(),
  total_downloads: z.number().optional(),
  dependencies: z.array(registryDependencySchema).optional(),
  author: registryWorkerAuthorSchema.nullable().optional(),
})
export type RegistryWorker = z.infer<typeof registryWorkerSchema>

export const registryPaginationSchema = z.object({
  next_cursor: z.string().nullable().optional(),
  has_more: z.boolean().optional(),
  page_size: z.number().optional(),
})
export type RegistryPagination = z.infer<typeof registryPaginationSchema>

export const registryWorkersListResponseSchema = z.object({
  workers: z.array(registryWorkerSchema),
  pagination: registryPaginationSchema,
})
export type RegistryWorkersListResponse = z.infer<
  typeof registryWorkersListResponseSchema
>

/* ---------------- registry::workers::info ---------------- */

export const registryWorkerInfoRequestSchema = z.object({
  name: z.string(),
  version: z.string().nullable().optional(),
  tag: z.string().nullable().optional(),
})
export type RegistryWorkerInfoRequest = z.infer<
  typeof registryWorkerInfoRequestSchema
>

export const apiReferenceFunctionSchema = z.object({
  name: z.string(),
  description: z.string().nullable().optional(),
  request_schema: z.unknown().nullable().optional(),
  response_schema: z.unknown().nullable().optional(),
  metadata: z.unknown().nullable().optional(),
})
export type ApiReferenceFunction = z.infer<typeof apiReferenceFunctionSchema>

export const apiReferenceTriggerSchema = z.object({
  name: z.string(),
  description: z.string().nullable().optional(),
  invocation_schema: z.unknown().nullable().optional(),
  return_schema: z.unknown().nullable().optional(),
  metadata: z.unknown().nullable().optional(),
})
export type ApiReferenceTrigger = z.infer<typeof apiReferenceTriggerSchema>

export const apiReferenceSchema = z.object({
  functions: z.array(apiReferenceFunctionSchema).optional(),
  triggers: z.array(apiReferenceTriggerSchema).optional(),
})
export type ApiReferenceShape = z.infer<typeof apiReferenceSchema>

export const skillsTreeSkillSchema = z.object({
  path: z.string(),
})
export const skillsTreeSchema = z.object({
  skills: z.array(skillsTreeSkillSchema).optional(),
})
export type SkillsTreeShape = z.infer<typeof skillsTreeSchema>

export const registryWorkerInfoResponseSchema = z.object({
  worker: registryWorkerSchema,
  readme: z.string().nullable().optional(),
  api_reference: apiReferenceSchema,
  skills_tree: skillsTreeSchema,
})
export type RegistryWorkerInfoResponse = z.infer<
  typeof registryWorkerInfoResponseSchema
>

/* ---------------- generic helpers ---------------- */

export function safeParseRequest<T>(
  schema: z.ZodType<T>,
  value: unknown,
): T | null {
  const parsed = schema.safeParse(value ?? {})
  return parsed.success ? parsed.data : null
}

export function safeParseResponse<T>(
  schema: z.ZodType<T>,
  value: unknown,
): T | null {
  const parsed = schema.safeParse(unwrapEnvelope(value))
  return parsed.success ? parsed.data : null
}
