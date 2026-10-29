import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import {
  loadApprovalGateConfig,
  saveApprovalGateDefaults,
} from './backend/approval-gate-config'
import { getShellHostRoots } from './backend/shell-roots'
import {
  fetchConsoleConfigValue,
  setConsoleConfigValue,
} from './console-config'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
const call = vi.fn()
beforeEach(() => {
  call.mockReset()
  vi.mocked(getIiiClient).mockResolvedValue({ call } as unknown as Awaited<
    ReturnType<typeof getIiiClient>
  >)
})

describe('namespace-scoped configuration consumers', () => {
  it('Console reads and writes the identity supplied by its backend', async () => {
    call.mockImplementation(async (fn) => {
      if (fn === 'console::configuration-id')
        return { id: 'project-console-hash' }
      if (fn === 'configuration::get') return { value: { http_port: 3213 } }
      return {}
    })
    expect(await fetchConsoleConfigValue()).toEqual({ http_port: 3213 })
    await setConsoleConfigValue({ http_port: 3313 })
    expect(call).toHaveBeenCalledWith('configuration::get', {
      id: 'project-console-hash',
      raw: true,
    })
    expect(call).toHaveBeenCalledWith('configuration::set', {
      id: 'project-console-hash',
      value: { http_port: 3313 },
    })
  })

  it('does not fall back to a global Console entry when the owner is missing', async () => {
    call.mockRejectedValue(new Error('function_not_found'))
    expect(await fetchConsoleConfigValue()).toBeNull()
    await expect(setConsoleConfigValue({})).rejects.toThrow(
      'function_not_found',
    )
    expect(
      call.mock.calls.every(([fn]) => fn === 'console::configuration-id'),
    ).toBe(true)
  })

  it('folder permissions are read from the addressed IDE', async () => {
    call.mockImplementation(async (fn) =>
      fn === 'ide::configuration-id'
        ? { id: 'custom-ide' }
        : { value: { fs: { host_roots: ['/project', 1] } } },
    )
    expect(await getShellHostRoots()).toEqual(['/project'])
    expect(call).toHaveBeenCalledWith('configuration::get', {
      id: 'custom-ide',
      raw: false,
    })
  })

  it('permission edits preserve sibling settings and use a single resolved ID', async () => {
    const existing = {
      default_mode: 'manual',
      rules: [],
      timeout: 9000,
      // biome-ignore lint/suspicious/noTemplateCurlyInString: persisted environment template, not JS interpolation
      token: '${TOKEN}',
    }
    call.mockImplementation(async (fn) => {
      if (fn === 'approval-gate::configuration-id')
        return { id: 'project-approvals' }
      if (fn === 'configuration::get') return { value: existing }
      return {}
    })
    expect((await loadApprovalGateConfig()).default_mode).toBe('manual')
    call.mockClear()
    await saveApprovalGateDefaults('auto', ['state::get'])
    expect(
      call.mock.calls.filter(
        ([fn]) => fn === 'approval-gate::configuration-id',
      ),
    ).toHaveLength(1)
    expect(call).toHaveBeenCalledWith('configuration::set', {
      id: 'project-approvals',
      value: {
        ...existing,
        default_mode: 'auto',
        rules: [{ function: 'state::get', action: 'allow', modes: ['auto'] }],
      },
    })
  })
})
