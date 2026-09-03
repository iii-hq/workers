import type { ConfigurationSchemaView } from '../tabs/WorkersTab/api'

const INTERNAL_CONFIGURATION_IDS = new Set(['coder', 'shell-ui'])

/**
 * Configuration records that belong to the Console implementation rather than
 * an operator-managed worker are never exposed in the settings catalog.
 */
export function isOperatorConfiguration(
  entry: Pick<ConfigurationSchemaView, 'id'>,
): boolean {
  return !INTERNAL_CONFIGURATION_IDS.has(entry.id)
}
