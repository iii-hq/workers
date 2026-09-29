/**
 * The configuration form's pure half: what the stored value looks like, what
 * a field change does to it, and what is wrong with it before it is saved.
 *
 * Kept separate from the component for one reason that matters: **keys this
 * form does not know about are preserved**. A worker gains a setting, an
 * operator sets it by hand, and the form must not silently drop it on the
 * next save.
 */

/** @typedef {Record<string, unknown>} Config */

export const DEFAULTS = Object.freeze({
  enabled: true,
  sources: { trace: { enabled: true }, log: { enabled: true, join_window_ms: 2000 } },
  investigation: { model: '' },
  projects: [],
  retention: {
    evidence_per_group: 5,
    occurrences_per_group: 1000,
    buckets_days: 30,
    resolved_ttl_days: 90,
    cron: '0 0 3 * * *',
  },
  archive: { bucket: '' },
})

/**
 * Merge a stored value onto the defaults without losing anything stored.
 * @param {unknown} stored
 * @returns {Config}
 */
export function normalize(stored) {
  /** @type {Record<string, any>} */
  const raw = stored && typeof stored === 'object' ? { ...stored } : {}
  // `projects` was called `repositories` until 2026-09. A value stored under
  // the old name is read, and saved back under the new one only: the worker
  // refuses the two together.
  const { repositories: former, ...value } = raw
  if (!Array.isArray(value.projects) && Array.isArray(former)) value.projects = former
  return {
    ...DEFAULTS,
    ...value,
    sources: { ...DEFAULTS.sources, ...(value.sources ?? {}) },
    investigation: { ...DEFAULTS.investigation, ...(value.investigation ?? {}) },
    retention: { ...DEFAULTS.retention, ...(value.retention ?? {}) },
    archive: { ...DEFAULTS.archive, ...(value.archive ?? {}) },
    projects: Array.isArray(value.projects) ? value.projects : [],
  }
}

/**
 * Set one dotted path, returning a new value. Untouched keys survive
 * verbatim — including the ones this form never renders.
 * @param {Config} config
 * @param {string} path
 * @param {unknown} next
 * @returns {Config}
 */
export function setPath(config, path, next) {
  const [head, ...rest] = path.split('.')
  if (rest.length === 0) return { ...config, [head]: next }
  const child = /** @type {Record<string, any>} */ (config)[head]
  return {
    ...config,
    [head]: setPath(child && typeof child === 'object' ? { ...child } : {}, rest.join('.'), next),
  }
}

/**
 * What would be refused on save, in the operator's words. The worker
 * validates again — this exists so a typo is caught before a round trip,
 * never instead of it.
 * @param {Config} config
 * @returns {string[]}
 */
export function problems(config) {
  /** @type {string[]} */
  const out = []
  /** @type {Record<string, any>} */
  const retention = (config.retention ?? {})
  const repositories = Array.isArray(config.projects) ? config.projects : []
  /** @type {Map<string, string>} */
  const seen = new Map()
  for (const repository of repositories) {
    const path = String(repository?.path ?? '')
    if (!path.startsWith('/')) {
      out.push(`${repository?.id || 'a project'} needs an absolute path`)
    }
    for (const worker of repository?.workers ?? []) {
      const owner = seen.get(worker)
      if (owner && owner !== repository.id) {
        out.push(`${worker} is mapped to both ${owner} and ${repository.id}`)
      }
      seen.set(worker, repository.id)
    }
  }
  const cron = String(retention.cron ?? '')
  const fields = cron.trim().split(/\s+/).length
  if (cron && fields !== 6 && fields !== 7) {
    out.push('the prune schedule needs a six or seven field cron expression')
  }
  if (Number(retention.evidence_per_group) < 1) {
    out.push('keep at least one bundle per group')
  }
  return out
}

/**
 * Add a repository row with an id nothing else is using.
 * @param {Config} config
 * @param {string} path
 * @returns {Config}
 */
export function addRepository(config, path) {
  /** @type {{ id: string, path: string, workers: string[] }[]} */
  const repositories = Array.isArray(config.projects) ? config.projects : []
  const base = path.split('/').filter(Boolean).pop() || 'project'
  let id = base
  let suffix = 2
  while (repositories.some((repository) => repository.id === id)) id = `${base}-${suffix++}`
  return { ...config, projects: [...repositories, { id, path, workers: [] }] }
}

/** @param {string} path */
const trimmed = (path) => path.replace(/\/+$/, '') || '/'

/**
 * The repository already mapped at this folder, if any. Picking a folder a
 * second time opens the one that is there instead of adding a copy.
 * @param {Config} config
 * @param {string} path
 * @returns {{ id: string, path: string, workers: string[] } | undefined}
 */
export function repositoryAt(config, path) {
  const repositories = Array.isArray(config.projects) ? config.projects : []
  return repositories.find((repository) => trimmed(String(repository?.path ?? '')) === trimmed(path))
}

/**
 * Point a repository at another folder; its workers stay mapped to it.
 * @param {Config} config
 * @param {string} id
 * @param {string} path
 * @returns {Config}
 */
export function setRepositoryPath(config, id, path) {
  const repositories = Array.isArray(config.projects) ? config.projects : []
  return {
    ...config,
    projects: repositories.map((repository) => (repository.id === id ? { ...repository, path } : repository)),
  }
}

/**
 * Give a repository its workers. A worker belongs to at most one checkout,
 * so one taken from another repository moves rather than being mapped
 * twice — the conflict is resolved where it is made, not reported on save.
 * @param {Config} config
 * @param {string} id
 * @param {string[]} workers
 * @returns {Config}
 */
export function setRepositoryWorkers(config, id, workers) {
  const repositories = Array.isArray(config.projects) ? config.projects : []
  const unique = [...new Set(workers.map((worker) => worker.trim()).filter(Boolean))]
  return {
    ...config,
    projects: repositories.map((repository) =>
      repository.id === id
        ? { ...repository, workers: unique }
        : { ...repository, workers: (repository.workers ?? []).filter((/** @type {string} */ worker) => !unique.includes(worker)) },
    ),
  }
}
