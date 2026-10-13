import { describe, expect, it } from 'vitest'
import type { ContainerEntry } from './compose-api'
import { captureDiscardedDraft, restoreDiscardedDraft } from './discarded-draft'
import {
  draftFrom,
  hasSettingsChanges,
  MASK,
  settingsPatch,
  validateSettingsDraft,
} from './model'

// biome-ignore lint/suspicious/noTemplateCurlyInString: Compose environment references stay literal in the draft.
const tokenReference = '${TOKEN}'

const entry: ContainerEntry = {
  name: 'http',
  worker: '../http',
  version: null,
  run: 'original command',
  start_after: ['state'],
  env_file: ['workers.env'],
  environment: [
    { key: 'LOG', value: 'info', secret: false },
    { key: 'SAVED_TOKEN', value: null, secret: true },
    { key: 'REMOVE', value: 'present', secret: false },
  ],
  config_override: 'port: 3111\n',
}

describe('discarded settings drafts', () => {
  it('isolates the captured entry and draft from later source and snapshot edits', () => {
    const original = structuredClone(entry)
    const draft = draftFrom(original)
    draft.run = 'pending command'
    draft.env[0].value = 'debug'
    const before = structuredClone({ entry: original, draft })
    const snapshot = captureDiscardedDraft(original, draft)
    expect(snapshot).toEqual(before)
    snapshot.entry.start_after.push('queue')
    snapshot.entry.env_file.push('other.env')
    snapshot.entry.environment[0].value = 'snapshot-only'
    snapshot.draft.startAfter.push('http')
    snapshot.draft.env[0].value = 'snapshot-only'
    expect({ entry: original, draft }).toEqual(before)
    original.environment[2].value = 'source-only'
    draft.env[2].value = 'source-only'
    expect(snapshot.entry.environment[2].value).toBe('present')
    expect(snapshot.draft.env[2].value).toBe('present')
  })

  it('recovers the exact multi-field draft including invalid YAML and unfinished environment edits', () => {
    const draft = draftFrom(entry)
    draft.run = '  pending command  '
    draft.startAfter = ['queue', 'state']
    draft.config = 'broken: [\n'
    draft.env[0].value = ''
    draft.env[1].replacing = true
    draft.env[2].removed = true
    draft.env.push(
      {
        key: 'LOG',
        value: 'duplicate',
        secret: false,
        replacing: false,
        removed: false,
        isNew: true,
      },
      {
        key: '',
        value: 'unfinished',
        secret: false,
        replacing: false,
        removed: false,
        isNew: true,
      },
      {
        key: 'NEW_TOKEN',
        value: 'typed-pending-token',
        secret: false,
        replacing: false,
        removed: false,
        isNew: true,
      },
    )
    const snapshot = captureDiscardedDraft(entry, draft)
    expect(snapshot.draft).toEqual(draft)
    const restored = restoreDiscardedDraft(snapshot, entry)
    expect(restored).toEqual(draft)
    expect(hasSettingsChanges(entry, restored)).toBe(true)
    expect(validateSettingsDraft(entry, restored).valid).toBe(false)
    expect(() => settingsPatch(entry, restored)).toThrow()
  })

  it('rebases only discarded local changes over newer fields and reordered environment rows', () => {
    const draft = draftFrom(entry)
    draft.run = 'pending command'
    draft.env[0].value = 'debug'
    draft.env[2].removed = true
    const snapshot = captureDiscardedDraft(entry, draft)
    const latest: ContainerEntry = {
      ...entry,
      run: 'externally changed command',
      start_after: ['queue'],
      config_override: 'port: 5000\n',
      environment: [
        entry.environment[1],
        { key: 'EXTERNAL', value: 'new', secret: false },
        { ...entry.environment[0], value: 'warn' },
        entry.environment[2],
      ],
    }
    const restored = restoreDiscardedDraft(snapshot, latest)
    expect(restored).toMatchObject({
      run: 'pending command',
      startAfter: ['queue'],
      config: 'port: 5000\n',
    })
    expect(restored.env.map((row) => row.key)).toEqual([
      'SAVED_TOKEN',
      'EXTERNAL',
      'LOG',
      'REMOVE',
    ])
    expect(settingsPatch(latest, restored).patch).toEqual({
      run: 'pending command',
      environment: { set: { LOG: 'debug' }, unset: ['REMOVE'] },
    })
  })

  it('masks stored credentials in snapshots and never fabricates unchanged credential replacements', () => {
    const legacy: ContainerEntry = {
      ...entry,
      environment: [
        { key: 'SAVED_TOKEN', value: MASK, secret: true },
        { key: 'LEGACY_SECRET', value: 'stored-plaintext', secret: false },
        { key: 'REFERENCED_TOKEN', value: tokenReference, secret: false },
      ],
    }
    const draft = draftFrom(legacy)
    draft.run = 'pending command'
    const snapshot = captureDiscardedDraft(legacy, draft)
    expect(snapshot.entry.environment).toEqual([
      { key: 'SAVED_TOKEN', value: null, secret: true },
      { key: 'LEGACY_SECRET', value: null, secret: true },
      { key: 'REFERENCED_TOKEN', value: tokenReference, secret: false },
    ])
    const restored = restoreDiscardedDraft(snapshot, legacy)
    expect(restored.env.slice(0, 2)).toEqual([
      {
        key: 'SAVED_TOKEN',
        value: '',
        secret: true,
        replacing: false,
        removed: false,
        isNew: false,
      },
      {
        key: 'LEGACY_SECRET',
        value: '',
        secret: true,
        replacing: false,
        removed: false,
        isNew: false,
      },
    ])
    expect(settingsPatch(legacy, restored).patch).toEqual({
      run: 'pending command',
    })
    expect(JSON.stringify(snapshot)).not.toContain('stored-plaintext')
    expect(JSON.stringify(snapshot)).not.toContain(MASK)
  })

  it('retains typed secret replacements and reconciles a new pending secret with a later masked row', () => {
    const draft = draftFrom(entry)
    draft.env[1].value = 'pending-replacement'
    draft.env[1].replacing = true
    draft.env.push({
      key: 'NEW_TOKEN',
      value: 'pending-new-token',
      secret: false,
      replacing: false,
      removed: false,
      isNew: true,
    })
    const snapshot = captureDiscardedDraft(entry, draft)
    expect(snapshot.draft.env[1]).toEqual(draft.env[1])
    expect(snapshot.draft.env.at(-1)).toEqual(draft.env.at(-1))
    const latest: ContainerEntry = {
      ...entry,
      environment: [
        { key: 'NEW_TOKEN', value: null, secret: true },
        entry.environment[2],
        entry.environment[0],
        entry.environment[1],
      ],
    }
    const restored = restoreDiscardedDraft(snapshot, latest)
    expect(restored.env[0]).toMatchObject({
      key: 'NEW_TOKEN',
      value: 'pending-new-token',
      secret: true,
      replacing: true,
      isNew: false,
    })
    expect(settingsPatch(latest, restored).patch.environment).toEqual({
      set: {
        NEW_TOKEN: 'pending-new-token',
        SAVED_TOKEN: 'pending-replacement',
      },
      unset: [],
    })
    expect(JSON.stringify(settingsPatch(latest, restored).patch)).not.toContain(
      MASK,
    )
  })

  it('does not recreate an unchanged write-only credential removed from the latest baseline', () => {
    const draft = draftFrom(entry)
    draft.config = 'port: 4000\n'
    const snapshot = captureDiscardedDraft(entry, draft)
    const latest = {
      ...entry,
      environment: [entry.environment[2], entry.environment[0]],
    }
    const restored = restoreDiscardedDraft(snapshot, latest)
    expect(restored.env.map((row) => row.key)).toEqual(['REMOVE', 'LOG'])
    expect(settingsPatch(latest, restored).patch).toEqual({
      config_override: 'port: 4000\n',
    })
  })

  it('returns an independent restored draft without changing either snapshot or latest entry', () => {
    const draft = draftFrom(entry)
    draft.startAfter = ['queue']
    draft.env[0].value = 'debug'
    const snapshot = captureDiscardedDraft(entry, draft)
    const latest = structuredClone(entry)
    const before = structuredClone({ snapshot, latest })
    const restored = restoreDiscardedDraft(snapshot, latest)
    restored.startAfter.push('http')
    restored.env[0].value = 'restore-only'
    expect({ snapshot, latest }).toEqual(before)
    expect(restoreDiscardedDraft(snapshot, latest)).toEqual(draft)
  })
})
