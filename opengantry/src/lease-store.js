import fs from 'node:fs';
import path from 'node:path';

import { GantryDenied } from './denied.js';

const DANGEROUS_HOLDER_IDS = new Set(['__proto__', 'constructor', 'prototype']);
const LOCK_WAIT_MS = 5000;
const LOCK_POLL_MS = 50;
const LOCK_STALE_MS = 30_000;
const LEASE_FILE_MODE = 0o600;

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
  if (!row.state || !KNOWN_STATES.has(row.state)) return false;
  return true;
}

function sleepMs(ms) {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

function lockHolderPid(lockPath) {
  try {
    const raw = fs.readFileSync(lockPath, 'utf8').trim();
    const pid = Number(raw);
    return Number.isFinite(pid) ? pid : null;
  } catch {
    return null;
  }
}

function isPidAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function clearStaleLock(lockPath) {
  try {
    const stat = fs.statSync(lockPath);
    const pid = lockHolderPid(lockPath);
    const staleByTime = Date.now() - stat.mtimeMs > LOCK_STALE_MS;
    const staleByPid = pid != null && !isPidAlive(pid);
    if (staleByTime || staleByPid) {
      fs.unlinkSync(lockPath);
    }
  } catch {
    /* ignore */
  }
}

async function withStoreLock(storePath, fn) {
  const lockPath = `${storePath}.lock`;
  fs.mkdirSync(path.dirname(lockPath), { recursive: true });
  const deadline = Date.now() + LOCK_WAIT_MS;
  let fd;
  while (true) {
    try {
      fd = fs.openSync(
        lockPath,
        fs.constants.O_CREAT | fs.constants.O_EXCL | fs.constants.O_WRONLY,
      );
      fs.writeFileSync(fd, String(process.pid));
      break;
    } catch (e) {
      if (e.code !== 'EEXIST') throw e;
      clearStaleLock(lockPath);
      if (Date.now() >= deadline) {
        throw new GantryDenied('LEASE_LOCK_TIMEOUT', 'lease store lock timeout');
      }
      await sleepMs(LOCK_POLL_MS);
    }
  }
  try {
    return fn();
  } finally {
    try {
      fs.closeSync(fd);
    } catch {
      /* ignore */
    }
    try {
      fs.unlinkSync(lockPath);
    } catch {
      /* ignore */
    }
  }
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

  persistMapUnlocked(map) {
    const dir = path.dirname(this.storePath);
    fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
    const tmp = `${this.storePath}.${process.pid}.${Date.now()}.tmp`;
    const body = JSON.stringify({ leases: [...map.values()] }, null, 2);
    try {
      fs.writeFileSync(tmp, body, { mode: LEASE_FILE_MODE });
      fs.renameSync(tmp, this.storePath);
      fs.chmodSync(this.storePath, LEASE_FILE_MODE);
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

  async mutate(apply) {
    if (this.corrupted) return false;
    return withStoreLock(this.storePath, () => {
      this.load();
      if (this.corrupted) return false;
      const map = new Map(this.leases);
      const ok = apply(map);
      if (ok === false) return false;
      this.persistMapUnlocked(map);
      this.leases = map;
      return true;
    });
  }

  get(msnId) {
    if (this.corrupted) return undefined;
    const row = this.leases.get(msnId);
    return row ? structuredClone(row) : undefined;
  }

  async upsert(lease) {
    if (this.corrupted) return false;
    const row = sanitizeRow(lease);
    return this.mutate((map) => {
      map.set(row.msn_id, row);
      return true;
    });
  }

  async bindMissionRel(msnId, missionRel) {
    if (this.corrupted) return false;
    return this.mutate((map) => {
      const existing = map.get(msnId);
      if (existing?.mission_rel) return true;
      const row = existing ?? {
        msn_id: msnId,
        branch: `gxt/${msnId.toLowerCase()}`,
        state: LEASE_STATES.active,
        session_refs: emptySessionRefs(),
      };
      map.set(msnId, sanitizeRow({ ...row, mission_rel: missionRel }));
      return true;
    });
  }

  async transition(msnId, from, to) {
    if (this.corrupted) return false;
    return this.mutate((map) => {
      const row = map.get(msnId);
      if (!row || row.state !== from) return false;
      map.set(msnId, sanitizeRow({ ...row, state: to }));
      return true;
    });
  }

  async acquireSession(msnId, holderId) {
    if (this.corrupted || DANGEROUS_HOLDER_IDS.has(holderId)) return null;
    let snapshot;
    const ok = await this.mutate((map) => {
      const row = map.get(msnId);
      if (!row) return false;
      const session_refs = normalizeSessionRefs(row.session_refs);
      const prev = Object.hasOwn(session_refs, holderId) ? session_refs[holderId] : 0;
      session_refs[holderId] = prev + 1;
      const nextRow = sanitizeRow({ ...row, session_refs });
      map.set(msnId, nextRow);
      snapshot = structuredClone(nextRow);
      return true;
    });
    return ok ? snapshot : null;
  }

  async releaseSession(msnId, holderId) {
    if (this.corrupted) return null;
    let snapshot;
    const ok = await this.mutate((map) => {
      const row = map.get(msnId);
      if (!row?.session_refs || !Object.hasOwn(row.session_refs, holderId)) {
        snapshot = row ? structuredClone(row) : null;
        return true;
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
      const nextRow = sanitizeRow({ ...row, session_refs, state });
      map.set(msnId, nextRow);
      snapshot = structuredClone(nextRow);
      return true;
    });
    return ok ? snapshot : null;
  }
}

export { LEASE_STATES };
