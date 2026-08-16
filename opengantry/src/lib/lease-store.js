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
    this.load();
  }

  load() {
    if (!fs.existsSync(this.storePath)) return;
    try {
      const raw = JSON.parse(fs.readFileSync(this.storePath, 'utf8'));
      for (const row of raw.leases ?? []) {
        this.leases.set(row.msn_id, row);
      }
    } catch {
      /* empty */
    }
  }

  persist() {
    const dir = path.dirname(this.storePath);
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(
      this.storePath,
      JSON.stringify({ leases: [...this.leases.values()] }, null, 2),
    );
  }

  get(msnId) {
    return this.leases.get(msnId);
  }

  upsert(lease) {
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
