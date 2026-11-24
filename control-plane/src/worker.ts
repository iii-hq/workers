import { existsSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import type { IIIClient } from 'iii-sdk'
import { z } from 'zod'
import { configSchema, defaultConfig } from './config.js'
import { sourceHash, ControlPlaneReadModel } from './read-model.js'
import { ControllerSessions } from './controller.js'
import { scopeDigest } from './digest.js'
import { resolveSummary, SUMMARY_MAX_AGE, summaryIsFresh, withAgentChecks } from './summary.js'
import type { ControlPlaneClient } from './types.js'
import { ControlPlaneWriter } from './writer.js'

const scopeInput = z.strictObject({ scope: z.string().min(1) })
const noInput = z.strictObject({})
const decidedInput = scopeInput.extend({ cursor: z.string().optional() })
const assumptionsInput = scopeInput.extend({ since: z.iso.datetime().optional() })
const request = (schema: z.ZodType) => ({ ...z.toJSONSchema(schema), type: 'object' as const })
function parseRequest<T>(schema: z.ZodType<T>, input: unknown): T {
  if (!input || typeof input !== 'object' || Array.isArray(input)) return schema.parse(input)
  const payload = { ...input } as Record<string, unknown>
  delete payload._caller_worker_id
  return schema.parse(payload)
}
const array = { type: 'array', items: { type: 'object' } }
const object = (properties: Record<string, unknown>, required = Object.keys(properties)) => ({
  type: 'object' as const,
  properties,
  required,
})
/** Internal trigger handlers: an engine event in, nothing out. Typed, so the release interface check accepts them. */
const eventHandler = {
  request_format: { type: 'object' as const, description: 'The engine event payload.' },
  response_format: object({}),
}

export async function registerControlPlane(iii: IIIClient, uiDirs: string[] = []) {
  const configurationId = process.env.III_CONFIG_NAME?.trim() || 'control-plane'
  const client: ControlPlaneClient = {
    call: <T>(id: string, input?: unknown) =>
      iii.call<unknown, T>({ function_id: id, payload: input ?? {} }),
  }
  let config = defaultConfig()
  const emit = (scope: string, as_of: string, parts: string[]) =>
    client.call('control-plane::on-changed', { scope, as_of, parts }).then(() => {})
  const read = new ControlPlaneReadModel(
    client,
    () => config,
    async (scope, asOf, parts) => {
      armWriterTimer()
      await emit(scope, asOf, parts)
    },
  )
  const projectIds = async () =>
    (
      await client.call<{ projects: Array<{ id: string }> }>('kanban::project::list', {
        include_archived: true,
      })
    ).projects.map(({ id }) => id)
  const scoped = async <T extends { scope: string }>(schema: z.ZodType<T>, input: unknown) => {
    const payload = parseRequest(schema, input)
    if (payload.scope !== 'all' && !(await projectIds()).includes(payload.scope)) {
      throw new Error('unknown Control plane project scope')
    }
    return payload
  }
  let availability = { github: false }
  const summaryDeadlines = new Map<string, ReturnType<typeof setTimeout>>()
  const writer = new ControlPlaneWriter(
    client,
    () => config,
    async (scope) => {
      const data = await read.read(scope)
      // Unread pull requests are not "no pull requests": the writer then gets none.
      if (availability.github && read.prsRead(scope)) return data
      const available = {
        ...data,
        prs: [],
        repositories: [],
        errors: { ...data.errors, prs: null },
      }
      available.sourceHash = sourceHash(available)
      return available
    },
    (scope) => {
      armSummaryDeadline(scope)
      armWriterTimer()
      return emit(scope, new Date().toISOString(), ['summary'])
    },
  )
  let writerReady = false
  let writerRunning = false
  let writerTimer: ReturnType<typeof setTimeout> | undefined
  function armWriterTimer(): void {
    if (!writerReady || writerRunning) return
    clearTimeout(writerTimer)
    const scopes = read.activeScopes()
    if (!config.summary.model || !scopes.length) return
    const due = Math.min(...scopes.map((scope) => writer.nextCheckAt(scope)))
    writerTimer = setTimeout(
      async () => {
        writerRunning = true
        try {
          const now = Date.now()
          await Promise.all(
            read
              .activeScopes()
              .filter((scope) => writer.nextCheckAt(scope) <= now)
              .map((scope) =>
                writer
                  .check(scope)
                  .catch((error) => console.error('control-plane schedule:', error)),
              ),
          )
        } finally {
          writerRunning = false
          armWriterTimer()
        }
      },
      Math.max(100, Math.min(60_000, due - Date.now())),
    )
    writerTimer.unref()
  }

  function armSummaryDeadline(scope: string): void {
    clearTimeout(summaryDeadlines.get(scope))
    summaryDeadlines.delete(scope)
    const snapshot = writer.snapshot(scope)
    const remaining = snapshot.checked_at
      ? Date.parse(snapshot.checked_at) + SUMMARY_MAX_AGE + 1 - Date.now()
      : 0
    if (snapshot.error || remaining <= 0) return
    const deadline = setTimeout(() => {
      summaryDeadlines.delete(scope)
      void emit(scope, new Date().toISOString(), ['summary']).catch((error) =>
        console.error('control-plane summary deadline:', error),
      )
    }, remaining)
    deadline.unref?.()
    summaryDeadlines.set(scope, deadline)
  }
  const refreshAvailability = async () => {
    const catalog = await client.call<{ functions: Array<{ function_id: string }> }>(
      'engine::functions::list',
      { include_internal: true },
    )
    const ids = new Set(catalog.functions.map((fn) => fn.function_id))
    availability = {
      github: ['github::pr::list', 'github::pr::checks', 'github::pr::view'].every((id) =>
        ids.has(id),
      ),
    }
  }

  iii.registerFunction(
    'control-plane::on-changed',
    ({ scope, as_of, parts }: { scope: string; as_of: string; parts: string[] }) => ({
      scope,
      as_of,
      parts,
    }),
    {
      description:
        'A cached Control plane view or summary changed. Refetch the named parts for this scope.',
      private: true,
      request_format: object({
        scope: { type: 'string' },
        as_of: { type: 'string', format: 'date-time' },
        parts: { type: 'array', items: { type: 'string' } },
      }),
      response_format: object({
        scope: { type: 'string' },
        as_of: { type: 'string', format: 'date-time' },
        parts: { type: 'array', items: { type: 'string' } },
      }),
    },
  )
  const summary = (scope: string) => {
    const snapshot = writer.snapshot(scope)
    return {
      ...snapshot,
      fresh:
        !snapshot.error &&
        snapshot.checked_at != null &&
        summaryIsFresh(
          typeof snapshot.checked_at === 'number'
            ? snapshot.checked_at
            : Date.parse(snapshot.checked_at),
        ),
    }
  }
  iii.registerFunction(
    'control-plane::overview',
    async (input: unknown) => {
      const { scope } = await scoped(scopeInput, input)
      const read_ = await read.read(scope)
      await writer.load(scope)
      armSummaryDeadline(scope)
      const current = summary(scope)
      const result = current.result ? resolveSummary(current.result, read_) : null
      const data = { ...read_, items: withAgentChecks(read_.items, result, read_) }
      const now = data.items.filter((item) => item.urgency === 'now')
      return {
        ...data,
        summary: { ...current, result },
        availability,
        prs_interval_min: config.prs.watched_interval_min,
        tiles: {
          needs_you: data.items.length,
          now: now.length,
          working: data.runs.filter((run) => run.state === 'working').length,
          waiting: data.runs.filter((run) => run.state === 'waiting' || run.state === 'stalled')
            .length,
          open_prs: data.prs.length,
          failing_prs: data.prs.filter((pr) => pr.checks === 'failing').length,
          active_releases: data.releaseCatalog.filter((release) => release.status === 'active')
            .length,
        },
        top_now: now.slice(0, 3),
        in_progress: data.runs.filter((run) => run.state === 'working'),
      }
    },
    {
      description:
        'Read the project portrait: factual tiles, questions, runs, PRs, releases, board and last valid summary.',
      request_format: request(scopeInput),
      response_format: object({
        as_of: { type: 'string' },
        tiles: { type: 'object' },
        items: array,
        runs: array,
        prs: array,
        questions: array,
        releases: array,
        board: { type: ['object', 'null'] },
        summary: { type: 'object' },
        availability: { type: 'object' },
        prs_interval_min: { type: 'number' },
      }),
    },
  )
  iii.registerFunction(
    'control-plane::needs-you::list',
    async (input: unknown) => {
      const { scope } = await scoped(scopeInput, input)
      const read_ = await read.read(scope)
      await writer.load(scope)
      const result = summary(scope).result
      const data = {
        ...read_,
        items: withAgentChecks(read_.items, result ? resolveSummary(result, read_) : null, read_),
      }
      return {
        items: data.items,
        now: data.items.filter((item) => item.urgency === 'now'),
        no_rush: data.items.filter((item) => item.urgency === 'no-rush'),
        as_of: data.as_of,
      }
    },
    {
      description:
        'Read what waits on the person: questions, stalled runs, cards marked for a person, chats ending on a question, and decisions the summary agent found; each with the agent check and recommendation when the writer has one. Ordered by urgency, cards blocked and age.',
      request_format: request(scopeInput),
      response_format: object({
        items: array,
        now: array,
        no_rush: array,
        as_of: { type: 'string' },
      }),
    },
  )
  for (const [id, field] of [
    ['control-plane::runs::list', 'runs'],
    ['control-plane::prs::list', 'prs'],
  ] as const)
    iii.registerFunction(
      id,
      async (input: unknown) => {
        const data = await read.read((await scoped(scopeInput, input)).scope)
        return {
          [field]: data[field],
          as_of: data.as_of,
          error: data.errors[field === 'runs' ? 'sessions' : 'prs'],
        }
      },
      {
        description:
          field === 'runs'
            ? 'Read root runs and their context_size in estimated tokens.'
            : 'Read open PRs from Project.repos with per-PR as_of and deterministic wait reasons.',
        request_format: request(scopeInput),
        response_format: object({
          [field]: array,
          as_of: { type: 'string' },
          error: { type: ['string', 'null'] },
        }),
      },
    )
  iii.registerFunction(
    'control-plane::decided::list',
    async (input: unknown) => {
      const { scope, cursor } = await scoped(decidedInput, input)
      const data = await read.read(scope)
      const all = data.questions.filter(
        (question) =>
          question.board && !['open', 'answering', 'expired', 'recorded'].includes(question.state),
      )
      const start = cursor ? all.findIndex((question) => question.key === cursor) + 1 : 0
      if (cursor && start === 0) throw new Error('invalid decided cursor')
      const questions = all.slice(start, start + 200)
      return {
        questions,
        ...(start + questions.length < all.length ? { cursor: questions.at(-1)?.key } : {}),
        median_decision_wait_ms: data.median_decision_wait_ms,
        as_of: data.as_of,
      }
    },
    {
      description:
        'Read the board decision log and median decision wait over seven days, with an opaque next cursor.',
      request_format: request(decidedInput),
      response_format: object(
        {
          questions: array,
          cursor: { type: 'string' },
          median_decision_wait_ms: { type: ['number', 'null'] },
          as_of: { type: 'string' },
        },
        ['questions', 'median_decision_wait_ms', 'as_of'],
      ),
    },
  )
  iii.registerFunction(
    'control-plane::assumptions::list',
    async (input: unknown) => {
      const { scope, since } = await scoped(assumptionsInput, input)
      const data = await read.read(scope)
      return {
        questions: data.questions.filter(
          (question) =>
            question.state === 'recorded' &&
            question.board &&
            (!since || Date.parse(question.board.comment.created_at) > Date.parse(since)),
        ),
        as_of: data.as_of,
      }
    },
    {
      description:
        'Read recorded assumptions, filtering since against the board comment creation time.',
      request_format: request(assumptionsInput),
      response_format: object({ questions: array, as_of: { type: 'string' } }),
    },
  )
  iii.registerFunction(
    'control-plane::watch',
    async (input: unknown) => {
      const { scope } = await scoped(scopeInput, input)
      const result = await read.watch(scope)
      armWriterTimer()
      return result
    },
    {
      description:
        'Mark a scope watched for two minutes and immediately refresh stale repository reads.',
      request_format: request(scopeInput),
      response_format: object({ watched_until: { type: 'string', format: 'date-time' } }),
    },
  )
  iii.registerFunction(
    'control-plane::summary::refresh',
    async (input: unknown) => {
      const { scope } = await scoped(scopeInput, input)
      const result = await writer.refresh(scope)
      armWriterTimer()
      return result
    },
    {
      description:
        'Request a read-only writer turn now. If one is already running for the scope, join it (writing: true) instead of starting another.',
      request_format: request(scopeInput),
      response_format: object(
        {
          result: { type: ['object', 'null'] },
          valid_hash: { type: ['string', 'null'] },
          checked_at: { type: ['string', 'null'] },
          error: { type: ['string', 'null'] },
          unavailable: { type: 'boolean' },
          writing: { type: 'boolean' },
        },
        ['result', 'valid_hash', 'checked_at', 'error', 'unavailable', 'writing'],
      ),
    },
  )
  iii.registerFunction(
    'control-plane::source-changed',
    ({ input }: { input?: unknown }) => {
      read.onSource(input)
      return {}
    },
    { ...eventHandler, metadata: { internal: true } },
  )
  iii.registerFunction(
    'control-plane::turn-started',
    ({ input }: { input?: unknown }) => {
      read.onSource(input, true)
      return {}
    },
    { ...eventHandler, metadata: { internal: true } },
  )
  iii.registerFunction(
    'control-plane::turn-completed',
    async ({ input }: { input?: unknown }) => {
      read.onSource(input, true)
      await writer.onCompleted(input)
      return {}
    },
    { ...eventHandler, metadata: { internal: true } },
  )
  const controllers = new ControllerSessions({
    iii: client,
    config: () => config,
    read: (scope) => read.read(scope),
  })
  iii.registerFunction(
    'control-plane::controller::open',
    async (input: unknown) => controllers.open((await scoped(scopeInput, input)).scope),
    {
      description:
        "Open the scope's controller chat: one per project (and one for All projects) that reads everything and acts for the person after they confirm, with every function an agent may call. Creates the session on first use (and after the person deleted it) and returns its id for the console chat.",
      request_format: request(scopeInput),
      response_format: object(
        {
          session_id: { type: 'string' },
          created: { type: 'boolean' },
          thinking_level: { type: 'string' },
        },
        ['session_id', 'created'],
      ),
    },
  )
  iii.registerFunction(
    'control-plane::digest',
    async (input: unknown) => {
      const { scope } = await scoped(scopeInput, input)
      return scopeDigest(client, await read.read(scope), scope)
    },
    {
      description:
        "Read one scope's bounded digest: board counts, releases, active and changed cards, the last day of activity, runs with their last message, what needs the person with its evidence, and open PRs. Start here instead of listing the board.",
      request_format: request(scopeInput),
      response_format: { type: 'object' },
    },
  )
  iii.registerFunction(
    'control-plane::configuration-id',
    (input: unknown) => {
      parseRequest(noInput, input)
      return { id: configurationId }
    },
    {
      description: 'Read the configuration id for this Control plane worker.',
      request_format: request(noInput),
      response_format: object({ id: { type: 'string' } }),
    },
  )
  iii.registerFunction(
    'control-plane::config-updated',
    async () => {
      const entry = await client.call<{ value: unknown }>('configuration::get', {
        id: configurationId,
      })
      config = configSchema.parse(entry.value)
      armWriterTimer()
      await read.tick()
      for (const scope of read.knownScopes())
        await emit(scope, new Date().toISOString(), ['summary', 'configuration'])
      return {}
    },
    { ...eventHandler, metadata: { internal: true } },
  )
  iii.registerFunction(
    'control-plane::availability-updated',
    async () => {
      const previous = availability.github
      await refreshAvailability()
      if (previous !== availability.github) {
        read.invalidatePrs()
        await read.tick()
        for (const scope of read.knownScopes())
          await emit(scope, new Date().toISOString(), ['availability', 'summary'])
      }
      return {}
    },
    { ...eventHandler, metadata: { internal: true } },
  )
  const sources = [
    ...['harness::on-ask', 'harness::on-ask-settled', 'kanban::on-change'].map(
      (function_id) => [{ function_id }, 'control-plane::source-changed'] as const,
    ),
    [{ function_id: 'harness::on-turn-started' }, 'control-plane::turn-started'],
    [{ function_id: 'harness::on-turn-completed' }, 'control-plane::turn-completed'],
    [
      {
        function_id: 'configuration::on-updated',
        filters: { 'input.id': { $eq: configurationId } },
      },
      'control-plane::config-updated',
    ],
    [{ function_id: 'engine::on-functions-changed' }, 'control-plane::availability-updated'],
  ] as const
  const observers = sources.map(([source, target]) =>
    iii.observe({ ...source, capture: ['input'] }, target),
  )
  await Promise.all(
    observers.map(async (observer, index) => {
      try {
        await observer.ready()
      } catch (cause) {
        throw new Error(
          `Control plane observer registration failed for ${sources[index][0].function_id}`,
          {
            cause,
          },
        )
      }
    }),
  )
  await client.call('configuration::ensure', {
    id: configurationId,
    name: 'Control plane',
    description: 'Control plane read model and summary writer.',
    // The input schema: a key with a default is optional, so a value saved before
    // the key existed still validates (the output schema would require it).
    schema: z.toJSONSchema(configSchema, { io: 'input' }),
    initial_value: defaultConfig(),
  })
  const entry = await client.call<{ value: unknown }>('configuration::get', { id: configurationId })
  config = configSchema.parse(entry.value)
  await refreshAvailability()
  await read.boot()
  const { keys } = await client.call<{ keys: string[] }>('state::list_keys', {
    scope: 'control-plane',
  })
  const storedScopes = keys
    .filter((key) => key.startsWith('summary:'))
    .map((key) => key.slice('summary:'.length))
  const projects = storedScopes.some((scope) => scope !== 'all') ? await projectIds() : []
  await Promise.all(
    storedScopes
      .filter((scope) => scope === 'all' || projects.includes(scope))
      .map(async (scope) => {
        await writer.load(scope)
        armSummaryDeadline(scope)
      }),
  )
  const uiDir = uiDirs.find((dir) => existsSync(join(dir, 'page.js')))
  if (uiDir) {
    const assets = {
      'control-plane/page.js': { file: 'page.js', kind: 'script', content_type: 'text/javascript' },
      'control-plane/styles.css': { file: 'styles.css', kind: 'style', content_type: 'text/css' },
    }
    iii.registerFunction(
      'control-plane::ui-content',
      async ({ path }: { path: string }) => {
        if (!Object.hasOwn(assets, path)) throw new Error('unknown Control plane asset')
        const asset = assets[path as keyof typeof assets]
        return {
          content: await readFile(join(uiDir, asset.file), 'utf8'),
          content_type: asset.content_type,
        }
      },
      {
        description: 'Serve the injected control plane page assets.',
        metadata: { internal: true },
        request_format: object({ path: { type: 'string' } }),
        response_format: object({ content: { type: 'string' }, content_type: { type: 'string' } }),
      },
    )
    for (const [path, asset] of Object.entries(assets)) {
      if (!existsSync(join(uiDir, asset.file))) continue
      const observer = iii.observe(
        { function_id: 'ade::ui::asset', args: { path, kind: asset.kind } },
        'control-plane::ui-content',
      )
      void observer.ready().catch((error) => {
        if (observer.state === 'error') console.error('Control plane asset observer:', path, error)
      })
      observers.push(observer)
    }
  }
  writerReady = true
  armWriterTimer()
  return {
    read,
    writer,
    dispose: async () => {
      writerReady = false
      clearTimeout(writerTimer)
      for (const deadline of summaryDeadlines.values()) clearTimeout(deadline)
      read.dispose()
      writer.dispose()
      await Promise.all(observers.map((observer) => observer.dispose()))
    },
  }
}
