import { describe, expect, it } from 'vitest'
import {
  WORKSPACE_LIST_FUNCTION_ID,
  WORKSPACE_ROOTS_FUNCTION_ID,
  WORKSPACE_VALIDATE_FUNCTION_ID,
} from './DirectoryPicker'

describe('DirectoryPicker workspace API wiring', () => {
  it('uses shell workspace control-plane functions', () => {
    expect(WORKSPACE_ROOTS_FUNCTION_ID).toBe('shell::workspace::roots')
    expect(WORKSPACE_LIST_FUNCTION_ID).toBe('shell::workspace::list')
    expect(WORKSPACE_VALIDATE_FUNCTION_ID).toBe('shell::workspace::validate')
  })
})
