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
  'iii-observability',
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
  'scrapling',
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

test('manifest covers the 40 worker-owned configuration entries', async () => {
  const manifest = await loadManifest()
  assert.doesNotThrow(() => manifest.validateWorkerConfigurationManifest())
  assert.equal(manifest.workerConfigurationManifest.length, 40)
  assert.equal(new Set(manifest.workerConfigurationIds).size, 40)
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
