// Configuration-worker integration. Registers openwiki's config schema so the
// default model and page-writer concurrency are editable in the console and
// hot-reload on change. Env vars seed the defaults on first registration.
const CONFIG_ID = 'openwiki';
const CONFIG_FN_ID = 'openwiki::on-config-change';

// Sanitize env seeds against the declared schema: a NaN or out-of-range
// max_parallel, or an unknown refresh cadence, would make the registered
// defaults/initial_value violate the schema itself.
const REFRESH_VALUES = ['off', '3h', '6h', '12h', 'daily', 'weekly'];
const rawParallel = parseInt(process.env.OPENWIKI_MAX_PARALLEL || '3', 10);
const envRefresh = process.env.OPENWIKI_REFRESH_DEFAULT || 'off';

const DEFAULTS = {
  model: process.env.OPENWIKI_MODEL || 'claude-haiku-4-5-20251001',
  max_parallel: Math.min(16, Math.max(1, Number.isFinite(rawParallel) ? rawParallel : 3)),
  refresh_default: REFRESH_VALUES.includes(envRefresh) ? envRefresh : 'off',
};

function schema() {
  return {
    type: 'object',
    additionalProperties: false,
    properties: {
      model: {
        type: 'string',
        description: 'Default generation model id, routed via llm-router (e.g. claude-haiku-4-5-20251001).',
        default: DEFAULTS.model,
      },
      max_parallel: {
        type: 'integer',
        minimum: 1,
        maximum: 16,
        description: 'Concurrent page writers per generation.',
        default: DEFAULTS.max_parallel,
      },
      refresh_default: {
        type: 'string',
        enum: ['off', '3h', '6h', '12h', 'daily', 'weekly'],
        description:
          'Default auto-refresh cadence for new wikis. Each wiki can override it in the UI. "off" means no scheduled refresh.',
        default: DEFAULTS.refresh_default,
      },
    },
  };
}

export function defaults() {
  return { ...DEFAULTS };
}

export async function registerConfig(iii) {
  await iii.trigger({
    function_id: 'configuration::register',
    payload: {
      id: CONFIG_ID,
      name: 'OpenWiki',
      description: 'OpenWiki worker: default model, page-writer concurrency, and auto-refresh cadence.',
      schema: schema(),
      initial_value: DEFAULTS,
    },
  });
}

export async function fetchConfig(iii) {
  try {
    const res = await iii.trigger({ function_id: 'configuration::get', payload: { id: CONFIG_ID, raw: false } });
    const v = res && typeof res === 'object' && 'value' in res ? res.value : res;
    return { ...DEFAULTS, ...(v || {}) };
  } catch {
    return { ...DEFAULTS };
  }
}

export function bindConfigTrigger(iii, onChange) {
  iii.registerFunction(
    CONFIG_FN_ID,
    async () => {
      await onChange();
      return { reloaded: true };
    },
    {
      description: 'Reload runtime config when the openwiki configuration entry changes.',
      request_format: { type: 'object', additionalProperties: true, properties: {} },
      response_format: {
        type: 'object',
        additionalProperties: false,
        required: ['reloaded'],
        properties: { reloaded: { type: 'boolean' } },
      },
    },
  );
  try {
    iii.registerTrigger({
      type: 'configuration',
      function_id: CONFIG_FN_ID,
      config: { configuration_id: CONFIG_ID, event_types: ['configuration:updated'] },
    });
  } catch {
    /* configuration worker may be absent; env defaults apply */
  }
}
