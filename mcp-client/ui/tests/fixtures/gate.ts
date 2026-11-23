import type { AccessClass, AccessSource, FunctionAccess } from '../../src/access'

export const L = (name: string) => `mcp::linear::${name}`
export const A = (name: string) => `mcp::linear-app::${name}`
export const rule = (id: string) => ({ function: id, action: 'allow', rule_id: `mcp-client:${id}` })
export const fa = (
  name: string,
  cls: AccessClass,
  source: AccessSource,
  kind: FunctionAccess['kind'] = 'tool',
): FunctionAccess => ({ functionId: L(name), kind, name, cls, source })
