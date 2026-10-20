import type { Choice } from '../types'

/**
 * `sandbox::catalog::list` rows (`{ images: [{ name, oci_ref, kind }] }`) as
 * choices: the name is what `sandbox::create` takes; the OCI ref behind it is
 * iii-sandbox's own configuration, shown so the operator sees what boots.
 * Presets (python, node) are code runners, not desktops: only `custom_images`.
 */
export function sandboxCatalogChoices(response: unknown): readonly Choice[] {
  const images = (response as { images?: unknown } | null | undefined)?.images
  if (!Array.isArray(images)) return []
  return images.flatMap((raw) => {
    if (!raw || typeof raw !== 'object') return []
    const row = raw as Record<string, unknown>
    if (row.kind === 'preset') return []
    const name = typeof row.name === 'string' ? row.name.trim() : ''
    if (!name) return []
    const ref = typeof row.oci_ref === 'string' ? row.oci_ref.trim() : ''
    return [{ value: name, label: ref ? `${name} — ${ref}` : name }]
  })
}
