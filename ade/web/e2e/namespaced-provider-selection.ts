export const namespacedProviderSpecs = [
  'e2e/provider-configuration-portable.spec.ts',
  'e2e/provider-configuration-visual.spec.ts',
  'e2e/model-picker-keyboard.spec.ts',
] as const

export const namespacedProviderSpecPattern = `**/{${namespacedProviderSpecs
  .map((spec) => spec.slice('e2e/'.length))
  .join(',')}}`

export type NamespacedProviderEnvironment = {
  CONSOLE_E2E_READY_FILE?: string
  READY_FILE?: string
}

export function hasNamespacedProviderConfiguration(
  environment: NamespacedProviderEnvironment = process.env,
): boolean {
  return (
    environment.CONSOLE_E2E_READY_FILE !== undefined ||
    environment.READY_FILE !== undefined
  )
}

export function genericConsoleTestIgnore(
  environment: NamespacedProviderEnvironment = process.env,
): string[] {
  return hasNamespacedProviderConfiguration(environment)
    ? []
    : [namespacedProviderSpecPattern]
}
