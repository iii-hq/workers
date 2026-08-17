import fs from 'node:fs';
import path from 'node:path';

const DANGEROUS_HOLDER_IDS = new Set(['__proto__', 'constructor', 'prototype']);

function emptySessionRefs() {
  return Object.create(null);
}

function normalizeSessionRefs(raw) {
  const out = emptySessionRefs();
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return out;
  for (const [holderId, count] of Object.entries(raw)) {
    if (DANGEROUS_HOLDER_IDS.has(holderId)) continue;
    if (typeof count !== 'number' || !Number.isFinite(count) || count < 0) continue;
    out[holderId] = Math.floor(count);
  }
  return out;
}

function validateLeaseRow(row) {
  if (!row || typeof row !== 'object' || Array.isArray(row)) return false;
  if (typeof row.msn_id !== 'string' || !row.msn_id.trim()) return false;
  if (row.state != null && typeof row.state !== 'string') return false;
  return true;
}

const LEASE_STATES = {
  active: 'active',
  tombstoned: 'tombstoned',
  promoting: 'promoting',
  dirty_rewritten: 'dirty_rewritten',
  reaped: 'reaped',
};

export class LeaseStore {
  constructor(storePath) {
    this.storePath = storePath;
    this.leases = new Map();
    this.corrupted = false;
    this.load();
  }

  load() {
    if (!fs.existsSync(this.storePath)) return;
    try {
      const raw = JSON.parse(fs.readFileSync(this.storePath, 'utf8'));
      if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
        this.markCorrupted();
        return;
      }
      if (!Array.isArray(raw.leases)) {
        this.markCorrupted();
        return;
      }
      const next = new Map();
      for (const row of raw.leases) {
        if (!validateLeaseRow(row)) {
          this.markCorrupted();
          return;
        }
        if (next.has(row.msn_id)) {
          this.markCorrupted();
          return;
        }
        next.set(row.msn_id, {
          ...row,
          session_refs: normalizeSessionRefs(row.session_refs),
        });
      }
      this.leases = next;
    } catch {
      this.markCorrupted();
    }
  }

  markCorrupted() {
    this.corrupted = true;
    this.leases.clear();
  }

  persist() {
    if (this.corrupted) return false;
    const dir = path.dirname(this.storePath);
    fs.mkdirSync(dir, { recursive: true });
    const tmp = `${this.storePath}.${process.pid}.${Date.now()}.tmp`;
    const body = JSON.stringify({ leases: [...this.leases.values()] }, null, 2);
    fs.writeFileSync(tmp, body);
    fs.renameSync(tmp, this.storePath);
    return true;
  }

  get(msnId) {
    if (this.corrupted) return undefined;
    return this.leases.get(msnId);
  }

  upsert(lease) {
    if (this.corrupted) return false;
    const row = {
      ...lease,
      session_refs: normalizeSessionRefs(lease.session_refs),
    };
    this.leases.set(row.msn_id, row);
    return this.persist();
  }

  acquireSession(msnId, holderId) {
    if (this.corrupted || DANGEROUS_HOLDER_IDS.has(holderId)) return null;
    const lease = this.leases.get(msnId);
    if (!lease) return null;
    lease.session_refs = normalizeSessionRefs(lease.session_refs);
    const prev = Object.hasOwn(lease.session_refs, holderId) ? lease.session_refs[holderId] : 0;
    lease.session_refs[holderId] = prev + 1;
    this.persist();
    return lease;
  }

  releaseSession(msnId, holderId) {
    if (this.corrupted) return null;
    const lease = this.leases.get(msnId);
    if (!lease?.session_refs || !Object.hasOwn(lease.session_refs, holderId)) return lease ?? null;
    lease.session_refs[holderId] -= 1;
    if (lease.session_refs[holderId] <= 0) {
      delete lease.session_refs[holderId];
    }
    const activeSessions = Object.values(lease.session_refs).reduce((a, b) => a + b, 0);
    if (activeSessions === 0 && lease.state === LEASE_STATES.promoting) {
      lease.state = LEASE_STATES.tombstoned;
    }
    this.persist();
    return lease;
  }
}

export { LEASE_STATES };
