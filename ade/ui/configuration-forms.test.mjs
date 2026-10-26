import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import esbuild from 'esbuild'

const EXPECTED_IDS = [
  'a2ui',
  'approval-gate',
  'bridge',
  'canvas',
  'claude-code',
  'codex',
  'computer',
  'cursor',
  'devin',
  'document',
  'editor',
  'email',
  'fp',
  'github',
  'grok',
  'harness',
  'http',
  'memory',
  'memory-consolidate',
  'opencode',
  'openwiki',
  'pdf',
  'pi',
  'provider-xai',
  'pubsub',
  'queue',
  'rbac-proxy',
  'sandbox-code-runner',
  'security-scan',
  'session-manager',
  'shell',
  'slack',
  'tailscale',
  'telegram-bot',
  'vscode',
  'web',
  'workflow',
  'worktree',
]

async function loadManifest() {
  const result = await esbuild.build({
    entryPoints: ['src/configuration-forms/manifest.ts'],
    bundle: true,
    format: 'esm',
    platform: 'node',
    write: false,
    logLevel: 'silent',
  })
  const source = result.outputFiles[0].text
  return import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`)
}

async function loadTracePreferences() {
  const result = await esbuild.build({
    entryPoints: ['src/injectable-ui-form/preferences.ts'],
    bundle: true,
    format: 'esm',
    platform: 'node',
    write: false,
    logLevel: 'silent',
  })
  const source = result.outputFiles[0].text
  return import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`)
}

async function loadStructuredValueHelpers() {
  const result = await esbuild.build({
    entryPoints: ['src/configuration-forms/structured-value.ts'],
    bundle: true,
    format: 'esm',
    platform: 'node',
    write: false,
    logLevel: 'silent',
  })
  const source = result.outputFiles[0].text
  return import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`)
}

async function loadConfigurationNormalization() {
  const result = await esbuild.build({
    entryPoints: ['src/configuration-forms/normalization.ts'],
    bundle: true,
    format: 'esm',
    platform: 'node',
    write: false,
    logLevel: 'silent',
  })
  const source = result.outputFiles[0].text
  return import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`)
}

async function loadConfigurationValues() {
  const result = await esbuild.build({
    entryPoints: ['src/configuration-forms/value.ts'],
    bundle: true,
    format: 'esm',
    platform: 'node',
    write: false,
    logLevel: 'silent',
  })
  const source = result.outputFiles[0].text
  return import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`)
}

async function loadSpecHelpers() {
  const result = await esbuild.build({
    entryPoints: ['src/configuration-forms/spec-helpers.ts'],
    bundle: true,
    format: 'esm',
    platform: 'node',
    write: false,
    logLevel: 'silent',
  })
  const source = result.outputFiles[0].text
  return import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`)
}

async function loadModelCatalog() {
  const result = await esbuild.build({
    entryPoints: ['src/configuration-forms/model-catalog.ts'],
    bundle: true,
    format: 'esm',
    platform: 'node',
    write: false,
    logLevel: 'silent',
  })
  const source = result.outputFiles[0].text
  return import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`)
}

test('manifest covers the 38 worker-owned configuration entries', async () => {
  const manifest = await loadManifest()
  assert.doesNotThrow(() => manifest.validateWorkerConfigurationManifest())
  assert.equal(manifest.workerConfigurationManifest.length, 38)
  assert.equal(new Set(manifest.workerConfigurationIds).size, 38)
  assert.deepEqual([...manifest.workerConfigurationIds].sort(), EXPECTED_IDS)
  assert.equal(manifest.workerConfigurationIds.includes('shell-ui'), false)
})

test('entry registers every manifest id explicitly', async () => {
  const manifest = await loadManifest()
  const entry = await readFile('config-form.tsx', 'utf8')
  for (const id of manifest.workerConfigurationIds) {
    assert.match(
      entry,
      new RegExp(
        `host\\.configForms\\.register\\('${id.replace(/[.*+?^${}()|[\\]\\\\]/g, '\\$&')}'`,
      ),
      `missing explicit registration for ${id}`,
    )
  }
})

test('the ide configuration entry (ui_form "ide") renders the shell spec', async () => {
  const entry = await readFile('config-form.tsx', 'utf8')
  assert.match(entry, /host\.configForms\.register\('ide', configurationForm\('shell'\)\)/)
  assert.match(entry, /host\.configForms\.register\('shell', configurationForm\('shell'\)\)/)
})

test('ide declares commit-message controls: a model with its thinking level, and multiline instructions', async () => {
  const manifest = await loadManifest()
  const ide = manifest.workerConfigurationSpecs.get('shell')
  const section = ide.sections.find((candidate) => candidate.title === 'Commit messages')
  const [commitMessages] = section.fields
  assert.equal(commitMessages.kind, 'object')
  assert.equal(commitMessages.path.join('.'), 'commit_messages')

  const [model, instructions] = commitMessages.fields
  assert.equal(model.kind, 'model')
  assert.equal(model.thinkingKey, 'thinking')
  assert.equal(model.thinkingDefault, 'low')
  assert.equal(model.optional, true)
  assert.equal(instructions.kind, 'text')
  assert.equal(instructions.multiline, true)
  assert.equal(instructions.optional, true)

  const declared = manifest.declaredFields(ide)
  for (const leaf of ['commit_messages.model', 'commit_messages.thinking', 'commit_messages.instructions']) {
    assert.ok(declared.includes(leaf), `${leaf} is declared`)
    assert.ok(ide.expectedFields.includes(leaf), `${leaf} is expected`)
  }
})

test('harness declares the co-author switch, on by default like the Rust default', async () => {
  const manifest = await loadManifest()
  const harness = manifest.workerConfigurationSpecs.get('harness')
  const field = harness.sections.flatMap((section) => section.fields).find((f) => f.path.join('.') === 'co_author')
  assert.equal(field?.kind, 'switch')
  assert.equal(field.defaultValue, true)
  assert.ok(harness.expectedFields.includes('co_author'))
})

test('harness picks its default context manager among the names the Rust config accepts, unset by default', async () => {
  const manifest = await loadManifest()
  const harness = manifest.workerConfigurationSpecs.get('harness')
  const field = harness.sections
    .flatMap((section) => section.fields)
    .find((f) => f.path.join('.') === 'default_context_strategy')
  assert.equal(field?.kind, 'select')
  assert.equal(field.optional, true)
  assert.deepEqual(
    field.options.map((option) => option.value),
    ['default', 'clm'],
  )
  for (const option of field.options) assert.match(option.value, /^[a-z0-9-]{1,64}$/)
  assert.ok(harness.expectedFields.includes('default_context_strategy'))
})

test('a model field declares its thinking sibling as a second leaf, and only when it has one', async () => {
  const manifest = await loadManifest()
  const helpers = await loadSpecHelpers()
  const specOf = (...fields) => ({ sections: [{ title: 't', fields }] })
  assert.deepEqual(
    manifest.declaredFields(specOf(helpers.model('a.b.model', 'Model', undefined, { thinkingKey: 'effort' }))),
    ['a.b.effort', 'a.b.model'],
  )
  assert.deepEqual(manifest.declaredFields(specOf(helpers.model('model', 'Model'))), ['model'])
})

async function loadModule(entry) {
  const result = await esbuild.build({
    entryPoints: [entry],
    bundle: true,
    format: 'esm',
    platform: 'node',
    write: false,
    logLevel: 'silent',
  })
  const source = result.outputFiles[0].text
  return import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`)
}

test('iii-sandbox custom images become choices that show the OCI ref; presets are not desktops', async () => {
  const { sandboxCatalogChoices } = await loadModule('src/configuration-forms/specs/sandbox-catalog.ts')
  assert.deepEqual(
    sandboxCatalogChoices({
      images: [
        { kind: 'preset', name: 'python', oci_ref: 'docker.io/iiidev/python:latest' },
        { kind: 'custom', name: ' desktop ', oci_ref: 'ghcr.io/trycua/linux:24.04-slim' },
        { kind: 'custom', name: 'bare' },
        { name: '   ' },
        null,
      ],
    }),
    [
      { value: 'desktop', label: 'desktop — ghcr.io/trycua/linux:24.04-slim' },
      { value: 'bare', label: 'bare' },
    ],
  )
  for (const unreadable of [undefined, null, {}, { images: 'nope' }, 'nope']) {
    assert.deepEqual(sandboxCatalogChoices(unreadable), [])
  }
})

test('remote choices follow the static ones and never hide the stored value', async () => {
  const { mergeChoices } = await loadModule('src/configuration-forms/remote-choices.ts')
  const fixed = [{ value: 'a', label: 'A' }]
  const remote = [
    { value: 'a', label: 'A again' },
    { value: 'b', label: 'B' },
  ]
  assert.deepEqual(mergeChoices(fixed, remote, 'b'), [
    { value: 'a', label: 'A' },
    { value: 'b', label: 'B' },
  ])
  // The catalog is down or dropped it: the configured value stays selectable.
  assert.deepEqual(mergeChoices(fixed, [], 'gone'), [
    { value: 'a', label: 'A' },
    { value: 'gone', label: 'gone', description: 'Not listed by the running worker' },
  ])
  assert.deepEqual(mergeChoices([], [], ''), [])
  assert.deepEqual(mergeChoices([], [], undefined), [])
})

test('computer picks its sandbox image from the live catalog and sizes the VM', async () => {
  const manifest = await loadManifest()
  const fields = manifest.workerConfigurationSpecs.get('computer').sections.flatMap((section) => section.fields)
  const byPath = (path) => fields.find((field) => field.path.join('.') === path)
  const image = byPath('sandbox_image')
  assert.equal(image.kind, 'select')
  assert.equal(image.optional, true)
  assert.equal(image.optionsFrom.functionId, 'sandbox::catalog::list')
  assert.deepEqual(image.optionsFrom.choices({ images: [{ name: 'desktop', oci_ref: 'r' }] })[0].value, 'desktop')
  assert.equal(byPath('sandbox_memory_mb').kind, 'number')
  assert.equal(byPath('sandbox_cpus').kind, 'number')
})

test('computer picks its judge provider from the judges registered next to the hub in its namespace', async () => {
  const manifest = await loadManifest()
  const fields = manifest.workerConfigurationSpecs.get('computer').sections.flatMap((section) => section.fields)
  const judge = fields.find((field) => field.path.join('.') === 'judge_provider')
  assert.equal(judge.kind, 'judge-provider')
  assert.equal(judge.optional, true)
  assert.equal(judge.owner, 'computer::run')

  const { judgeCatalog, statusProblem, judgeOptions, judgeSetup, operationNote } = await loadModule(
    'src/configuration-forms/judge-providers.ts',
  )
  const row = (function_id, namespace = 'proj') => ({ function_id, namespace, metadata: { internal: true } })
  const listed = {
    functions: [
      row('computer::run'),
      row('judge::evaluate'),
      row('judge-typesafe::evaluate'),
      row('judge-typesafe::status'),
      row('judge-clef::evaluate'),
      row('judge-clef::models::list'),
      // A provider that only registered its status is not one yet; another project's is not ours.
      row('judge-openai::status'),
      row('judge-decider::evaluate', 'other'),
      row('judge::evaluate', 'other'),
      { function_id: 7 },
      null,
    ],
  }
  assert.deepEqual(judgeCatalog(listed, 'computer::run'), {
    namespace: 'proj',
    hub: true,
    providers: [
      { key: 'clef', hosted: false },
      { key: 'typesafe', hosted: true },
    ],
  })
  // Without the owner registered, the hub's namespace stands in; without either, nothing is usable.
  assert.equal(judgeCatalog({ functions: [row('judge::evaluate', 'x')] }, 'computer::run').namespace, 'x')
  for (const unreadable of [undefined, null, {}, { functions: 'nope' }]) {
    assert.deepEqual(judgeCatalog(unreadable, 'computer::run'), { namespace: '', hub: false, providers: [] })
  }

  assert.equal(statusProblem({ configured: true }), null)
  assert.equal(statusProblem({}), null)
  assert.equal(statusProblem({ configured: false, reason: ' TYPESAFE_API_KEY is not set ' }), 'TYPESAFE_API_KEY is not set')
  assert.equal(statusProblem({ configured: false }), 'No usable API key')

  const providers = [
    { key: 'clef', hosted: false, problem: null },
    { key: 'typesafe', hosted: true, problem: 'TYPESAFE_API_KEY is not set' },
  ]
  assert.deepEqual(judgeOptions(providers, 'semif'), [
    { value: 'clef', label: 'clef', description: 'Local model, loads on first use', disabled: false },
    { value: 'typesafe', label: 'typesafe', description: 'TYPESAFE_API_KEY is not set', disabled: true },
    { value: 'semif', label: 'semif', description: 'judge-semif is not installed' },
  ])
  assert.equal(judgeOptions(providers, 'clef').length, 2)

  const compose = { projects: [{ namespace: 'proj', containers: [{ container: 'judge', state: 'stopped' }] }] }
  const usable = judgeCatalog(listed, 'computer::run')
  assert.equal(judgeSetup(usable, compose), null, 'a registered hub and provider need nothing')
  assert.deepEqual(judgeSetup({ ...usable, hub: false }, compose), { start: ['judge'], add: [] })
  assert.deepEqual(judgeSetup({ ...usable, hub: false }, null), { start: [], add: ['judge'] })
  assert.deepEqual(judgeSetup({ ...usable, providers: [] }, compose), { start: [], add: ['judge-typesafe'] })

  assert.equal(
    operationNote({ phase: 'installing', completed: 1, total: 3, last_event: { container: 'judge', detail: 'downloading' } }),
    'installing · judge · downloading · 1 of 3',
  )
  assert.equal(operationNote({}), null)
})

test('router catalog rows become picker options the way the agent editor maps them', async () => {
  const catalog = await loadModelCatalog()
  const options = catalog.modelOptionsFromCatalog({
    models: [
      {
        id: ' sonnet ',
        provider: 'anthropic',
        display_name: ' Claude Sonnet ',
        context_window: 200000,
        supports_thinking: true,
        supports_vision: false,
        reasoning_efforts: [{ effort: ' low ', description: ' Fast ' }, { effort: '' }, null, { effort: 'high' }],
      },
      { id: 'openai::gpt', provider: 'openai' },
      { id: 'bare' },
      { id: '   ', provider: 'x' },
      null,
    ],
  })
  assert.deepEqual(options, [
    {
      id: 'anthropic::sonnet',
      label: 'Claude Sonnet',
      contextWindow: 200000,
      supportsThinking: true,
      supportsVision: false,
      reasoningEfforts: [{ effort: 'low', description: 'Fast' }, { effort: 'high', description: undefined }],
    },
    {
      id: 'openai::gpt',
      label: 'openai::gpt',
      contextWindow: undefined,
      supportsThinking: undefined,
      supportsVision: undefined,
      reasoningEfforts: undefined,
    },
    {
      id: 'bare',
      label: 'bare',
      contextWindow: undefined,
      supportsThinking: undefined,
      supportsVision: undefined,
      reasoningEfforts: undefined,
    },
  ])
  for (const unreadable of [undefined, null, {}, { models: 'nope' }, 'nope']) {
    assert.deepEqual(catalog.modelOptionsFromCatalog(unreadable), [])
  }
})

test('the stored model stays selectable when the catalog does not list it', async () => {
  const catalog = await loadModelCatalog()
  const listed = [{ id: 'anthropic::sonnet', label: 'Sonnet' }]
  assert.equal(catalog.withStoredModel(listed, null), listed)
  assert.equal(catalog.withStoredModel(listed, 'anthropic::sonnet'), listed)
  assert.deepEqual(catalog.withStoredModel(listed, 'openai::gone'), [
    ...listed,
    { id: 'openai::gone', label: 'openai::gone', supportsThinking: true },
  ])
  assert.deepEqual(catalog.withStoredModel([], 'openai::gone'), [
    { id: 'openai::gone', label: 'openai::gone', supportsThinking: true },
  ])
})

test('queue declares explicit controls for every known adapter', async () => {
  const manifest = await loadManifest()
  const queue = manifest.workerConfigurationSpecs.get('queue')
  const adapter = queue.sections
    .flatMap((section) => section.fields)
    .find((field) => field.path.join('.') === 'adapter')

  assert.equal(adapter.kind, 'variant')
  assert.deepEqual(
    adapter.options.map((option) => option.value),
    ['builtin', 'redis', 'rabbitmq', 'in_memory', 'file_based'],
  )
  assert.equal(
    adapter.options.some((option) => option.fields.some((field) => field.kind === 'structured-value')),
    false,
  )
  assert.deepEqual(
    adapter.options.find((option) => option.value === 'builtin').defaultValue,
    { name: 'builtin', config: { store_method: 'in_memory' } },
  )
})

test('variant selection preserves opaque and future adapter settings', async () => {
  const values = await loadConfigurationValues()
  const opaqueConfig = `\${QUEUE_ADAPTER_CONFIG}`
  const declaredPaths = [
    ['config', 'redis_url'],
    ['config', 'amqp_url'],
  ]
  const rabbitDefaults = {
    name: 'rabbitmq',
    config: { amqp_url: 'amqp://localhost:5672' },
  }

  const opaque = values.selectVariantValue(
    { name: 'redis', config: opaqueConfig, future: true },
    'name',
    'config',
    'rabbitmq',
    rabbitDefaults,
    declaredPaths,
    true,
  )
  assert.deepEqual(opaque, {
    name: 'rabbitmq',
    config: opaqueConfig,
    future: true,
  })

  const futureAdapter = values.selectVariantValue(
    {
      name: 'next-generation',
      config: { amqp_url: 'amqp://future', future_option: { keep: true } },
      future: true,
    },
    'name',
    'config',
    'rabbitmq',
    rabbitDefaults,
    declaredPaths,
    false,
  )
  assert.deepEqual(futureAdapter, {
    name: 'rabbitmq',
    config: {
      amqp_url: 'amqp://future',
      future_option: { keep: true },
    },
    future: true,
  })
})

test('known object variants replace declared fields and retain unknown siblings', async () => {
  const values = await loadConfigurationValues()
  const next = values.selectVariantValue(
    {
      name: 'redis',
      config: {
        redis_url: 'redis://remote',
        future_option: { keep: true },
      },
      future: true,
    },
    'name',
    'config',
    'rabbitmq',
    { name: 'rabbitmq', config: { amqp_url: 'amqp://localhost:5672' } },
    [
      ['config', 'redis_url'],
      ['config', 'amqp_url'],
    ],
    true,
  )

  assert.deepEqual(next, {
    name: 'rabbitmq',
    config: {
      future_option: { keep: true },
      amqp_url: 'amqp://localhost:5672',
    },
    future: true,
  })
})

test('adapter structured-value controls cover every JSON shape without text serialization', async () => {
  const helpers = await loadStructuredValueHelpers()
  const cases = [
    [{ nested: true }, 'object'],
    [[1, 'two'], 'list'],
    ['text', 'string'],
    [42, 'number'],
    [false, 'boolean'],
    [null, 'null'],
  ]
  for (const [value, kind] of cases) {
    assert.equal(helpers.structuredValueKind(value), kind)
    assert.equal(helpers.structuredValueKind(helpers.emptyStructuredValue(kind)), kind)
  }
})

test('renaming a structured adapter key preserves order, nested values and collision safety', async () => {
  const helpers = await loadStructuredValueHelpers()
  const original = {
    redis_url: 'redis://localhost',
    nested: { sentinels: ['one', 'two'] },
  }
  assert.deepEqual(
    helpers.renameStructuredKey(original, 'redis_url', 'endpoint'),
    {
      endpoint: 'redis://localhost',
      nested: { sentinels: ['one', 'two'] },
    },
  )
  assert.equal(
    helpers.renameStructuredKey(original, 'redis_url', 'nested'),
    original,
  )
})

test('legacy computer and tailscale envelopes migrate without losing unrelated values', async () => {
  const values = await loadConfigurationValues()

  for (const [wrapper, editedField, editedValue] of [
    ['computer', 'max_sessions', 9],
    ['tailscale', 'allow_funnel', true],
  ]) {
    const root = {
      tenant: { keep: true },
      [editedField]: 'stale-flat-value',
      [wrapper]: {
        [editedField]: wrapper === 'computer' ? 3 : false,
        future_worker_field: { keep: true },
      },
    }
    const editable = values.legacyConfigurationValue(root, wrapper)
    assert.equal(editable[editedField], wrapper === 'computer' ? 3 : false)

    const migrated = values.migrateLegacyConfiguration(
      root,
      wrapper,
      { ...editable, [editedField]: editedValue },
      [editedField],
    )
    assert.equal(Object.hasOwn(migrated, wrapper), false)
    assert.equal(migrated[editedField], editedValue)
    assert.deepEqual(migrated.tenant, { keep: true })
    assert.deepEqual(migrated.future_worker_field, { keep: true })
  }
})

test('computer and tailscale declarative specs opt into their legacy envelopes', async () => {
  const manifest = await loadManifest()
  assert.equal(manifest.workerConfigurationSpecs.get('computer').legacyWrapper, 'computer')
  assert.equal(manifest.workerConfigurationSpecs.get('tailscale').legacyWrapper, 'tailscale')
})

test('typed controls preserve environment values and derive deliberate literal replacements', async () => {
  const values = await loadConfigurationValues()

  assert.equal(values.isRawTypedValue('${PORT:3111}'), true)
  assert.equal(values.isEnvironmentValue('${PORT}'), true)
  assert.equal(values.numberLiteralForRawValue('${PORT:3111}', 1), 3111)
  assert.equal(values.numberLiteralForRawValue('${PORT}', 7), 7)

  assert.equal(values.booleanLiteralForRawValue('${HEADLESS:true}', false), true)
  assert.equal(values.booleanLiteralForRawValue('${HEADLESS:FALSE}', true), false)
  assert.equal(values.booleanLiteralForRawValue('${HEADLESS}', true), true)

  assert.equal(
    values.selectLiteralForRawValue(
      '${MODE:auto}',
      ['manual', 'auto', 'full'],
      'manual',
    ),
    'auto',
  )
  assert.equal(
    values.selectLiteralForRawValue(
      '${MODE}',
      ['manual', 'auto', 'full'],
      'manual',
    ),
    'manual',
  )
})

test('the setup switch reads on by default and keeps worker_sources when set', async () => {
  const preferences = await loadTracePreferences()
  assert.equal(preferences.onboardingAutoOpen({}), true)
  assert.equal(preferences.onboardingAutoOpen({ onboarding: { auto_open: true } }), true)
  assert.equal(preferences.onboardingAutoOpen({ onboarding: { auto_open: false } }), false)

  const sources = { judge: '/src/judge' }
  const off = preferences.withOnboardingAutoOpen({ http_port: 3113, onboarding: { worker_sources: sources } }, false)
  assert.deepEqual(off, { http_port: 3113, onboarding: { worker_sources: sources, auto_open: false } })
  assert.equal(preferences.onboardingAutoOpen(off), false)
})

test('trace preference edits preserve opaque view and sibling fields', async () => {
  const preferences = await loadTracePreferences()
  const value = {
    futureRoot: { keep: true },
    traces: {
      futureTracePreference: 'keep',
      activeViewId: 'view-a',
      views: [
        {
          id: 'view-a',
          name: 'Original',
          groupBy: 'iii.session.id',
          filters: { status: 'error' },
          futureViewField: { keep: true },
        },
      ],
      spanFilters: {
        hiddenGroups: ['old'],
        futureSpanFilter: { keep: true },
      },
    },
  }

  const renamed = preferences.renameTraceView(value, 'view-a', 'Renamed')
  assert.equal(renamed.traces.views[0].name, 'Renamed')
  assert.deepEqual(renamed.traces.views[0].futureViewField, { keep: true })
  assert.deepEqual(renamed.traces.views[0].filters, { status: 'error' })

  const filtered = preferences.withTraceFilterList(
    renamed,
    'hiddenWorkers',
    ['console', 'console', '  context-manager  '],
  )
  assert.deepEqual(filtered.traces.spanFilters.hiddenWorkers, [
    'console',
    'context-manager',
  ])
  assert.deepEqual(filtered.traces.spanFilters.futureSpanFilter, { keep: true })
  assert.equal(filtered.traces.futureTracePreference, 'keep')
  assert.deepEqual(filtered.futureRoot, { keep: true })
})

test('removing the active trace view selects all traces without rewriting siblings', async () => {
  const preferences = await loadTracePreferences()
  const value = {
    traces: {
      activeViewId: 'view-a',
      views: [
        { id: 'view-a', name: 'A', custom: true },
        { id: 'view-b', name: 'B', custom: true },
      ],
      followTurns: true,
    },
  }
  const next = preferences.removeTraceView(value, 'view-a')
  assert.equal(next.traces.activeViewId, null)
  assert.deepEqual(next.traces.views, [
    { id: 'view-b', name: 'B', custom: true },
  ])
  assert.equal(next.traces.followTurns, true)
})

test('telegram aliases migrate with Rust timeout precedence and preserve future fields', async () => {
  const normalization = await loadConfigurationNormalization()
  const value = {
    bot_token: 'token',
    harness_send_timeout_ms: 1200,
    approval_timeout_ms: 2300,
    state_timeout_ms: 3400,
    thinking_display: 'compact',
    use_rich: true,
    edit_throttle_ms: 50,
    futureRoot: { keep: true },
    updates: {
      name: 'webhook',
      futureAdapterField: ['keep'],
      config: {
        url: 'https://engine.example/telegram-bot/webhook',
        secret: 'secret',
        futureWebhookField: { keep: true },
      },
    },
  }

  const next = normalization.normalizeTelegramBotConfiguration(value)

  assert.equal(next.timeout_ms, 1200)
  assert.equal(next.harness_send_timeout_ms, undefined)
  assert.equal(next.approval_timeout_ms, undefined)
  assert.equal(next.state_timeout_ms, undefined)
  assert.equal(next.thinking_display, undefined)
  assert.equal(next.use_rich, undefined)
  assert.equal(next.edit_throttle_ms, undefined)
  assert.deepEqual(next.futureRoot, { keep: true })
  assert.equal(
    next.updates.config.base_url,
    'https://engine.example/telegram-bot/webhook',
  )
  assert.equal(next.updates.config.url, undefined)
  assert.equal(next.updates.config.secret, 'secret')
  assert.deepEqual(next.updates.config.futureWebhookField, { keep: true })
  assert.deepEqual(next.updates.futureAdapterField, ['keep'])
})

test('telegram canonical keys win while known aliases are removed', async () => {
  const normalization = await loadConfigurationNormalization()
  const value = {
    timeout_ms: 9000,
    harness_send_timeout_ms: 1200,
    futureRoot: 'keep',
    updates: {
      name: 'webhook',
      config: {
        base_url: 'https://canonical.example',
        url: 'https://legacy.example/telegram-bot/webhook',
        futureWebhookField: 42,
      },
    },
  }

  const next = normalization.normalizeTelegramBotConfiguration(value)

  assert.equal(next.timeout_ms, 9000)
  assert.equal(next.harness_send_timeout_ms, undefined)
  assert.equal(next.futureRoot, 'keep')
  assert.equal(next.updates.config.base_url, 'https://canonical.example')
  assert.equal(next.updates.config.url, undefined)
  assert.equal(next.updates.config.futureWebhookField, 42)
})

test('telegram null timeout aliases follow Option fallback without coercion', async () => {
  const normalization = await loadConfigurationNormalization()
  const value = {
    timeout_ms: null,
    harness_send_timeout_ms: null,
    approval_timeout_ms: 2300,
    state_timeout_ms: 3400,
  }

  const next = normalization.normalizeTelegramBotConfiguration(value)
  assert.equal(next.timeout_ms, 2300)
  assert.equal(next.harness_send_timeout_ms, undefined)
  assert.equal(next.approval_timeout_ms, undefined)
  assert.equal(next.state_timeout_ms, undefined)
})

test('github normalization shows effective notification defaults without overriding stored values', async () => {
  const normalization = await loadConfigurationNormalization()
  const stored = {
    token: '',
    webhooks: { enabled: true, notifications: { profile: 'agent_actionable', quiet_ms: 5000 }, future: 1 },
  }
  const shown = normalization.normalizeWorkerConfiguration('github', stored)
  assert.equal(shown.webhooks.enabled, true)
  assert.equal(shown.webhooks.future, 1)
  assert.equal(shown.webhooks.queue, 'github-webhooks')
  assert.equal(shown.webhooks.notifications.profile, 'agent_actionable')
  assert.equal(shown.webhooks.notifications.quiet_ms, 5000)
  assert.equal(shown.webhooks.notifications.max_wait_ms, 120000)
  assert.equal(shown.webhooks.notifications.suppress_bot_noise, true)
  assert.deepEqual(shown.webhooks.notifications.ignored_actors, [])
  assert.equal(stored.webhooks.notifications.max_wait_ms, undefined, 'input is not mutated')
  const legacy = normalization.normalizeWorkerConfiguration('github', { token: '' })
  assert.equal(legacy.webhooks.notifications.profile, 'all')
  assert.equal(normalization.normalizeWorkerConfiguration('github', shown), shown, 'complete values are stable')
})

test('normalization leaves unrelated and already canonical values untouched', async () => {
  const normalization = await loadConfigurationNormalization()
  const canonical = {
    timeout_ms: 5000,
    updates: {
      name: 'webhook',
      config: { base_url: 'https://engine.example', future: true },
    },
    futureRoot: { keep: true },
  }
  const otherWorker = {
    url: 'https://example.test',
    approval_timeout_ms: 1234,
  }
  const futureAdapter = {
    updates: {
      name: 'future-adapter',
      config: { url: 'future-specific-value', keep: true },
    },
  }

  assert.equal(
    normalization.normalizeTelegramBotConfiguration(canonical),
    canonical,
  )
  assert.equal(
    normalization.normalizeWorkerConfiguration('http', otherWorker),
    otherWorker,
  )
  assert.equal(
    normalization.normalizeTelegramBotConfiguration('future-shape'),
    'future-shape',
  )
  assert.equal(
    normalization.normalizeTelegramBotConfiguration(futureAdapter),
    futureAdapter,
  )
})
