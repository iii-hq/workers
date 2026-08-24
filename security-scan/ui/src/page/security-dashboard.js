/** @typedef {'vulnerabilities' | 'dependencies' | 'secrets' | 'supply-chain'} FindingCategory */
/** @typedef {'vulnerabilities' | 'dependencies' | 'secrets' | 'supply_chain'} AssessmentKey */
/** @typedef {'assessed' | 'not_assessed' | 'unknown'} AssessmentStatus */
/** @typedef {{ status: AssessmentStatus, reason?: string }} DashboardAssessment */
/** @typedef {'critical' | 'high' | 'medium' | 'low' | 'info'} FindingSeverity */
/**
 * @typedef {{
 *   rule_id: string,
 *   severity: FindingSeverity,
 *   title: string,
 *   description: string,
 *   evidence: string,
 *   location?: { path: string, line_start?: number, line_end?: number },
 *   remediation: string,
 *   suggested_patch?: string,
 * }} DashboardFinding
 */

/** @type {ReadonlyArray<{ id: FindingCategory, assessmentKey: AssessmentKey, label: string }>} */
export const FINDING_CATEGORIES = [
  {
    id: 'vulnerabilities',
    assessmentKey: 'vulnerabilities',
    label: 'vulnerabilities',
  },
  {
    id: 'dependencies',
    assessmentKey: 'dependencies',
    label: 'dependencies / packages',
  },
  { id: 'secrets', assessmentKey: 'secrets', label: 'secrets' },
  { id: 'supply-chain', assessmentKey: 'supply_chain', label: 'supply chain' },
]

const SUPPLY_CHAIN_TERMS = [
  'supply chain',
  'dependency confusion',
  'typosquat',
  'provenance',
  'sigstore',
  'sbom',
  'github action',
  'workflow',
  'ci cd',
  'build pipeline',
  'release artifact',
  'artifact signing',
  'unsigned artifact',
]

const SECRET_TERMS = [
  'secret',
  'credential',
  'api key',
  'access key',
  'private key',
  'password',
  'auth token',
  'bearer token',
  'hardcoded token',
  'token leak',
]

const DEPENDENCY_TERMS = [
  'dependency',
  'dependencies',
  'package',
  'lockfile',
  'lock file',
  'npm',
  'pnpm',
  'yarn',
  'pip',
  'pypi',
  'cargo',
  'crate',
  'maven',
  'gradle',
  'composer',
  'gem',
  'third party library',
  'vulnerable component',
]

/** @param {string} value @param {readonly string[]} terms */
function includesTerm(value, terms) {
  return terms.some((term) => value.includes(term))
}

/** @param {Pick<DashboardFinding, 'rule_id' | 'title'>} finding @returns {FindingCategory} */
export function classifyFinding(finding) {
  const searchable = `${finding.rule_id} ${finding.title}`
    .toLowerCase()
    .replace(/[_./:\\-]+/g, ' ')
    .replace(/\s+/g, ' ')

  if (includesTerm(searchable, SUPPLY_CHAIN_TERMS)) return 'supply-chain'
  if (includesTerm(searchable, SECRET_TERMS)) return 'secrets'
  if (includesTerm(searchable, DEPENDENCY_TERMS)) return 'dependencies'
  return 'vulnerabilities'
}

/**
 * @param {readonly DashboardFinding[]} findings
 * @returns {Record<FindingCategory, DashboardFinding[]>}
 */
export function categorizeFindings(findings) {
  /** @type {Record<FindingCategory, DashboardFinding[]>} */
  const categories = {
    vulnerabilities: [],
    dependencies: [],
    secrets: [],
    'supply-chain': [],
  }
  for (const finding of findings)
    categories[classifyFinding(finding)].push(finding)
  return categories
}

/**
 * @param {readonly DashboardFinding[]} findings
 * @returns {Record<FindingSeverity, number>}
 */
export function countSeverities(findings) {
  /** @type {Record<FindingSeverity, number>} */
  const counts = { critical: 0, high: 0, medium: 0, low: 0, info: 0 }
  for (const finding of findings) counts[finding.severity] += 1
  return counts
}

/** @param {DashboardAssessment} assessment @param {number} findingCount */
export function categoryCoverageLabel(assessment, findingCount) {
  if (assessment.status === 'assessed') {
    return findingCount === 0 ? 'assessed · no findings' : 'assessed'
  }
  if (assessment.status === 'not_assessed') return 'not assessed'
  return findingCount === 0
    ? 'none reported · coverage unknown'
    : 'coverage unknown'
}

/** @param {DashboardAssessment} assessment */
export function emptyCategoryMessage(assessment) {
  if (assessment.status === 'assessed') {
    return 'No findings were verified in this assessed area.'
  }
  if (assessment.status === 'not_assessed') {
    const reason = assessment.reason?.trim()
    return reason ? `Not assessed: ${reason}` : 'This area was not assessed.'
  }
  return 'No findings were reported. Coverage was not recorded for this run.'
}

/**
 * @param {readonly string[]} statuses
 * @param {Partial<Record<string, number>>} counts
 * @param {number} total
 */
export function buildStatusOptions(statuses, counts, total) {
  return [
    { value: 'all', label: `all statuses (${total})` },
    ...statuses.map((status) => ({
      value: status,
      label: `${status.replaceAll('_', ' ')} (${counts[status] ?? 0})`,
    })),
  ]
}

/** @param {string} repository */
function validGitHubRepository(repository) {
  const parts = repository.split('/')
  if (parts.length !== 2) return null
  const [owner, name] = parts
  if (!/^[A-Za-z0-9](?:[A-Za-z0-9-]{0,37}[A-Za-z0-9])?$/.test(owner))
    return null
  if (!/^[A-Za-z0-9._-]{1,100}$/.test(name) || name === '.' || name === '..')
    return null
  return `${owner}/${name}`
}

/** @param {string} targetSha */
function validTargetSha(targetSha) {
  return /^[a-fA-F0-9]{40}$/.test(targetSha)
}

/** @param {string} path */
function encodedRelativePath(path) {
  if (
    !path ||
    path.startsWith('/') ||
    path.includes('\\') ||
    /[\0\r\n]/.test(path)
  ) {
    return null
  }
  const parts = path.split('/')
  if (parts.some((part) => !part || part === '.' || part === '..')) return null
  return parts.map((part) => encodeURIComponent(part)).join('/')
}

/**
 * @param {string} repository
 * @param {string} targetSha
 * @param {string} path
 * @param {number | undefined} lineStart
 * @param {number | undefined} lineEnd
 */
export function githubBlobUrl(repository, targetSha, path, lineStart, lineEnd) {
  const safeRepository = validGitHubRepository(repository)
  const safePath = encodedRelativePath(path)
  if (!safeRepository || !validTargetSha(targetSha) || !safePath) return null

  let anchor = ''
  if (Number.isInteger(lineStart) && /** @type {number} */ (lineStart) > 0) {
    anchor = `#L${lineStart}`
    if (
      Number.isInteger(lineEnd) &&
      /** @type {number} */ (lineEnd) >= /** @type {number} */ (lineStart)
    ) {
      anchor += `-L${lineEnd}`
    }
  }
  return `https://github.com/${safeRepository}/blob/${targetSha}/${safePath}${anchor}`
}

/** @param {string} repository @param {string} commitSha */
export function githubCommitUrl(repository, commitSha) {
  const safeRepository = validGitHubRepository(repository)
  if (!safeRepository || !validTargetSha(commitSha)) return null
  return `https://github.com/${safeRepository}/commit/${commitSha}`
}

/** @param {string | undefined} commitSha @param {string} targetSha */
export function sourceCommitPresentation(commitSha, targetSha) {
  if (!commitSha || !validTargetSha(commitSha)) return null
  return {
    sha: commitSha,
    short: commitSha.slice(0, 8),
    differsFromTarget:
      validTargetSha(targetSha) &&
      commitSha.toLowerCase() !== targetSha.toLowerCase(),
  }
}

/** @param {string} repository @param {string} targetSha */
export function githubZipUrl(repository, targetSha) {
  const safeRepository = validGitHubRepository(repository)
  if (!safeRepository || !validTargetSha(targetSha)) return null
  return `https://github.com/${safeRepository}/archive/${targetSha}.zip`
}

/** @param {string} remediation */
export function isUsefulRemediation(remediation) {
  const normalized = remediation.trim().replace(/\s+/g, ' ')
  if (!normalized) return false
  return !/^(?:not provided(?: per (?:the )?review scope)?|n\/?a|none|unknown|no remediation(?: provided)?|not available)\.?$/i.test(
    normalized,
  )
}

/** @param {string | undefined} summary @param {number} findingCount @param {string} status */
export function conciseReportTitle(summary, findingCount, status) {
  const normalized = summary?.trim().replace(/\s+/g, ' ') ?? ''
  if (normalized) {
    if (normalized.length <= 116) return normalized
    const sentence = normalized
      .slice(0, 116)
      .match(/^(.{40,}?[.!?])(?:\s|$)/)?.[1]
    if (sentence) return sentence
    const boundary = normalized.slice(0, 113).lastIndexOf(' ')
    return `${normalized.slice(0, boundary > 70 ? boundary : 113).trimEnd()}…`
  }
  if (status === 'completed' && findingCount === 0)
    return 'No security findings reported'
  if (status === 'completed') {
    return `${findingCount} security ${findingCount === 1 ? 'finding' : 'findings'} reported`
  }
  if (status === 'failed') return 'Security review failed'
  if (status === 'cancelled') return 'Security review cancelled'
  return `${status.replaceAll('_', ' ')} security review`
}

/**
 * Reconstruct only the public fields parsed from security-scan::read. This
 * keeps future internal response fields out of downloads by default.
 *
 * @param {Record<string, any>} run
 */
export function serializeSanitizedRun(run) {
  const sanitized = {
    schema_version: run.schema_version,
    run_id: run.run_id,
    repository: run.repository,
    target_sha: run.target_sha,
    mode: run.mode,
    status: run.status,
    attempt: run.attempt,
    ...(run.error
      ? {
          error: {
            code: run.error.code,
            message: run.error.message,
            retryable: run.error.retryable,
          },
        }
      : {}),
    created_at: run.created_at,
    updated_at: run.updated_at,
    ...(run.completed_at == null ? {} : { completed_at: run.completed_at }),
    ...(run.report
      ? {
          report: {
            summary: run.report.summary,
            assessments: {
              vulnerabilities: { ...run.report.assessments.vulnerabilities },
              dependencies: { ...run.report.assessments.dependencies },
              secrets: { ...run.report.assessments.secrets },
              supply_chain: { ...run.report.assessments.supply_chain },
            },
            findings: run.report.findings.map(
              (/** @type {DashboardFinding} */ finding) => ({
                rule_id: finding.rule_id,
                severity: finding.severity,
                title: finding.title,
                description: finding.description,
                evidence: finding.evidence,
                ...(finding.location
                  ? {
                      location: {
                        path: finding.location.path,
                        ...(finding.location.line_start == null
                          ? {}
                          : { line_start: finding.location.line_start }),
                        ...(finding.location.line_end == null
                          ? {}
                          : { line_end: finding.location.line_end }),
                      },
                    }
                  : {}),
                remediation: finding.remediation,
                ...(finding.suggested_patch == null
                  ? {}
                  : { suggested_patch: finding.suggested_patch }),
              }),
            ),
          },
        }
      : {}),
  }
  return `${JSON.stringify(sanitized, null, 2)}\n`
}

/** @param {{ repository: string, target_sha: string }} run */
export function reportDownloadFilename(run) {
  const repository =
    run.repository.replace(/[^A-Za-z0-9._-]+/g, '-').replace(/^-+|-+$/g, '') ||
    'repository'
  const sha = /^[a-fA-F0-9]{8,}$/.test(run.target_sha)
    ? run.target_sha.slice(0, 8)
    : 'run'
  return `security-report-${repository}-${sha}.json`
}

/** @typedef {'all' | 'dependabot' | 'code_scanning'} GitHubAlertFilter */
/** @typedef {'not_collected' | 'not_configured' | 'auth' | 'permission' | 'disabled' | 'partial' | 'unavailable' | 'complete'} GitHubCollectionState */
/** @typedef {{ source: 'dependabot' | 'code_scanning' }} SourceAlert */

export const GITHUB_ALERT_FILTERS = [
  { id: 'all', label: 'All' },
  { id: 'dependabot', label: 'Dependabot' },
  { id: 'code_scanning', label: 'Code scanning' },
]

const REQUIRED_GITHUB_SOURCES = ['dependabot', 'code_scanning']

/**
 * @template {{ source: string }} T
 * @param {readonly T[]} sources
 * @returns {T[]}
 */
export function assertCompleteGithubSourceSet(sources) {
  const bySource = new Map()
  for (const source of sources) {
    if (bySource.has(source.source)) {
      throw new Error(
        `GitHub source summaries contained duplicate ${source.source}`,
      )
    }
    bySource.set(source.source, source)
  }
  for (const required of REQUIRED_GITHUB_SOURCES) {
    if (!bySource.has(required)) {
      throw new Error(`GitHub source summaries were missing ${required}`)
    }
  }
  if (bySource.size !== REQUIRED_GITHUB_SOURCES.length) {
    throw new Error('GitHub source summaries contained an unexpected source')
  }
  return [...sources]
}

/** @param {number} count @param {boolean} complete */
export function sourceCountLabel(count, complete) {
  const safeCount = Number.isFinite(count) && count >= 0 ? Math.floor(count) : 0
  return complete ? String(safeCount) : `${safeCount}+`
}

/**
 * Source counts deliberately remain separate. Harness and GitHub can report
 * the same weakness with different identities, so adding them would imply a
 * deduplication result that the reconciliation contract does not provide.
 *
 * @param {number | null} harnessCount
 * @param {number | null} githubCount
 * @param {boolean} githubCountComplete
 * @param {boolean} [matchingAvailable]
 */
export function buildSecuritySourceSummary(
  harnessCount,
  githubCount,
  githubCountComplete,
  matchingAvailable = false,
) {
  const githubLabel =
    githubCount == null
      ? 'not available'
      : sourceCountLabel(githubCount, githubCountComplete)
  return {
    harness:
      harnessCount == null
        ? 'Harness verification not available'
        : `${sourceCountLabel(harnessCount, true)} Harness-verified ${harnessCount === 1 ? 'finding' : 'findings'}`,
    github: `${githubLabel} GitHub open alert ${githubCount === 1 ? 'record' : 'records'}`,
    qualification: matchingAvailable
      ? 'GitHub counts are alert records, not a unique vulnerability total. Harness findings cover a different scope, so source counts remain separate even when records are matched.'
      : 'GitHub counts are alert records, not a unique vulnerability total. Harness findings cover a different scope, so the counts are not additive; cross-source matching is unavailable.',
  }
}

/** @param {string} status @returns {GitHubCollectionState} */
export function githubCollectionState(status) {
  if (status === 'authentication_required') return 'auth'
  if (status === 'permission_denied') return 'permission'
  if (status === 'complete') return 'complete'
  if (
    status === 'not_collected' ||
    status === 'not_configured' ||
    status === 'disabled' ||
    status === 'partial' ||
    status === 'unavailable'
  ) {
    return status
  }
  return 'unavailable'
}

/** @param {readonly { status: string }[]} sources @returns {GitHubCollectionState} */
export function overallGithubCollectionState(sources) {
  if (sources.length === 0) return 'not_collected'
  const states = sources.map((source) => githubCollectionState(source.status))
  if (states.every((state) => state === 'complete')) return 'complete'
  if (states.includes('partial') || states.includes('complete'))
    return 'partial'
  return states[0]
}

/** @param {readonly { status: string, record_count: number | null }[]} sources */
export function githubOpenAlertCount(sources) {
  let knownCount = 0
  let hasKnownCount = false
  for (const source of sources) {
    if (
      source.record_count == null ||
      !Number.isFinite(source.record_count) ||
      source.record_count < 0
    )
      continue
    knownCount += source.record_count
    hasKnownCount = true
  }
  return {
    count: hasKnownCount ? knownCount : null,
    complete:
      sources.length > 0 &&
      sources.every((source) => source.status === 'complete'),
  }
}

/** @param {string} scope @param {string} [targetSha] */
export function reconciliationScopeLabel(scope, targetSha) {
  if (scope === 'exact_commit') {
    const normalizedTarget = targetSha ?? ''
    const shortTarget = /^[a-fA-F0-9]{8,}$/.test(normalizedTarget)
      ? normalizedTarget.slice(0, 8)
      : ''
    return shortTarget ? `Exact commit ${shortTarget}` : 'Exact commit'
  }
  if (scope === 'repository_default_branch')
    return 'Current repository default branch'
  if (scope === 'repository_snapshot') return 'Current repository snapshot'
  return 'Repository scope unavailable'
}

/** @param {string} matchingStatus */
export function alertMatchLabel(matchingStatus) {
  return matchingStatus === 'available'
    ? 'Cross-source matching available'
    : 'Matching unavailable'
}

/** @param {readonly { status: string }[]} sources @param {boolean} alreadyAttempted */
export function shouldAutoCollectGithubSources(sources, alreadyAttempted) {
  return (
    !alreadyAttempted &&
    sources.length > 0 &&
    sources.every((source) => source.status === 'not_collected')
  )
}

/** @param {GitHubCollectionState} state @param {number | null} count */
export function githubCollectionStateCopy(state, count) {
  if (state === 'not_collected') {
    return {
      label: 'Not collected',
      detail: 'GitHub alerts have not been collected for this run.',
      action: 'Collect GitHub alerts',
    }
  }
  if (state === 'not_configured') {
    return {
      label: 'Not configured',
      detail: 'GitHub security sources are not configured for this worker.',
      action: 'Retry source',
    }
  }
  if (state === 'auth') {
    return {
      label: 'Authentication required',
      detail: 'GitHub authentication is missing or invalid.',
      action: 'Retry source',
    }
  }
  if (state === 'permission') {
    return {
      label: 'Permission denied',
      detail:
        'The configured GitHub identity cannot read security alerts for this repository.',
      action: 'Retry source',
    }
  }
  if (state === 'disabled') {
    return {
      label: 'Disabled on repository',
      detail:
        'GitHub security alerts are disabled or unavailable for this repository.',
      action: 'Retry source',
    }
  }
  if (state === 'partial') {
    return {
      label: 'Partial snapshot',
      detail: 'Some alerts could not be collected. Counts are lower bounds.',
      action: 'Refresh source',
    }
  }
  if (state === 'unavailable') {
    return {
      label: 'Temporarily unavailable',
      detail: 'GitHub did not return a usable security snapshot.',
      action: 'Retry source',
    }
  }
  if (count === 0) {
    return {
      label: 'Complete, no open alerts',
      detail:
        'The GitHub snapshot completed and found no open alert records in the collected sources.',
      action: 'Refresh source',
    }
  }
  return {
    label: 'Complete snapshot',
    detail: 'Open GitHub alert records were collected successfully.',
    action: 'Refresh source',
  }
}

/** @param {readonly SourceAlert[]} alerts @param {GitHubAlertFilter} filter */
export function filterGithubAlerts(alerts, filter) {
  return filter === 'all'
    ? [...alerts]
    : alerts.filter((alert) => alert.source === filter)
}

/** @param {number} current @param {number} total @param {number} pageSize */
export function nextVisibleAlertCount(current, total, pageSize) {
  const safeTotal = Math.max(0, Math.floor(total))
  const safePageSize = Math.max(1, Math.floor(pageSize))
  return Math.min(safeTotal, Math.max(0, Math.floor(current)) + safePageSize)
}
