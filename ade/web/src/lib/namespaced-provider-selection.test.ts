import { describe, expect, it } from 'vitest'
import {
  genericConsoleTestIgnore,
  hasNamespacedProviderConfiguration,
  namespacedProviderSpecPattern,
  namespacedProviderSpecs,
} from '../../e2e/namespaced-provider-selection'

describe('namespaced provider Playwright selection', () => {
  it('keeps every stack-dependent spec out of the generic Console run', () => {
    expect(genericConsoleTestIgnore({})).toEqual([
      namespacedProviderSpecPattern,
    ])
    expect(namespacedProviderSpecs).toEqual([
      'e2e/provider-configuration-portable.spec.ts',
      'e2e/provider-configuration-visual.spec.ts',
      'e2e/model-picker-keyboard.spec.ts',
    ])
  })

  it('opts into every namespaced assertion only with a manifest path', () => {
    expect(
      hasNamespacedProviderConfiguration({
        CONSOLE_E2E_READY_FILE: '/tmp/provider-e2e/ready.json',
      }),
    ).toBe(true)
    expect(
      genericConsoleTestIgnore({
        READY_FILE: '/tmp/provider-e2e/ready.json',
      }),
    ).toEqual([])
  })

  it('keeps explicitly blank manifest configuration selected for runtime failure', () => {
    expect(
      genericConsoleTestIgnore({
        CONSOLE_E2E_READY_FILE: '',
      }),
    ).toEqual([])
  })
})
