/** Composite key used by test mocks mirroring `${scope}/${key}` iii state storage. */
export function stateStoreKey(scope: string, key: string): string {
  return `${scope}/${key}`;
}

export function payloadStoreKey(payload: { scope?: string; key?: string }): string {
  return stateStoreKey(payload.scope ?? '', payload.key ?? '');
}
