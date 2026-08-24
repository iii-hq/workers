import assert from 'node:assert/strict'
import test from 'node:test'
import {
  assertCompleteGithubSourceSet,
  buildSecuritySourceSummary,
  buildStatusOptions,
  categorizeFindings,
  categoryCoverageLabel,
  classifyFinding,
  conciseReportTitle,
  emptyCategoryMessage,
  filterGithubAlerts,
  githubBlobUrl,
  githubCollectionState,
  githubCollectionStateCopy,
  githubCommitUrl,
  githubOpenAlertCount,
  githubZipUrl,
  isUsefulRemediation,
  nextVisibleAlertCount,
  overallGithubCollectionState,
  reconciliationScopeLabel,
  reportDownloadFilename,
  serializeSanitizedRun,
  shouldAutoCollectGithubSources,
  sourceCommitPresentation,
  sourceCountLabel,
} from './security-dashboard.js'

function finding(ruleId, title, severity = 'high') {
  return {
    rule_id: ruleId,
    severity,
    title,
    description: 'description',
    evidence: 'evidence',
    remediation: 'Rotate the exposed value.',
  }
}

test('classifies all four overview categories from rule ids and titles', () => {
  assert.equal(
    classifyFinding(finding('command-injection', 'Unsafe shell interpolation')),
    'vulnerabilities',
  )
  assert.equal(
    classifyFinding(finding('deps/npm-audit', 'Vulnerable package version')),
    'dependencies',
  )
  assert.equal(
    classifyFinding(finding('hardcoded-secret', 'Embedded API key')),
    'secrets',
  )
  assert.equal(
    classifyFinding(finding('release-provenance', 'Unsigned release artifact')),
    'supply-chain',
  )
  assert.equal(
    classifyFinding(
      finding('dependency-confusion', 'Unscoped internal package'),
    ),
    'supply-chain',
  )

  const grouped = categorizeFindings([
    finding('sql-injection', 'SQL injection'),
    finding('cargo-lockfile', 'Outdated crate'),
    finding('credential-leak', 'Password committed'),
    finding('ci-workflow', 'Mutable GitHub Action'),
  ])
  assert.deepEqual(
    Object.fromEntries(
      Object.entries(grouped).map(([category, items]) => [
        category,
        items.length,
      ]),
    ),
    { vulnerabilities: 1, dependencies: 1, secrets: 1, 'supply-chain': 1 },
  )
})

test('builds an explicit all-status option and status counts', () => {
  assert.deepEqual(
    buildStatusOptions(
      ['queued', 'completed', 'failed'],
      { completed: 4, failed: 1 },
      7,
    ),
    [
      { value: 'all', label: 'all statuses (7)' },
      { value: 'queued', label: 'queued (0)' },
      { value: 'completed', label: 'completed (4)' },
      { value: 'failed', label: 'failed (1)' },
    ],
  )
})

test('distinguishes assessed, unassessed, and legacy unknown coverage', () => {
  assert.equal(
    categoryCoverageLabel({ status: 'assessed' }, 0),
    'assessed · no findings',
  )
  assert.equal(
    categoryCoverageLabel({ status: 'not_assessed' }, 0),
    'not assessed',
  )
  assert.equal(
    categoryCoverageLabel({ status: 'unknown' }, 0),
    'none reported · coverage unknown',
  )
  assert.equal(
    emptyCategoryMessage({
      status: 'not_assessed',
      reason: 'No supported manifest was present.',
    }),
    'Not assessed: No supported manifest was present.',
  )
  assert.equal(
    emptyCategoryMessage({ status: 'unknown' }),
    'No findings were reported. Coverage was not recorded for this run.',
  )
})

test('creates commit-pinned GitHub URLs only from safe repository ids and paths', () => {
  const sha = '0123456789abcdef0123456789abcdef01234567'
  assert.equal(
    githubBlobUrl('iii-hq/iii', sha, 'src/space file.ts', 12, 18),
    `https://github.com/iii-hq/iii/blob/${sha}/src/space%20file.ts#L12-L18`,
  )
  assert.equal(
    githubZipUrl('iii-hq/iii', sha),
    `https://github.com/iii-hq/iii/archive/${sha}.zip`,
  )
  assert.equal(
    githubBlobUrl('https://github.com/iii-hq/iii', sha, 'src/a.ts', 1),
    null,
  )
  assert.equal(githubBlobUrl('iii-hq/iii', 'main', 'src/a.ts', 1), null)
  assert.equal(githubBlobUrl('iii-hq/iii', sha, '../secret', 1), null)
  assert.equal(githubBlobUrl('iii-hq/iii', sha, '/etc/passwd', 1), null)
  assert.equal(githubBlobUrl('iii-hq/iii', sha, 'src\\a.ts', 1), null)
})

test('makes a different code-scanning analysis commit explicit and safely linkable', () => {
  const targetSha = '0123456789abcdef0123456789abcdef01234567'
  const analysisSha = 'fedcba9876543210fedcba9876543210fedcba98'

  assert.deepEqual(sourceCommitPresentation(analysisSha, targetSha), {
    sha: analysisSha,
    short: 'fedcba98',
    differsFromTarget: true,
  })
  assert.equal(
    githubCommitUrl('iii-hq/iii', analysisSha),
    `https://github.com/iii-hq/iii/commit/${analysisSha}`,
  )
  assert.equal(
    sourceCommitPresentation(targetSha, targetSha)?.differsFromTarget,
    false,
  )
  assert.equal(sourceCommitPresentation('main', targetSha), null)
  assert.equal(
    githubCommitUrl('https://github.com/iii-hq/iii', analysisSha),
    null,
  )
})

test('omits placeholder remediation without treating real guidance as empty', () => {
  assert.equal(isUsefulRemediation('Not provided per review scope'), false)
  assert.equal(isUsefulRemediation('N/A'), false)
  assert.equal(
    isUsefulRemediation('Rotate the token and invalidate active sessions.'),
    true,
  )
})

test('uses a concise report title with deterministic fallbacks', () => {
  assert.equal(
    conciseReportTitle(
      'One unsafe workflow can publish unsigned artifacts.',
      1,
      'completed',
    ),
    'One unsafe workflow can publish unsigned artifacts.',
  )
  assert.equal(
    conciseReportTitle('', 0, 'completed'),
    'No security findings reported',
  )
  assert.equal(
    conciseReportTitle(undefined, 2, 'completed'),
    '2 security findings reported',
  )
})

test('serializes the complete public report while dropping unknown internal fields', () => {
  const run = {
    schema_version: 'security-scan.run.v1',
    run_id: 'run-1',
    repository: 'iii-hq/iii',
    target_sha: '0123456789abcdef0123456789abcdef01234567',
    mode: 'suggest',
    status: 'completed',
    attempt: 2,
    created_at: 1,
    updated_at: 2,
    completed_at: 3,
    internal_checkout_root: '/private/checkout',
    report: {
      summary: 'One finding.',
      assessments: {
        vulnerabilities: { status: 'assessed' },
        dependencies: { status: 'not_assessed', reason: 'No manifest.' },
        secrets: { status: 'assessed' },
        supply_chain: { status: 'assessed' },
      },
      findings: [
        {
          ...finding('hardcoded-secret', 'Embedded API key', 'critical'),
          location: { path: 'src/config.ts', line_start: 4, line_end: 5 },
          suggested_patch: 'diff --git a/src/config.ts b/src/config.ts',
          internal_tool_trace: 'hidden',
        },
      ],
    },
  }

  const text = serializeSanitizedRun(run)
  const parsed = JSON.parse(text)
  assert.equal(text.endsWith('\n'), true)
  assert.equal(parsed.internal_checkout_root, undefined)
  assert.equal(parsed.report.findings[0].internal_tool_trace, undefined)
  assert.equal(
    parsed.report.findings[0].suggested_patch,
    'diff --git a/src/config.ts b/src/config.ts',
  )
  assert.deepEqual(parsed.report.assessments.dependencies, {
    status: 'not_assessed',
    reason: 'No manifest.',
  })
  assert.deepEqual(parsed.report.findings[0].location, {
    path: 'src/config.ts',
    line_start: 4,
    line_end: 5,
  })
  assert.equal(
    reportDownloadFilename(run),
    'security-report-iii-hq-iii-01234567.json',
  )
})

test('keeps Harness and GitHub source counts separate without inventing a unique total', () => {
  const summary = buildSecuritySourceSummary(3, 221, true)
  assert.equal(summary.harness, '3 Harness-verified findings')
  assert.equal(summary.github, '221 GitHub open alert records')
  assert.match(summary.qualification, /not additive/i)
  assert.match(summary.qualification, /not a unique vulnerability total/i)
  assert.equal(Object.values(summary).join(' ').includes('224'), false)
})

test('requires exactly one summary for each GitHub security source', () => {
  const dependabot = {
    source: 'dependabot',
    status: 'complete',
    record_count: 151,
  }
  const codeScanning = {
    source: 'code_scanning',
    status: 'complete',
    record_count: 70,
  }

  assert.deepEqual(assertCompleteGithubSourceSet([dependabot, codeScanning]), [
    dependabot,
    codeScanning,
  ])
  assert.throws(
    () => assertCompleteGithubSourceSet([dependabot]),
    /code_scanning/,
  )
  assert.throws(
    () => assertCompleteGithubSourceSet([dependabot, dependabot, codeScanning]),
    /duplicate dependabot/,
  )
})

test('formats incomplete GitHub counts as lower bounds', () => {
  assert.equal(sourceCountLabel(221, false), '221+')
  assert.equal(sourceCountLabel(0, false), '0+')
  assert.equal(
    buildSecuritySourceSummary(3, 221, false).github,
    '221+ GitHub open alert records',
  )
  assert.equal(
    buildSecuritySourceSummary(null, null, false).harness,
    'Harness verification not available',
  )
})

test('gives every collection state explicit non-color-only copy', () => {
  const expected = {
    not_collected: 'Not collected',
    not_configured: 'Not configured',
    auth: 'Authentication required',
    permission: 'Permission denied',
    disabled: 'Disabled on repository',
    partial: 'Partial snapshot',
    unavailable: 'Temporarily unavailable',
    complete: 'Complete snapshot',
  }
  for (const [state, label] of Object.entries(expected)) {
    const copy = githubCollectionStateCopy(state, 4)
    assert.equal(copy.label, label)
    assert.ok(copy.detail.length > 20)
    assert.ok(copy.action.length > 5)
  }
  assert.equal(
    githubCollectionStateCopy('complete', 0).label,
    'Complete, no open alerts',
  )
})

test('filters alert sources and paginates in bounded increments', () => {
  const alerts = [
    { source: 'dependabot' },
    { source: 'code_scanning' },
    { source: 'dependabot' },
  ]
  assert.equal(filterGithubAlerts(alerts, 'all').length, 3)
  assert.equal(filterGithubAlerts(alerts, 'dependabot').length, 2)
  assert.equal(filterGithubAlerts(alerts, 'code_scanning').length, 1)
  assert.equal(nextVisibleAlertCount(25, 221, 25), 50)
  assert.equal(nextVisibleAlertCount(200, 221, 25), 221)
  assert.equal(nextVisibleAlertCount(221, 221, 25), 221)
})

test('maps frozen backend states and keeps partial source counts honest', () => {
  assert.equal(githubCollectionState('authentication_required'), 'auth')
  assert.equal(githubCollectionState('permission_denied'), 'permission')
  assert.equal(
    overallGithubCollectionState([
      { status: 'complete' },
      { status: 'unavailable' },
    ]),
    'partial',
  )
  assert.deepEqual(
    githubOpenAlertCount([
      { status: 'complete', record_count: 200 },
      { status: 'unavailable', record_count: null },
    ]),
    { count: 200, complete: false },
  )
})

test('labels exact commit scope only when the backend scope is exact_commit', () => {
  const sha = '0123456789abcdef0123456789abcdef01234567'
  assert.equal(
    reconciliationScopeLabel('exact_commit', sha),
    'Exact commit 01234567',
  )
  assert.equal(
    reconciliationScopeLabel('repository_default_branch', sha),
    'Current repository default branch',
  )
  assert.equal(
    reconciliationScopeLabel('repository_snapshot', sha),
    'Current repository snapshot',
  )
})

test('auto-collects only a first not-collected snapshot', () => {
  const uncollected = [{ status: 'not_collected' }, { status: 'not_collected' }]
  assert.equal(shouldAutoCollectGithubSources(uncollected, false), true)
  assert.equal(shouldAutoCollectGithubSources(uncollected, true), false)
  for (const status of [
    'authentication_required',
    'permission_denied',
    'disabled',
    'unavailable',
    'not_configured',
    'partial',
  ]) {
    assert.equal(shouldAutoCollectGithubSources([{ status }], false), false)
  }
})
