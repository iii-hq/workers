import { describe, expect, it } from 'vitest'
import {
  actionRequiredText,
  approvedAlwaysText,
  filesystemDecisionText,
} from './approval-announcements'

describe('approval announcements', () => {
  it('name the call namespace when one is set', () => {
    expect(actionRequiredText('state::list', 'laptop')).toBe(
      'Action required: approve or deny state::list in namespace laptop.',
    )
    expect(actionRequiredText('shell::fs::read', 'laptop', '/data')).toBe(
      'Action required: shell::fs::read in namespace laptop needs approval to access /data.',
    )
    expect(approvedAlwaysText('state::list', 'laptop')).toBe(
      'approved always this session: state::list in namespace laptop',
    )
    expect(
      filesystemDecisionText('allowed /data for this session', 'laptop'),
    ).toBe('allowed /data for this session; the call is in namespace laptop')
  })

  it('stay unchanged without a namespace', () => {
    expect(actionRequiredText('state::list')).toBe(
      'Action required: approve or deny state::list.',
    )
    expect(actionRequiredText('shell::fs::read', undefined, '/data')).toBe(
      'Action required: shell::fs::read needs approval to access /data.',
    )
    expect(approvedAlwaysText('state::list')).toBe(
      'approved always this session: state::list',
    )
    expect(filesystemDecisionText('denied filesystem access')).toBe(
      'denied filesystem access',
    )
  })
})
