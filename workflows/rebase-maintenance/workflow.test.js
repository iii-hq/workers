globalThis.testRebaseMaintenance = function () {
  let passed = 0;
  const ok = (b, m) => { if (!b) throw Error(m); passed++; };
  const base = () => ({run_id:'test',version:0,now:1,approved:true,route_verified:true,snapshot_ready:true,no_progress:0,calls:0,limit:4,concurrency:3,input_revision:'r',groups:['a','b','c'].map(id=>({id,session:id,paths:['/root/'+id],status:'pending',attempt:0}))});
  const step = s => validateRebaseMaintenance(rebaseMaintenanceStep(s), '/root');
  let s = base(), o = step(s); ok(o.actions.length===3&&o.state.calls===3,'fanout');
  ok(step(o.state).actions[0].type==='wait','missing results wait');
  s=o.state; s.results=s.groups.map(g=>({group:g.id,run_id:'test',attempt:1,revision:'r',producer_stopped:true,event_id:g.id,outcome:'done',evidence:['source']}));
  o=step(s); ok(o.actions[0].target==='coordinator-checks','results to checks');
  s=o.state;s.checks={acceptable:true};o=step(s);ok(o.actions[0].target==='independent-review'&&o.state.calls===4,'independent review reserved');
  s=o.state;s.review={status:'done',accepted:true,stopped:true};ok(step(s).actions[0].complete===true,'complete only after review');
  s=base();s.approved=false;ok(step(s).actions[0].type==='request approval','approval');
  s=base();s.route_verified=false;ok(step(s).actions[0].reason==='prerequisite','route');
  s=base();s.snapshot_ready=false;ok(step(s).actions[0].reason==='prerequisite','backup');
  s=base();s.limit=1;ok(step(s).actions.length===1,'budget');
  s=base();s.calls=4;ok(step(s).actions[0].complete===false,'exhausted no dispatch');
  s=base();s.concurrency=1;ok(step(s).actions.length===1,'concurrency');
  s=base();s.groups[1].paths=['/root/a'];ok(step(s).actions.length===2,'lock');
  s=base();o=step(s);s=o.state;s.now=1200002;o=step(s);ok(o.state.groups.every(g=>g.reconcile)&&o.actions[0].type==='wait','expiry preserves active work');
  s=base();s.groups[0].status='blocked';ok(step(s).actions[0].complete===false,'partial failure');
  s=base();s.no_progress=2;ok(step(s).actions[0].reason==='no-progress','stopping');
  s=base();s.deadline=1;ok(step(s).actions[0].reason==='deadline','deadline');
  s=base();o=step(s);s=o.state;const r={group:'a',run_id:'test',attempt:1,revision:'r',producer_stopped:true,event_id:'e',outcome:'done',evidence:['proof']};s.results=[r,r];o=step(s);ok(o.state.groups[0].processed.length===1,'duplicate');
  s=base();o=step(s);s=o.state;s.results=[{...r,revision:'old'}];ok(step(s).state.groups[0].status==='running','late revision');
  s=base();o=step(s);s=o.state;s.results=[{...r,producer_stopped:false}];ok(step(s).state.groups[0].status==='running','producer not stopped');
  s=base();o=step(s);s=o.state;s.results=[{...r,evidence:[]}];ok(step(s).state.groups[0].status==='blocked','invalid report');
  s=base();s.groups[0].attempt=2;ok(step(s).actions.length===2,'attempt cap');
  s=base();s.groups.forEach(g=>g.status='delivered');s.checks={acceptable:false};ok(step(s).actions[0].reason==='checks-blocked','failed checks');
  s=base();s.groups.forEach(g=>g.status='delivered');s.checks={acceptable:true};s.review={status:'done',accepted:true,stopped:false};ok(step(s).actions[0].complete===false,'stopped reviewer required');
  let rejected=false;try{s=base();s.groups[0].paths=['/root/../escape'];step(s);}catch{rejected=true;}ok(rejected,'path escape');
  return {passed};
};

// No workflow registration or external effects: import the pure step only.
import('node:test').then(({ test }) => {
  test('pure maintenance result transitions', async t => {
    const { default: assert } = await import('node:assert/strict');
    await import('./workflow.js');
    await t.test('existing transition contracts', () => assert.ok(testRebaseMaintenance().passed > 0));
    const base=()=>({run_id:'pure',version:0,now:10,approved:true,route_verified:true,snapshot_ready:true,no_progress:0,calls:0,limit:4,concurrency:3,input_revision:'r',groups:[{id:'a',session:'clean',paths:['/root/a'],status:'pending',attempt:0}]});
    const result={group:'a',run_id:'pure',attempt:1,revision:'r',producer_stopped:true,event_id:'delivery',outcome:'done',evidence:['proof']};
    await t.test('late result releases expiry reconciliation and reaches checks without global deadline', () => {
      let o=rebaseMaintenanceStep(base()); assert.equal(o.actions[0].type,'dispatch'); assert.equal(o.state.groups[0].expires_at,1200010);
      let s=o.state; s.now=1200009; o=rebaseMaintenanceStep(s); assert.equal(o.state.groups[0].status,'running');
      s=o.state; s.now=1200010; o=rebaseMaintenanceStep(s);
      assert.equal(o.state.groups[0].status,'expired'); assert.equal(o.state.groups[0].reconcile,true); assert.equal(o.actions[0].type,'wait');
      s=o.state; s.now=1200011; s.results=[result]; o=rebaseMaintenanceStep(s);
      assert.equal(o.state.groups[0].status,'delivered'); assert.equal(o.state.groups[0].reconcile,false);
      assert.deepEqual(o.actions.map(a=>[a.type,a.target]),[['verify','coordinator-checks']]);
      s=o.state; s.checks={acceptable:true}; o=rebaseMaintenanceStep(s); assert.equal(o.actions[0].target,'independent-review');
    });
    await t.test('pending attempt zero cannot accept unsolicited delivery or failure', () => {
      for(const outcome of ['done','blocked']) {
        const s=base(); s.results=[{...result,attempt:0,outcome}]; const o=rebaseMaintenanceStep(s);
        assert.equal(o.actions[0].type,'dispatch'); assert.equal(o.state.groups[0].status,'running');
        assert.equal(o.state.groups[0].attempt,1); assert.equal(o.state.groups[0].processed,undefined);
      }
    });
    await t.test('delivered duplicates and repeated event IDs preserve the first outcome', () => {
      for(const events of [[result,{...result,event_id:'conflict',outcome:'blocked'}],[result,{...result,outcome:'blocked'}]]) {
        let s=rebaseMaintenanceStep(base()).state; s.results=events; let o=rebaseMaintenanceStep(s);
        assert.equal(o.state.groups[0].status,'delivered'); assert.deepEqual(o.state.groups[0].processed,['delivery']);
        s=o.state; s.results=[{...result,event_id:'after-delivery',outcome:'failed'}]; o=rebaseMaintenanceStep(s);
        assert.equal(o.state.groups[0].status,'delivered'); assert.deepEqual(o.state.groups[0].processed,['delivery']);
        assert.equal(o.actions[0].target,'coordinator-checks');
      }
      let s=rebaseMaintenanceStep(base()).state; s.results=[{...result,outcome:'blocked'},result]; const o=rebaseMaintenanceStep(s);
      assert.equal(o.state.groups[0].status,'blocked'); assert.equal(o.actions[0].reason,'incomplete');
    });
    await t.test('expired results still require matching run attempt revision and stopped producer', () => {
      let s=rebaseMaintenanceStep(base()).state; s.now=1200010; s=rebaseMaintenanceStep(s).state;
      for(const change of [{run_id:'other'},{attempt:0},{revision:'old'},{producer_stopped:false}]) {
        const input={...s,results:[{...result,...change}]}; const o=rebaseMaintenanceStep(input);
        assert.equal(o.state.groups[0].status,'expired'); assert.equal(o.state.groups[0].reconcile,true); assert.equal(o.actions[0].type,'wait');
      }
    });
  });
});
