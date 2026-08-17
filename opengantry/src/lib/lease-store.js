import fs from 'node:fs';
import path from 'node:path';

import { GantryDenied } from './denied.js';

const DANGEROUS_HOLDER_IDS = new Set(['__proto__', 'constructor', 'prototype']);

const LEASE_STATES = {
  active: 'active',
  tombstoned: 'tombstoned',
  promoting: 'promoting',
  dirty_rewritten: 'dirty_rewritten',
  reaped: 'reaped',
};

const KNOWN_STATES = new Set(Object.values(LEASE_STATES));

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

function sanitizeRow(row) {
  const { verdict_expected: _ve, ...rest } = row;
  return {
    ...rest,
    session_refs: normalizeSessionRefs(row.session_refs),
  };
}

function validateLeaseRow(row) {
  if (!row || typeof row !== 'object' || Array.isArray(row)) return false;
  if (typeof row.msn_id !== 'string' || !row.msn_id.trim()) return false;
  if (row.state != null && !KNOWN_STATES.has(row.state)) return false;
  return true;
}

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
        next.set(row.msn_id, sanitizeRow(row));
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

  persistMap(map) {
    const dir = path.dirname(this.storePath);
    fs.mkdirSync(dir, { recursive: true });
    const tmp = `${this.storePath}.${process.pid}.${Date.now()}.tmp`;
    const body = JSON.stringify({ leases: [...map.values()] }, null, 2);
    try {
      fs.writeFileSync(tmp, body);
      fs.renameSync(tmp, this.storePath);
    } catch (e) {
      try {
        fs.unlinkSync(tmp);
      } catch {
        /* ignore */
      }
      throw new GantryDenied('LEASE_PERSIST_FAILED', e.message);
    }
    return true;
  }

  get(msnId) {
    if (this.corrupted) return undefined;
    const row = this.leases.get(msnId);
    return row ? structuredClone(row) : undefined;
  }

  upsert(lease) {
    if (this.corrupted) return false;
    const row = sanitizeRow(lease);
    const next = new Map(this.leases);
    next.set(row.msn_id, row);
    this.persistMap(next);
    this.leases = next;
    return true;
  }

  bindMissionRel(msnId, missionRel) {
    if (this.corrupted) return false;
    const existing = this.leases.get(msnId);
    if (existing?.mission_rel) return true;
    const row = existing ?? {
      msn_id: msnId,
      branch: `gxt/${msnId.toLowerCase()}`,
      state: LEASE_STATES.active,
      session_refs: emptySessionRefs(),
    };
    const next = new Map(this.leases);
    next.set(msnId, sanitizeRow({ ...row, mission_rel: missionRel }));
    this.persistMap(next);
    this.leases = next;
    return true;
  }

  transition(msnId, from, to) {
    if (this.corrupted) return false;
    const row = this.leases.get(msnId);
    if (!row || row.state !== from) return false;
    const next = new Map(this.leases);
    next.set(msnId, sanitizeRow({ ...row, state: to }));
    this.persistMap(next);
    this.leases = next;
    return true;
  }

  acquireSession(msnId, holderId) {
    if (this.corrupted || DANGEROUS_HOLDER_IDS.has(holderId)) return null;
    const row = this.leases.get(msnId);
    if (!row) return null;
    const session_refs = normalizeSessionRefs(row.session_refs);
    const prev = Object.hasOwn(session_refs, holderId) ? session_refs[holderId] : 0;
    session_refs[holderId] = prev + 1;
    const next = new Map(this.leases);
    next.set(msnId, sanitizeRow({ ...row, session_refs }));
    this.persistMap(next);
    this.leases = next;
    return structuredClone(next.get(msnId));
  }

  releaseSession(msnId, holderId) {
    if (this.corrupted) return null;
    const row = this.leases.get(msnId);
    if (!row?.session_refs || !Object.hasOwn(row.session_refs, holderId)) {
      return row ? structuredClone(row) : null;
    }
    const session_refs = normalizeSessionRefs(row.session_refs);
    session_refs[holderId] -= 1;
    if (session_refs[holderId] <= 0) {
      delete session_refs[holderId];
    }
    let state = row.state;
    const activeSessions = Object.values(session_refs).reduce((a, b) => a + b, 0);
    if (activeSessions === 0 && state === LEASE_STATES.promoting) {
      state = LEASE_STATES.tombstoned;
    }
    const next = new Map(this.leases);
    next.set(msnId, sanitizeRow({ ...row, session_refs, state }));
    this.persistMap(next);
    this.leases = next;
    return structuredClone(next.get(msnId));
  }
}

export { LEASE_STATES };
