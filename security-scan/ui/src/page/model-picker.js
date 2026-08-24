/** @typedef {import('./security-scan-data').CatalogModel} CatalogModel */

/**
 * Sentinel selection: keep taking the model from the open chat composer (and
 * the operator default when that chat has none). Leading space keeps it out of
 * the `provider::id` namespace, so a catalog entry can never collide with it.
 */
export const FOLLOW_CHAT = ' follow-chat'

/**
 * @param {CatalogModel[]} catalog
 * @param {string} followLabel Human label for what following the chat resolves to.
 */
export function modelPickerOptions(catalog, followLabel) {
  return [
    { value: FOLLOW_CHAT, label: `follow chat · ${followLabel}` },
    ...catalog.map((model) => ({ value: model.key, label: model.label })),
  ]
}

/**
 * A pinned model that has left the catalog (provider removed, credential
 * cleared) must not be sent: the router would reject an id it no longer
 * serves. An empty catalog is "not loaded yet", not "model is gone".
 *
 * @param {CatalogModel[]} catalog
 * @param {string} selection
 */
export function selectionIsStale(catalog, selection) {
  if (selection === FOLLOW_CHAT) return false
  if (catalog.length === 0) return false
  return !catalog.some((model) => model.key === selection)
}

/**
 * What `security-scan::request` should carry. A pinned catalog key is sent as
 * `model` alone — the worker splits `provider::id` when no explicit provider
 * accompanies it. Following the chat with no composer model sends nothing, so
 * the worker applies the operator default.
 *
 * @param {string} selection
 * @param {string | null} composerModel
 * @returns {string | null}
 */
export function requestedModel(selection, composerModel) {
  if (selection !== FOLLOW_CHAT) return selection
  const model = composerModel?.trim()
  return model ? model : null
}
