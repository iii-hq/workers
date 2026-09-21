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

/** @typedef {{ key: string, id: string, provider: string, label: string }} CatalogModel */

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
 * Rows from `router::models::list` into catalog entries, dropping anything
 * that cannot be addressed. An unreachable router yields nothing, and the
 * picker says it is empty rather than pretending.
 * @param {unknown} response
 * @returns {CatalogModel[]}
 */
export function readCatalog(response) {
  const rows =
    response && typeof response === 'object' ? /** @type {any} */ (response).models : null
  if (!Array.isArray(rows)) return []
  /** @type {CatalogModel[]} */
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
    models.push({ key: `${provider}::${id}`, id, provider, label: display })
  }
  models.sort((left, right) => left.key.localeCompare(right.key))
  return models
}

/**
 * The picker's groups, one per provider, plus a group of its own for a stored
 * model the catalog does not offer.
 * @param {CatalogModel[]} catalog
 * @param {string} selected The current `catalogKey`, possibly absent from the catalog.
 */
export function modelGroups(catalog, selected) {
  /** @type {Map<string, { label: string, options: { value: string, label: string, description?: string }[] }>} */
  const byProvider = new Map()
  for (const model of catalog) {
    const group = byProvider.get(model.provider) ?? { label: model.provider, options: [] }
    group.options.push({ value: model.key, label: model.label })
    byProvider.set(model.provider, group)
  }
  const groups = [...byProvider.values()]
  if (selected && !catalog.some((model) => model.key === selected)) {
    // Kept on purpose: a provider that is down, or an id typed by hand, is
    // still the operator's choice until they change it.
    groups.unshift({
      label: 'configured',
      options: [
        {
          value: selected,
          label: splitKey(selected).model || selected,
          description: catalog.length
            ? 'not offered by the router right now'
            : 'the router catalog is not loaded',
        },
      ],
    })
  }
  return groups
}
