export interface ConfigurationFamilyEntry {
  id: string
  metadata?: unknown
}

/** Stable injected-form family carried by worker registration metadata. */
export function configurationFormFamily(
  entry: ConfigurationFamilyEntry,
): string {
  const metadata = entry.metadata
  if (
    metadata !== null &&
    typeof metadata === 'object' &&
    !Array.isArray(metadata)
  ) {
    const candidate = Reflect.get(metadata, 'ui_form')
    if (typeof candidate === 'string' && candidate.trim()) {
      return candidate.trim()
    }
  }
  return entry.id
}

export type ConfigurationFamilyResolution =
  | { kind: 'resolved'; id: string }
  | { kind: 'missing' }
  | { kind: 'ambiguous'; ids: string[] }

/**
 * Resolve a page/form family to the live configuration entry that owns it.
 *
 * A literal id remains a valid match, while a renamed `III_CONFIG_NAME`
 * instance matches through metadata. The family resolves only when exactly
 * one live entry matches. Multiple instances deliberately remain unresolved:
 * the pane action must show the worker list instead of silently editing an
 * arbitrary instance (including when one instance uses the default id).
 */
export function resolveConfigurationFamily(
  familyId: string,
  entries: readonly ConfigurationFamilyEntry[],
): ConfigurationFamilyResolution {
  const ids = entries
    .filter(
      (entry) =>
        entry.id === familyId || configurationFormFamily(entry) === familyId,
    )
    .map((entry) => entry.id)

  if (ids.length === 1) return { kind: 'resolved', id: ids[0] }
  if (ids.length === 0) return { kind: 'missing' }
  return { kind: 'ambiguous', ids }
}
