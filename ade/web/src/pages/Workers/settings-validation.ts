import { isMap, parseDocument } from 'yaml'
import type { ContainerEntry } from './compose-api'
import type { SettingsDraft } from './model'

export type SettingsValidation = {
  env: Record<number, string>
  config: string | null
  configDetail?: string
  configPosition?: { line: number; column: number }
  valid: boolean
}

export function validateSettingsDraft(
  _entry: ContainerEntry,
  draft: SettingsDraft,
): SettingsValidation {
  const env: Record<number, string> = {}
  const keys = new Map<string, number>()
  draft.env.forEach((row, index) => {
    if (row.removed) return
    if (!row.key.trim()) {
      env[index] = 'Enter a variable name or remove this row.'
      return
    }
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(row.key)) {
      env[index] =
        'Start with a letter or underscore; use only letters, numbers and underscores.'
      return
    }
    const previous = keys.get(row.key)
    if (previous !== undefined) {
      const message = `Duplicate name "${row.key}". Rename or remove one of these variables.`
      env[previous] = message
      env[index] = message
    } else {
      keys.set(row.key, index)
    }
    if (row.secret && row.replacing && row.value === '') {
      env[index] ??= 'Enter a new secret value or cancel replacement.'
    }
  })

  let config: string | null = null
  let configDetail: SettingsValidation['configDetail']
  let configPosition: SettingsValidation['configPosition']
  const document = parseDocument(draft.config, {
    uniqueKeys: true,
    merge: true,
  })
  const error = document.errors[0]
  if (error) {
    const position = error.linePos?.[0]
    if (position) configPosition = { line: position.line, column: position.col }
    const where = position
      ? ` at line ${position.line}, column ${position.col}`
      : ''
    let message = error.message
      .split('\n')[0]
      .replace(/ at line \d+, column \d+:?$/, '')
    if (
      error.code === 'BAD_INDENT' &&
      message ===
        'Flow sequence in block collection must be sufficiently indented and end with a ]'
    ) {
      configDetail = message
      message = 'Check the list’s indentation and close it with ].'
    }
    config = `Fix YAML${where}: ${message}`
  } else if (document.contents !== null && !isMap(document.contents)) {
    config =
      'Use a YAML mapping of key: value settings, or leave the override empty.'
  } else {
    try {
      // Parsing alone does not resolve aliases or validate their merge targets.
      document.toJS()
    } catch (cause) {
      config = `Fix YAML: ${cause instanceof Error ? cause.message : String(cause)}`
    }
  }
  return {
    env,
    config,
    valid: Object.keys(env).length === 0 && config === null,
    ...(configDetail ? { configDetail } : {}),
    ...(configPosition ? { configPosition } : {}),
  }
}
