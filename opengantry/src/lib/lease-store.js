import fs from 'node:fs';
import path from 'node:path';

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
        this.corrupted = true;
        return;
      }
      for (const row of raw.leases ?? []) {
        if (!row?.msn_id) {
          this.corrupted = true;
          return;
        }
        this.leases.set(row.msn_id, row);
      }
    } catch {
      this.corrupted = true;
    }
  }

  persist() {
    if (this.corrupted) return;
    const dir = path.dirname(this.storePath);
    fs.mkdirSync(dir, { recursive: true });
    const tmp = `${this.storePath}.${process.pid}.${Date.now()}.tmp`;
    const body = JSON.stringify({ leases: [...this.leases.values()] }, null, 2);
    fs.writeFileSync(tmp, body);
    fs.renameSync(tmp, this.storePath);
  }

  get(msnId) {
    return this.leases.get(msnId);
  }

  upsert(lease) {
    if (this.corrupted) return;
    this.leases.set(lease.msn_id, lease);
    this.persist();
  }

  acquireSession(msnId, holderId) {
    const lease = this.leases.get(msnId);
    if (!lease) return null;
    lease.session_refs = lease.session_refs ?? {};
    lease.session_refs[holderId] = (lease.session_refs[holderId] ?? 0) + 1;
    this.persist();
    return lease;
  }

  releaseSession(msnId, holderId) {
    const lease = this.leases.get(msnId);
    if (!lease?.session_refs?.[holderId]) return lease;
    lease.session_refs[holderId] -= 1;
    if (lease.session_refs[holderId] <= 0) delete lease.session_refs[holderId];
    const activeSessions = Object.values(lease.session_refs).reduce((a, b) => a + b, 0);
    if (activeSessions === 0 && lease.state === LEASE_STATES.promoting) {
      lease.state = LEASE_STATES.tombstoned;
    }
    this.persist();
    return lease;
  }
}

export { LEASE_STATES };
