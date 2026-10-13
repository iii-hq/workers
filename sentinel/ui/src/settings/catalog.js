/**
 * The model catalog behind the investigation picker: the pure half.
 *
 * Two rules shape it. A stored model that has left the catalog is **kept and
 * shown**, never silently dropped — a provider whose credential was cleared
 * comes back, and a form that quietly emptied the field would have thrown the
 * choice away. And the value written back is the pair the worker's schema
 * already has, `model` plus `provider`, rather than a joined string the
 * worker would have to split.
 */

/**
 * Joins the two stored fields into the key the picker selects by.
 * @param {string | undefined | null} model
 * @param {string | undefined | null} provider
 */
export function catalogKey(model, provider) {
  const id = (model ?? '').trim()
  if (!id) return ''
  const owner = (provider ?? '').trim()
  return owner ? `${owner}::${id}` : id
}

/**
 * And splits it back into the two fields the configuration stores.
 * @param {string | undefined | null} key
 * @returns {{ model: string, provider: string }}
 */
export function splitKey(key) {
  const value = key ?? ''
  const index = value.indexOf('::')
  if (index <= 0) return { model: value.trim(), provider: '' }
  return { model: value.slice(index + 2).trim(), provider: value.slice(0, index).trim() }
}

/**
 * Rows from `router::models::list` as options for the console's
 * `ModelPicker`, keyed like the stored pair, dropping anything that cannot
 * be addressed. An unreachable router yields nothing, and the picker says it
 * is empty rather than pretending.
 * @param {unknown} response
 * @returns {import('@iii-dev/console-ui').ModelOption[]}
 */
export function readCatalog(response) {
  const rows =
    response && typeof response === 'object' ? /** @type {any} */ (response).models : null
  if (!Array.isArray(rows)) return []
  /** @type {import('@iii-dev/console-ui').ModelOption[]} */
  const models = []
  for (const raw of rows) {
    if (!raw || typeof raw !== 'object') continue
    const row = /** @type {Record<string, unknown>} */ (raw)
    const id = typeof row.id === 'string' ? row.id.trim() : ''
    const provider = typeof row.provider === 'string' ? row.provider.trim() : ''
    if (!id || !provider) continue
    const display =
      typeof row.display_name === 'string' && row.display_name.trim()
        ? row.display_name.trim()
        : id
    models.push({
      id: `${provider}::${id}`,
      label: display,
      contextWindow: typeof row.context_window === 'number' ? row.context_window : undefined,
      supportsThinking: typeof row.supports_thinking === 'boolean' ? row.supports_thinking : undefined,
      supportsVision: typeof row.supports_vision === 'boolean' ? row.supports_vision : undefined,
    })
  }
  models.sort((left, right) => left.id.localeCompare(right.id))
  return models
}

/**
 * The catalog plus the stored model when the router does not offer it. Kept
 * on purpose: the picker shows only a value it has an option for, and a
 * provider that is down, or an id typed into the configuration by hand, is
 * still the operator's choice until they change it.
 * @param {import('@iii-dev/console-ui').ModelOption[]} options
 * @param {string} selected The current `catalogKey`, possibly absent from the catalog.
 */
export function withStoredModel(options, selected) {
  if (!selected || options.some((option) => option.id === selected)) return options
  const name = splitKey(selected).model || selected
  return [...options, { id: selected, label: `${name} (not offered now)` }]
}
