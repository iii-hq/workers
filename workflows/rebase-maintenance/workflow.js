globalThis.rebaseMaintenanceRevision = '1';
globalThis.rebaseMaintenanceStep = function (input) {
  const s = JSON.parse(JSON.stringify(input));
  s.version = (s.version || 0) + 1;
  const emit = (type, data = {}) => ({ state: s, actions: [{ id: [s.run_id, s.version, type].join(':'), type, ...data }] });
  const groups = s.groups || [];
  for (const r of s.results || []) {
    const g = groups.find(x => x.id === r.group);
    if (!g || r.run_id !== s.run_id || r.attempt !== g.attempt || r.revision !== s.input_revision || !r.producer_stopped || g.processed?.includes(r.event_id)) continue;
    g.processed = [...(g.processed || []), r.event_id];
    g.status = r.outcome === 'done' && Array.isArray(r.evidence) && r.evidence.length ? 'delivered' : 'blocked';
  }
  s.results = [];
  for (const g of groups) if (g.status === 'running' && s.now >= g.expires_at) { g.status = 'expired'; g.reconcile = true; }
  const active = groups.filter(g => g.status === 'running' || g.reconcile);
  if (s.deadline && s.now >= s.deadline) return emit('finish', { complete: false, reason: 'deadline', active: active.map(g => g.id) });
  if (s.no_progress >= 2) return emit('finish', { complete: false, reason: 'no-progress' });
  if (s.approved !== true) return emit('request approval');
  if (s.route_verified !== true || s.snapshot_ready !== true) return emit('finish', { complete: false, reason: 'prerequisite' });
  if (groups.some(g => g.status === 'blocked' || g.status === 'expired')) return emit(active.length ? 'wait' : 'finish', { complete: false, reason: 'incomplete' });
  const available = Math.max(0, Math.min(s.concurrency, s.limit - s.calls) - active.length);
  const occupied = new Set(active.flatMap(g => g.paths));
  const ready = [];
  for (const g of groups) {
    if (ready.length >= available) break;
    if (g.status !== 'pending' || g.attempt >= 2 || g.paths.some(p => occupied.has(p))) continue;
    ready.push(g); g.paths.forEach(p => occupied.add(p));
  }
  if (ready.length) {
    const actions = ready.map(g => { g.attempt++; g.status = 'running'; g.expires_at = s.now + 1200000; s.calls++; return { id: [s.run_id, s.version, 'dispatch', g.id, g.attempt].join(':'), type: 'dispatch', group: g.id, attempt: g.attempt, session: g.session, paths: g.paths }; });
    return { state: s, actions };
  }
  if (active.length) return emit('wait');
  if (!groups.every(g => g.status === 'delivered')) return emit('finish', { complete: false, reason: 'missing-result' });
  if (!s.checks) return emit('verify', { target: 'coordinator-checks' });
  if (s.checks.acceptable !== true) return emit('finish', { complete: false, reason: 'checks-blocked' });
  if (!s.review) {
    if (s.calls >= s.limit) return emit('finish', { complete: false, reason: 'budget' });
    s.calls++; s.review = { status: 'running' };
    return emit('verify', { target: 'independent-review' });
  }
  if (s.review.status === 'running') return emit('wait');
  if (s.review.accepted !== true || s.review.stopped !== true) return emit('finish', { complete: false, reason: 'review' });
  return emit('finish', { complete: true });
};
globalThis.validateRebaseMaintenance = function (out, root) {
  if (!out?.state || !Array.isArray(out.actions) || !out.actions.length || out.actions.length > 3 || out.state.calls > out.state.limit) throw Error('invalid bounded output');
  const ids = new Set(); const paths = new Set();
  for (const a of out.actions) {
    if (!a.id || ids.has(a.id) || !['dispatch','wait','request approval','verify','finish'].includes(a.type)) throw Error('invalid action');
    ids.add(a.id);
    if (a.type === 'dispatch') {
      const g = out.state.groups.find(g => g.id === a.group);
      if (!g || g.status !== 'running' || g.session !== a.session || JSON.stringify(a.paths) !== JSON.stringify(g.paths)) throw Error('group contract');
      for (const p of a.paths) { if (!p.startsWith(root + '/') || p.split('/').includes('..') || paths.has(p)) throw Error('path/lock'); paths.add(p); }
    }
  }
  return out;
};
