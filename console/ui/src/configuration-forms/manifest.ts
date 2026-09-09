import { agentCollectionWorkerSpecs } from './specs/collections-agents'
import { serviceCollectionWorkerSpecs } from './specs/collections-services'
import { nestedWorkerSpecs } from './specs/nested'
import { observabilityWorkerSpec } from './specs/observability'
import { scalarWorkerSpecs } from './specs/scalar'
import type { FormFieldSpec, WorkerConfigurationSpec } from './types'

export const workerConfigurationManifest = [
  ...scalarWorkerSpecs,
  ...nestedWorkerSpecs,
  ...agentCollectionWorkerSpecs,
  ...serviceCollectionWorkerSpecs,
  observabilityWorkerSpec,
] as const satisfies readonly WorkerConfigurationSpec[]

export const workerConfigurationIds = workerConfigurationManifest.map((spec) => spec.id)

export const workerConfigurationSpecs = new Map(workerConfigurationManifest.map((spec) => [spec.id, spec]))

function join(prefix: string, path: readonly string[]): string {
  const suffix = path.join('.')
  return prefix && suffix ? `${prefix}.${suffix}` : prefix || suffix
}

function collectField(field: FormFieldSpec, prefix: string): string[] {
  const path = join(prefix, field.path)
  if (
    field.kind === 'text' ||
    field.kind === 'password' ||
    field.kind === 'number' ||
    field.kind === 'select' ||
    field.kind === 'switch'
  ) {
    return [path]
  }
  if (field.kind === 'string-list') return [`${path}[]`]
  if (field.kind === 'dynamic-map') return [`${path}.*`]
  if (field.kind === 'structured-value') return [`${path}.*`]
  if (field.kind === 'filter-list') {
    return [`${path}[].match`, `${path}[].metadata.*`]
  }
  if (field.kind === 'permission-rules') {
    return [
      `${path}[].shorthand`,
      `${path}[].function`,
      `${path}[].action`,
      `${path}[].rule_id`,
      `${path}[].modes[]`,
      `${path}[].args.*`,
    ]
  }
  if (field.kind === 'object') {
    return field.fields.flatMap((child) => collectField(child, path))
  }
  if (field.kind === 'variant') {
    const discriminator = field.discriminator ?? 'name'
    return [
      `${path}.${discriminator}`,
      ...field.options.flatMap((option) => option.fields.flatMap((child) => collectField(child, path))),
    ]
  }
  if (field.kind === 'object-list' || field.kind === 'object-map') {
    const collectionPath = field.kind === 'object-list' ? `${path}[]` : `${path}.*`
    return field.fields.flatMap((child) => collectField(child, collectionPath))
  }
  return []
}

export function declaredFields(spec: WorkerConfigurationSpec): string[] {
  return [
    ...new Set(spec.sections.flatMap((section) => section.fields.flatMap((field) => collectField(field, '')))),
  ].sort()
}

export function validateWorkerConfigurationManifest(): void {
  if (workerConfigurationManifest.length !== 40) {
    throw new Error(`Expected 40 worker configuration forms, found ${workerConfigurationManifest.length}`)
  }

  const ids = new Set<string>()
  for (const spec of workerConfigurationManifest) {
    if (spec.id === 'shell-ui') {
      throw new Error('shell-ui is internal state and must not expose a form')
    }
    if (ids.has(spec.id)) throw new Error(`Duplicate configuration form: ${spec.id}`)
    ids.add(spec.id)

    const declared = declaredFields(spec)
    const expected = [...new Set(spec.expectedFields)].sort()
    if (declared.join('\n') !== expected.join('\n')) {
      const missing = expected.filter((field) => !declared.includes(field))
      const unexpected = declared.filter((field) => !expected.includes(field))
      throw new Error(
        `${spec.id} field parity failed; missing=[${missing.join(', ')}] unexpected=[${unexpected.join(', ')}]`,
      )
    }
  }
}
