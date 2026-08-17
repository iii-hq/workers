/** Mirrors production verify-pass lease binding for tests and demos. */
export function bindVerdictLease(leases, msnId, expected, { missionRel, ...overrides } = {}) {
  const lease = leases.get(msnId) ?? {
    msn_id: msnId,
    state: 'active',
    session_refs: Object.create(null),
  };
  if (missionRel) lease.mission_rel = missionRel;
  lease.verdict_expected = expected;
  leases.upsert({ ...lease, ...overrides });
  return lease;
}
