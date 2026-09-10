export interface LocalTerminalLease {
  paneId: string
  sessionId: string
  reconnectToken: string
  lastSequence: number
}

export class TerminalLeaseStorageError extends Error {
  constructor(message: string, options?: { cause?: unknown }) {
    super(message, options)
    this.name = 'TerminalLeaseStorageError'
  }
}

const MAX_LEASE_PAYLOAD_BYTES = 64 * 1024
const textEncoder = new TextEncoder()
const inMemoryTerminalLeases = new Map<
  string,
  Map<string, LocalTerminalLease>
>()

type LeaseInput = LocalTerminalLease & { accessKey?: unknown }

function payloadByteLength(value: string): number {
  return textEncoder.encode(value).length
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0
}

function isValidLastSequence(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0
}

function parseLease(value: unknown): LocalTerminalLease | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const raw = value as Record<string, unknown>
  if ('accessKey' in raw) return null
  if (
    !isNonEmptyString(raw.paneId) ||
    !isNonEmptyString(raw.sessionId) ||
    !isNonEmptyString(raw.reconnectToken) ||
    !isValidLastSequence(raw.lastSequence)
  ) {
    return null
  }
  return {
    paneId: raw.paneId,
    sessionId: raw.sessionId,
    reconnectToken: raw.reconnectToken,
    lastSequence: raw.lastSequence,
  }
}

function readStoredLeases(raw: string | null): LocalTerminalLease[] {
  if (raw == null) return []
  if (payloadByteLength(raw) > MAX_LEASE_PAYLOAD_BYTES) return []
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return []
  }
  if (!Array.isArray(parsed)) return []

  const leases: LocalTerminalLease[] = []
  const seenPaneIds = new Set<string>()
  for (const entry of parsed) {
    const lease = parseLease(entry)
    if (!lease || seenPaneIds.has(lease.paneId)) return []
    seenPaneIds.add(lease.paneId)
    leases.push(lease)
  }
  return leases
}

function serializeLeases(leases: LocalTerminalLease[]): string {
  return JSON.stringify(leases)
}

function assertSavableLease(lease: LeaseInput): LocalTerminalLease {
  if ('accessKey' in lease && lease.accessKey !== undefined) {
    throw new Error('access keys must not be persisted in terminal leases')
  }
  const parsed = parseLease(lease)
  if (!parsed) {
    throw new Error('invalid terminal lease')
  }
  return parsed
}

function readStorageItem(storage: Storage, key: string): string | null {
  try {
    return storage.getItem(key)
  } catch (err) {
    throw new TerminalLeaseStorageError(
      'failed to read terminal leases from storage',
      { cause: err },
    )
  }
}

function writeStorageItem(storage: Storage, key: string, value: string): void {
  try {
    storage.setItem(key, value)
  } catch (err) {
    throw new TerminalLeaseStorageError(
      'failed to write terminal leases to storage',
      { cause: err },
    )
  }
}

function deleteStorageItem(storage: Storage, key: string): void {
  try {
    storage.removeItem(key)
  } catch (err) {
    throw new TerminalLeaseStorageError(
      'failed to remove terminal leases from storage',
      { cause: err },
    )
  }
}

export function loadTerminalLeases(
  storage: Storage,
  key: string,
): LocalTerminalLease[] {
  try {
    return readStoredLeases(readStorageItem(storage, key))
  } catch (err) {
    if (err instanceof TerminalLeaseStorageError) return []
    throw err
  }
}

export function saveTerminalLease(
  storage: Storage,
  key: string,
  lease: LeaseInput,
): void {
  const nextLease = assertSavableLease(lease)
  const leases = readStoredLeases(readStorageItem(storage, key))
  const index = leases.findIndex((entry) => entry.paneId === nextLease.paneId)
  if (index >= 0) {
    leases[index] = nextLease
  } else {
    leases.push(nextLease)
  }
  const serialized = serializeLeases(leases)
  if (payloadByteLength(serialized) > MAX_LEASE_PAYLOAD_BYTES) {
    throw new Error('terminal lease payload exceeds 64 KiB')
  }
  writeStorageItem(storage, key, serialized)
}

export function removeTerminalLease(
  storage: Storage,
  key: string,
  paneId: string,
): void {
  if (!isNonEmptyString(paneId)) return
  const leases = readStoredLeases(readStorageItem(storage, key)).filter(
    (entry) => entry.paneId !== paneId,
  )
  if (leases.length === 0) {
    deleteStorageItem(storage, key)
    return
  }
  writeStorageItem(storage, key, serializeLeases(leases))
}

export function loadRecoverableTerminalLeases(
  storage: Storage | null,
  key: string,
): LocalTerminalLease[] {
  const leases = new Map<string, LocalTerminalLease>()
  if (storage) {
    for (const lease of loadTerminalLeases(storage, key)) {
      leases.set(lease.paneId, lease)
    }
  }
  for (const lease of inMemoryTerminalLeases.get(key)?.values() ?? []) {
    leases.set(lease.paneId, lease)
  }
  return [...leases.values()]
}

export function saveRecoverableTerminalLease(
  storage: Storage | null,
  key: string,
  lease: LeaseInput,
): void {
  const nextLease = assertSavableLease(lease)
  const leases =
    inMemoryTerminalLeases.get(key) ?? new Map<string, LocalTerminalLease>()
  leases.set(nextLease.paneId, nextLease)
  inMemoryTerminalLeases.set(key, leases)
  if (storage) saveTerminalLease(storage, key, nextLease)
}

export function removeRecoverableTerminalLease(
  storage: Storage | null,
  key: string,
  paneId: string,
): void {
  const leases = inMemoryTerminalLeases.get(key)
  leases?.delete(paneId)
  if (leases?.size === 0) inMemoryTerminalLeases.delete(key)
  if (storage) removeTerminalLease(storage, key, paneId)
}
