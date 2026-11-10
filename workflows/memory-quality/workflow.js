globalThis.memoryQualityRevision = '12';
// Pure POSIX lexical paths only: this does not resolve filesystem symlinks.
const memoryQualityPath = path => {
  if (typeof path !== 'string' || !path.startsWith('/') || path.includes('\0')) return null;
  const parts = [];
  for (const part of path.split('/')) {
    if (!part || part === '.') continue;
    if (part === '..') { if (!parts.length) return null; parts.pop(); }
    else parts.push(part);
  }
  return '/' + parts.join('/');
};
const memoryQualityContains = (root, path) => root !== null && path !== null && (root === '/' || root === path || path.startsWith(root + '/'));
const memoryQualityOverlap = (a, b) => {
  const left = memoryQualityPath(a), right = memoryQualityPath(b);
  return left === null || right === null || memoryQualityContains(left, right) || memoryQualityContains(right, left);
};
globalThis.memoryQualityStep = function(snapshot) {
  const s = JSON.parse(JSON.stringify(snapshot));
  const emit = (type, detail = {}) => ({ state: s, actions: [{ id: `${s.run_id}:${s.state_version}:${type}:${detail.unit || 'run'}`, type, ...detail }] });
  s.state_version++;
  if (!Array.isArray(s.units) || !Number.isFinite(s.now_ms) || !(Number.isFinite(s.deadline_ms) || (s.deadline_ms === null && s.deadline_mode === 'user-open-ended'))) throw Error('invalid snapshot');
  const phaseMs = Number.isFinite(s.limits.phase_ms) && s.limits.phase_ms > 0 ? s.limits.phase_ms : 1800000;
  for (const r of s.results || []) {
    const u = s.units.find(x => x.id === r.unit);
    if (!u || !['running','verifying','expired'].includes(u.status) || r.run_id !== s.run_id || r.attempt !== u.attempt || r.revision !== s.code_revision || u.processed?.includes(r.event_id) || !r.producer_stopped) continue;
    const phase = u.status === 'expired' ? u.expired_phase : u.status;
    if (phase !== 'running' && !(phase === 'verifying' && r.independent === true)) continue;
    u.processed = [...(u.processed || []), r.event_id];
    u.needs_reconciliation = false;
    u.status = phase;
    delete u.expired_phase;
    if (!['done','blocked','failed'].includes(r.outcome) || !Array.isArray(r.evidence) || !r.evidence.length) { u.status = 'failed'; u.reason = 'invalid-result'; continue; }
    if (r.outcome !== 'done') { u.status = r.outcome; u.reason = r.reason || r.outcome; continue; }
    if (phase === 'verifying' && r.accepted === true) u.status = 'accepted';
    else if (phase === 'running') u.status = 'delivered';
  }
  s.results = [];
  // Expired is a quarantined phase: wait requests producer reconciliation using
  // the existing action vocabulary; only a matching stopped receipt releases it.
  // Verification receipts must be independent review receipts, not stale writers.
  for (const u of s.units) {
    if (['running','verifying'].includes(u.status)) {
      if (!Number.isFinite(u.phase_deadline_ms)) u.phase_deadline_ms = s.now_ms + phaseMs;
      if (s.now_ms >= u.phase_deadline_ms) { u.expired_phase = u.status; u.status = 'expired'; u.needs_reconciliation = true; }
    }
    if (u.status === 'expired' && u.needs_reconciliation) {
      const receipt = (s.continuations || []).find(c => c.unit === u.id && c.attempt === u.attempt && c.revision === s.code_revision && c.producer_stopped === true && (u.expired_phase !== 'verifying' || c.kind === 'review-correction'));
      if (receipt) {
        u.status = 'failed'; u.reason = 'phase-expired'; u.needs_reconciliation = false;
        delete u.expired_phase;
        s.continuations = s.continuations.filter(c => c !== receipt);
      }
    }
  }
  const unreconciled = s.units.filter(u => u.needs_reconciliation);
  const preps = s.preparations || [];
  for (const r of s.preparation_results || []) {
    const p=preps.find(p=>p.id===r.id);
    if(!p||!['running','expired','delivered'].includes(p.status)||r.run_id!==s.run_id||r.attempt!==p.attempt||r.revision!==s.code_revision||typeof r.event_id!=='string'||!r.event_id||p.processed?.includes(r.event_id)||r.producer_stopped!==true) continue;
    if(p.status==='delivered'&&r.independent!==true) continue;
    p.processed=[...(p.processed||[]),r.event_id];
    p.needs_reconciliation=false;
    delete p.expired_phase;
    p.status=r.outcome==='done'&&Array.isArray(r.evidence)&&r.evidence.length?(r.independent===true&&r.accepted===true?'accepted':'delivered'):(r.outcome==='blocked'?'blocked':'failed');
  }
  s.preparation_results=[];
  for(const p of preps) if(p.status==='running'&&s.now_ms>=p.deadline_ms){p.status='expired';p.needs_reconciliation=true;}
  const stages=s.parallel_stages||[];
  for(const r of s.stage_results||[]){
    const p=stages.find(p=>p.id===r.id);
    if(!p||!['running','verifying','expired'].includes(p.status)||r.run_id!==s.run_id||r.attempt!==p.attempt||r.revision!==s.code_revision||typeof r.event_id!=='string'||!r.event_id||p.processed?.includes(r.event_id)||r.producer_stopped!==true)continue;
    const phase=p.status==='expired'?p.expired_phase:p.status;
    if(phase!=='running'&&!(phase==='verifying'&&r.independent===true))continue;
    p.processed=[...(p.processed||[]),r.event_id];
    p.needs_reconciliation=false;
    delete p.expired_phase;
    if(r.outcome!=='done'||!Array.isArray(r.evidence)||!r.evidence.length){p.status=r.outcome==='blocked'?'blocked':'failed';continue;}
    p.status=phase==='running'?'delivered':r.accepted===true?'accepted':'failed';
  }
  s.stage_results=[];
  for(const p of stages)if(['running','verifying'].includes(p.status)&&s.now_ms>=p.deadline_ms){p.expired_phase=p.status;p.status='expired';p.needs_reconciliation=true;}
  const prepPending=preps.some(p=>p.required&&p.status!=='accepted')||stages.some(p=>p.required&&p.status!=='accepted');
  const correctionOpen = s.limits.correction_mode === 'user-unbounded' && s.authorization?.correction_rounds_unbounded === true;
  const correctionAvailable = u => correctionOpen || (u.correction_rounds || 0) < (u.correction_limit ?? s.limits.correction_rounds);
  // A stopped continuation may release quarantine before global limits are
  // evaluated, but must pass the same authorization, locks and correction cap.
  const stageContinuationEligible = (c,p) => {
    if(!p||s.authorization?.parallel_implementation!==true||p.authorized!==true||c.producer_stopped!==true||c.revision!==s.code_revision||c.attempt!==p.attempt||!['running','verifying','delivered','blocked','failed','expired'].includes(p.status))return false;
    if(c.run_id!==undefined&&c.run_id!==s.run_id||c.event_id!==undefined&&(typeof c.event_id!=='string'||!c.event_id||p.processed?.includes(c.event_id)))return false;
    if(p.status==='expired') {
      if(!p.needs_reconciliation||c.run_id!==s.run_id||typeof c.event_id!=='string'||!c.event_id||!['running','verifying'].includes(p.expired_phase))return false;
      if(p.expired_phase==='verifying'&&(c.kind!=='review-correction'||c.independent!==true))return false;
    } else if(p.needs_reconciliation)return false;
    return typeof c.session_id==='string'&&c.session_id.trim().length>0&&(c.kind!=='review-correction'||correctionAvailable(p))&&!(p.paths||[]).some(path=>[...(s.write_locks||[]).flatMap(l=>l.paths||[]),...stages.filter(x=>x!==p&&(['running','verifying'].includes(x.status)||x.needs_reconciliation)).flatMap(x=>x.paths||[])].some(held=>memoryQualityOverlap(path,held)))&&[...s.units.filter(u=>['running','verifying'].includes(u.status)),...preps.filter(p=>p.status==='running'||p.needs_reconciliation),...stages.filter(p=>['running','verifying'].includes(p.status)||p.needs_reconciliation)].filter(x=>x.id!==p.id).length<Math.min(8,s.limits.concurrency||4);
  };
  const stoppedStageContinuation=(s.stage_continuations||[]).find(c=>{
    const p=stages.find(p=>p.id===c.id);
    return p?.status==='expired'&&stageContinuationEligible(c,p);
  });
  if(stoppedStageContinuation){
    const p=stages.find(p=>p.id===stoppedStageContinuation.id);
    p.status='failed';p.reason='phase-expired';p.needs_reconciliation=false;
    delete p.expired_phase;
    // Keep the receipt queued until dispatch: another guard or preparation may
    // use this step's action without losing the authorized continuation.
  }
  const busy = [...s.units.filter(u => ['running','verifying'].includes(u.status)),...preps.filter(p=>p.status==='running'||p.needs_reconciliation),...stages.filter(p=>['running','verifying'].includes(p.status)||p.needs_reconciliation)];
  if (unreconciled.length) return emit('wait', { reason: 'phase-expired', active: unreconciled.map(u => u.id) });
  if (!prepPending && s.scope_units?.length && s.scope_units.every(id => s.units.find(u => u.id === id)?.status === 'accepted')) return emit('finish', { complete: s.units.every(u => u.status === 'accepted'), scope_complete: true, reason: 'authorized-scope-complete' });
  if (!prepPending && s.units.every(u => u.status === 'accepted')) return emit('finish', { complete: true });
  const halted = (Number.isFinite(s.deadline_ms) && s.now_ms >= s.deadline_ms) || s.consumed.agent_calls >= s.limits.agent_calls || s.consumed.no_progress_rounds >= s.limits.no_progress_rounds || (!correctionOpen && s.units.some(u => (u.correction_rounds || 0) > (u.correction_limit ?? s.limits.correction_rounds)));
  if (halted) return emit(busy.length ? 'wait' : 'finish', { complete: false, reason: 'limit', active: busy.map(u=>u.id) });
  if (!s.approved) return emit('request approval', { reason: 'execution-authorization' });
  if (!s.preflight_ok || !s.route_verified) return emit('finish', { complete: false, reason: 'prerequisite' });
  if(s.authorization?.parallel_preparation===true){
    const slots=Math.max(0,Math.min(8,s.limits.concurrency||4)-busy.length);
    const budget=Math.max(0,s.limits.agent_calls-s.consumed.agent_calls);
    const ready=preps.filter(p=>p.status==='pending'&&p.read_only===true&&!p.needs_reconciliation&&p.attempt<(p.attempt_limit??s.limits.attempts_per_unit)&&p.depends_on.every(id=>s.units.find(u=>u.id===id)?.status==='accepted')).slice(0,Math.min(slots,budget));
    if(ready.length){const actions=ready.map(p=>{p.attempt++;p.status='running';p.deadline_ms=s.now_ms+(p.watchdog_ms||1200000);s.consumed.agent_calls++;return {id:[s.run_id,s.state_version,'dispatch','preparation',p.id,p.attempt].join(':'),type:'dispatch',unit:p.unit,preparation:true,preparation_id:p.id,attempt:p.attempt,read_only:true,session_id:p.session_id};});return {state:s,actions};}
  }
  if(s.authorization?.parallel_implementation===true){
    const continuation=stoppedStageContinuation||(s.stage_continuations||[]).find(c=>stageContinuationEligible(c,stages.find(p=>p.id===c.id)));
    if(continuation){
      const p=stages.find(p=>p.id===continuation.id);p.status='running';p.session_id=continuation.session_id;p.deadline_ms=s.now_ms+(p.watchdog_ms||1200000);
      if(continuation.event_id&&!p.processed?.includes(continuation.event_id))p.processed=[...(p.processed||[]),continuation.event_id];
      if(continuation.kind==='review-correction'){p.correction_rounds=(p.correction_rounds||0)+1;s.consumed.correction_rounds++;}
      s.stage_continuations=s.stage_continuations.filter(c=>c!==continuation);s.consumed.agent_calls++;
      return {state:s,actions:[{id:[s.run_id,s.state_version,'dispatch','stage',p.id,p.attempt,'correction',p.correction_rounds||0].join(':'),type:'dispatch',unit:p.unit,stage_id:p.id,attempt:p.attempt,correction_round:p.correction_rounds||0,session_id:p.session_id,paths:p.paths,continuation:true,receipt:continuation.receipt}]};
    }
    const slots=Math.max(0,Math.min(8,s.limits.concurrency||4)-busy.length);
    const budget=Math.max(0,s.limits.agent_calls-s.consumed.agent_calls);
    const held=[...(s.write_locks||[]).flatMap(l=>l.paths||[]),...stages.filter(p=>['running','verifying'].includes(p.status)||p.needs_reconciliation).flatMap(p=>p.paths||[])];
    const ready=[];
    for(const p of stages){
      if(ready.length>=Math.min(slots,budget))break;
      if(p.status!=='pending'||p.authorized!==true||p.needs_reconciliation||p.attempt>=(p.attempt_limit??s.limits.attempts_per_unit)||!Array.isArray(p.paths)||!p.paths.length)continue;
      if(!p.depends_on.every(id=>preps.some(x=>x.id===id&&x.status==='accepted')||s.units.some(x=>x.id===id&&x.status==='accepted')||stages.some(x=>x.id===id&&x.status==='accepted')))continue;
      if(p.paths.some(path=>held.some(other=>memoryQualityOverlap(path,other))))continue;
      ready.push(p);held.push(...p.paths);
    }
    if(ready.length){const actions=ready.map(p=>{p.attempt++;p.status='running';p.deadline_ms=s.now_ms+(p.watchdog_ms||1200000);s.consumed.agent_calls++;return {id:[s.run_id,s.state_version,'dispatch','stage',p.id,p.attempt].join(':'),type:'dispatch',unit:p.unit,stage_id:p.id,attempt:p.attempt,session_id:p.session_id,paths:p.paths};});return {state:s,actions};}
    if(slots>0&&budget>0){const p=stages.find(p=>p.status==='delivered');if(p){p.status='verifying';p.deadline_ms=s.now_ms+(p.watchdog_ms||1200000);s.consumed.agent_calls++;return {state:s,actions:[{id:[s.run_id,s.state_version,'verify','stage',p.id,p.attempt].join(':'),type:'verify',stage_id:p.id,unit:p.unit,attempt:p.attempt}]};}}
  }
  const eligible = u => ['pending','failed'].includes(u.status) && (!s.scope_units || s.scope_units.includes(u.id)) && u.attempt < (u.attempt_limit ?? s.limits.attempts_per_unit) && u.depends_on.every(d => s.units.find(x => x.id === d)?.status === 'accepted');
  if(prepPending&&!s.units.some(eligible)&&!s.units.some(u=>['running','verifying','delivered'].includes(u.status)))return emit(busy.length?'wait':'finish',{complete:false,reason:'preparation-incomplete'});
  if (s.units.some(u => u.status === 'blocked')) return emit('finish', { complete: false, reason: 'blocked-unit' });
  const continuation = (s.continuations || []).find(c => c.producer_stopped && c.revision === s.code_revision && s.units.some(u => u.id === c.unit && (u.status === 'running' || (u.status === 'verifying' && c.kind === 'review-correction' && correctionAvailable(u))) && u.attempt === c.attempt));
  if (continuation) { const u = s.units.find(u => u.id === continuation.unit); if (continuation.kind === 'review-correction') { u.correction_rounds = (u.correction_rounds || 0) + 1; u.status = 'running'; s.consumed.correction_rounds++; } u.phase_deadline_ms = s.now_ms + phaseMs; s.continuations = s.continuations.filter(c => c !== continuation); s.consumed.agent_calls++; return emit('dispatch', { unit: continuation.unit, attempt: continuation.attempt, resume: true, correction_round: u.correction_rounds || 0, receipt: continuation.receipt }); }
  const delivered = s.units.find(u => u.status === 'delivered');
  if (delivered) { delivered.status = 'verifying'; delivered.phase_deadline_ms = s.now_ms + phaseMs; s.consumed.agent_calls++; return emit('verify', { unit: delivered.id, attempt: delivered.attempt }); }
  if (s.units.some(u => ['running','verifying'].includes(u.status))) return emit('wait');
  const next = s.units.find(eligible) || s.units.find(u => ['pending','failed'].includes(u.status) && (!s.scope_units || s.scope_units.includes(u.id)) && u.depends_on.every(d => s.units.find(x => x.id === d)?.status === 'accepted'));
  if (!next) return emit('finish', { complete: false, reason: 'unsatisfied-dependencies' });
  if (next.attempt >= (next.attempt_limit ?? s.limits.attempts_per_unit)) return emit('finish', { complete: false, reason: 'attempts-exhausted', unit: next.id });
  next.attempt++; next.status = 'running'; next.phase_deadline_ms = s.now_ms + phaseMs; s.consumed.agent_calls++;
  return emit('dispatch', { unit: next.id, attempt: next.attempt });
};
globalThis.validateMemoryQualityActions = function(output, paths) {
  if (!output || !Array.isArray(output.actions) || output.actions.length > Math.min(8, output.state.limits.concurrency || 4)) throw Error('invalid actions');
  for (const a of output.actions) {
    if (!['dispatch','wait','request approval','verify','finish'].includes(a.type) || !a.id) throw Error('unknown action');
    if(a.preparation){const p=output.state.preparations?.find(p=>p.id===a.preparation_id);if(output.state.authorization?.parallel_preparation!==true||a.type!=='dispatch'||a.read_only!==true||!p||!p.read_only||p.session_id!==a.session_id||p.attempt!==a.attempt||p.status!=='running')throw Error('unauthorized preparation');}
    if(a.stage_id){const p=output.state.parallel_stages?.find(p=>p.id===a.stage_id);if(output.state.authorization?.parallel_implementation!==true||!p||p.authorized!==true||p.attempt!==a.attempt)throw Error('unauthorized stage');for(const path of p.paths||[])if(!paths.some(root=>memoryQualityContains(memoryQualityPath(root),memoryQualityPath(path))))throw Error('unauthorized stage path');if(a.type==='dispatch'&&(p.status!=='running'||p.session_id!==a.session_id||JSON.stringify(p.paths)!==JSON.stringify(a.paths)))throw Error('stage contract');}
    if (a.path && !paths.some(p => memoryQualityContains(memoryQualityPath(p),memoryQualityPath(a.path)))) throw Error('unauthorized path');
  }
  if (output.state.consumed.agent_calls > output.state.limits.agent_calls) throw Error('call limit');
  return output;
};
