import { vi, type Mock } from 'vitest';
import type { RunRequest } from '../../../src/turn-orchestrator/run-request.js';
import * as storeModule from '../../../src/turn-orchestrator/state-runtime/store.js';
import type { TurnStore } from '../../../src/turn-orchestrator/state-runtime/store.js';

export const defaultRunRequest: RunRequest = {
  provider: 'openai',
  model: 'gpt-4',
  mode: 'agent',
  system_prompt: '',
  function_schemas: [],
};

export type MockTurnStore = {
  [K in keyof TurnStore]: TurnStore[K] extends (...args: infer A) => infer R
    ? Mock<(...args: A) => R>
    : TurnStore[K];
};

export function mockTurnStore(overrides: Partial<TurnStore> = {}): MockTurnStore {
  return {
    loadRecord: vi.fn(async () => null),
    loadRecordStrict: vi.fn(async () => null),
    saveRecord: vi.fn(async () => {}),
    writeRecord: vi.fn(async () => {}),
    ensureSession: vi.fn(async () => {}),
    loadMessages: vi.fn(async () => []),
    loadTrailingResultIds: vi.fn(async () => new Set<string>()),
    appendMessages: vi.fn(async () => {}),
    loadRunRequest: vi.fn(async () => defaultRunRequest),
    saveRunRequest: vi.fn(async () => {}),
    ...overrides,
  } as MockTurnStore;
}

/** Mock `createTurnStore` and return the store instance for assertions. */
export function installMockTurnStore(overrides: Partial<TurnStore> = {}): MockTurnStore {
  const store = mockTurnStore(overrides);
  vi.spyOn(storeModule, 'createTurnStore').mockReturnValue(store);
  return store;
}
