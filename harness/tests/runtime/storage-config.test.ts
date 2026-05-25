import { describe, expect, it } from 'vitest';
import {
  DEFAULT_DATABASE_NAME,
  loadStorageConfig,
  resolveDatabaseName,
} from '../../src/runtime/storage-config.js';

describe('loadStorageConfig', () => {
  it('defaults to harness when storage section is absent', () => {
    expect(loadStorageConfig({})).toEqual({ database_name: DEFAULT_DATABASE_NAME });
  });

  it('reads storage.database_name when set', () => {
    expect(loadStorageConfig({ storage: { database_name: 'shared-pool' } })).toEqual({
      database_name: 'shared-pool',
    });
  });

  it('falls back when storage.database_name is empty or non-string', () => {
    expect(loadStorageConfig({ storage: { database_name: '' } })).toEqual({
      database_name: DEFAULT_DATABASE_NAME,
    });
    expect(loadStorageConfig({ storage: { database_name: 42 } })).toEqual({
      database_name: DEFAULT_DATABASE_NAME,
    });
  });
});

describe('resolveDatabaseName', () => {
  const storage = { database_name: 'shared-pool' };

  it('prefers worker section override over shared storage', () => {
    expect(resolveDatabaseName({ database_name: 'worker-pool' }, storage)).toBe('worker-pool');
  });

  it('uses shared storage when worker section has no override', () => {
    expect(resolveDatabaseName({}, storage)).toBe('shared-pool');
  });

  it('defaults to harness when neither worker nor storage set a name', () => {
    expect(resolveDatabaseName({}, { database_name: DEFAULT_DATABASE_NAME })).toBe('harness');
  });

  it('treats empty worker override as absent and falls through to storage', () => {
    expect(resolveDatabaseName({ database_name: '' }, storage)).toBe('shared-pool');
    expect(resolveDatabaseName({ database_name: 0 }, storage)).toBe('shared-pool');
  });
});
