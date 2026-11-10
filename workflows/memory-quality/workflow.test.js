globalThis.testMemoryQuality = function() {
  const base = () => ({run_id:'test',state_version:1,now_ms:1,deadline_ms:100,code_revision:'r',approved:true,preflight_ok:true,route_verified:true,consumed:{agent_calls:0,no_progress_rounds:0,correction_rounds:0},limits:{agent_calls:24,no_progress_rounds:2,correction_rounds:3,attempts_per_unit:2},units:[{id:'a',status:'pending',attempt:0,depends_on:[]},{id:'b',status:'pending',attempt:0,depends_on:['a']}],results:[]});
  let n=0; const check=(ok,name)=>{if(!ok)throw Error(name); n++;};
  const step=s=>validateMemoryQualityActions(memoryQualityStep(s),['/authorized']);
  let s=base(); let o=step(s); check(o.actions[0].type==='dispatch'&&o.actions[0].unit==='a','dependencies');
  s=o.state; check(step(s).actions[0].type==='wait','missing result');
  const r={run_id:'test',unit:'a',attempt:1,revision:'r',event_id:'e1',producer_stopped:true,outcome:'done',evidence:['line']};
  s.results=[r]; o=step(s); check(o.actions[0].type==='verify','producer separation');
  s=o.state; s.results=[{...r,event_id:'e2',independent:true,accepted:true}]; o=step(s); check(o.actions[0].unit==='b'&&o.state.units[0].status==='accepted','accept and advance');
  s=o.state; s.results=[r,r]; check(step(s).actions[0].type==='wait','duplicates');
  s=base(); s.units[0].status='failed';s.units[0].attempt=2;check(step(s).actions[0].reason==='attempts-exhausted','attempt limit');
  s=base();s.now_ms=100;check(step(s).actions[0].complete===false,'expiry');
  s=base();s.consumed.agent_calls=24;check(step(s).actions[0].type==='finish','call limit');
  s=base();s.consumed.no_progress_rounds=2;check(step(s).actions[0].reason==='limit','no progress');
  s=base();s.approved=false;check(step(s).actions[0].type==='request approval','approval');
  s=base();s.units[0].status='accepted';s.units[1].status='blocked';check(step(s).actions[0].complete===false,'required incomplete');
  s=base();s.units.forEach(u=>u.status='accepted');check(step(s).actions[0].complete===true,'success');
  s=base();s.units[0].status='running';s.units[0].attempt=1;s.results=[{...r,evidence:[]}];check(step(s).state.units[0].attempt===2,'invalid result retries bounded');
  s=base();s.units[0].status='running';s.units[0].attempt=1;s.results=[{...r,revision:'old'}];check(step(s).actions[0].type==='wait','late revision');
  let rejected=false;try{validateMemoryQualityActions({state:base(),actions:[{id:'x',type:'shell'}]},[]);}catch{rejected=true;}check(rejected,'unknown action');
  rejected=false;try{validateMemoryQualityActions({state:base(),actions:[{id:'x',type:'dispatch',path:'/escape'}]},['/authorized']);}catch{rejected=true;}check(rejected,'path boundary');
  s=base();s.units[0].status='failed';s.units[0].attempt=2;s.units[0].attempt_limit=3;o=step(s);check(o.actions[0].type==='dispatch'&&o.actions[0].attempt===3,'authorized extra attempt');
  s=o.state;s.units[0].status='failed';check(step(s).actions[0].reason==='attempts-exhausted','extra attempt ceiling');
  s=base();s.scope_units=['a'];s.units[0].status='accepted';o=step(s);check(o.actions[0].scope_complete===true&&o.actions[0].complete===false&&o.state.units[1].attempt===0,'scope stops next phase');
  s=base();s.now_ms=100;s.units[0].status='verifying';s.units[0].attempt=1;s.results=[{...r,outcome:'blocked',independent:true}];check(step(s).actions[0].type==='finish','terminal result clears busy before deadline gate');
  s=base();s.units[0].status='running';s.units[0].attempt=1;s.results=[{...r,producer_stopped:false}];o=step(s);o.state.results=[r];check(step(o.state).actions[0].type==='verify','not stopped result remains recoverable');
  s=base();s.deadline_ms=null;s.deadline_mode='user-open-ended';s.now_ms=999999;o=step(s);check(o.actions[0].type==='dispatch'&&o.state.units[0].phase_deadline_ms>s.now_ms,'open global deadline still bounds phases');
  s=base();s.units[0].correction_rounds=4;check(step(s).actions[0].type==='finish','per unit correction cap');
  s=base();s.units[0].status='running';s.units[0].attempt=1;s.continuations=[{unit:'a',attempt:1,revision:'r',producer_stopped:true,receipt:'command'}];o=step(s);check(o.actions[0].resume===true&&o.state.units[0].attempt===1,'command receipt resumes same attempt');
  s=base();s.units[0].status='running';s.units[0].attempt=1;s.continuations=[{unit:'a',attempt:1,revision:'r',producer_stopped:false}];check(step(s).actions[0].type==='wait','cannot resume active producer');
  s=base();s.units[0].status='verifying';s.units[0].attempt=1;s.continuations=[{unit:'a',attempt:1,revision:'r',producer_stopped:true,kind:'review-correction',receipt:'review'}];o=step(s);check(o.actions[0].resume===true&&o.state.units[0].status==='running'&&o.state.units[0].correction_rounds===1,'review remediation same delivery');
  s=base();s.units[0].status='verifying';s.units[0].attempt=1;s.units[0].correction_rounds=3;s.continuations=[{unit:'a',attempt:1,revision:'r',producer_stopped:true,kind:'review-correction'}];check(step(s).actions[0].type==='wait','review correction cap blocks dispatch');
  s=base();s.units[0].status='verifying';s.units[0].attempt=1;s.units[0].correction_rounds=3;s.units[0].correction_limit=6;s.continuations=[{unit:'a',attempt:1,revision:'r',producer_stopped:true,kind:'review-correction'}];o=step(s);check(o.actions[0].correction_round===4,'explicit per-unit renewal');
  s=o.state;s.units[0].status='verifying';s.units[0].correction_rounds=6;s.continuations=[{unit:'a',attempt:1,revision:'r',producer_stopped:true,kind:'review-correction'}];check(step(s).actions[0].type==='wait','renewed ceiling still blocks');
  s=base();s.authorization={correction_rounds_unbounded:true};s.limits.correction_mode='user-unbounded';s.units[0].status='verifying';s.units[0].attempt=1;s.units[0].correction_rounds=1000;s.continuations=[{unit:'a',attempt:1,revision:'r',producer_stopped:true,kind:'review-correction'}];o=step(s);check(o.actions[0].correction_round===1001,'authorized unbounded correction rounds');
  s=o.state;s.units[0].status='verifying';s.consumed.no_progress_rounds=2;s.continuations=[{unit:'a',attempt:1,revision:'r',producer_stopped:true,kind:'review-correction'}];check(step(s).actions[0].type!=='dispatch','unbounded corrections still stop without progress');
  s=base();s.limits.correction_mode='user-unbounded';s.units[0].correction_rounds=1000;check(step(s).actions[0].type==='finish','unbounded requires explicit authorization');
  const pb=()=>{const x=base();x.authorization={parallel_preparation:true};x.limits.concurrency=4;x.units[0].status='accepted';x.units[1].status='running';x.units[1].attempt=1;x.preparations=['f','m','i'].map(id=>({id,unit:id,session_id:'clean-'+id,status:'pending',read_only:true,required:true,attempt:0,depends_on:['a']}));return x;};
  s=pb();o=step(s);check(o.actions.length===3&&o.state.consumed.agent_calls===3,'three-way readonly fanout');
  check(step(o.state).actions[0].type==='wait','missing preparations wait');
  s=pb();s.limits.concurrency=2;check(step(s).actions.length===1,'active unit occupies slot');
  s=pb();s.consumed.agent_calls=23;o=step(s);check(o.actions.length===1&&o.state.consumed.agent_calls===24,'fanout budget bounded');
  s=pb();s.consumed.agent_calls=24;check(step(s).actions.every(a=>a.type!=='dispatch'),'budget stops fanout');
  s=pb();s.authorization.parallel_preparation=false;check(step(s).actions[0].type==='wait','fanout authorization');
  s=pb();s.units[0].status='pending';check(step(s).actions[0].type==='wait','fanout dependencies');
  s=pb();s.preparations.forEach(p=>p.read_only=false);check(step(s).actions[0].type==='wait','writes never fanout');
  s=pb();s.preparations.forEach(p=>p.attempt=2);check(step(s).actions[0].type==='wait','fanout attempts');
  s=pb();o=step(s);s=o.state;s.now_ms=2000000;o=step(s);check(o.state.preparations.every(p=>p.status==='expired'&&p.needs_reconciliation)&&o.actions[0].type==='wait','expired producer reconciles not restarts');
  s=pb();o=step(s);s=o.state;const pr={id:'f',run_id:'test',attempt:1,revision:'r',event_id:'p1',producer_stopped:true,outcome:'done',evidence:['source']};s.preparation_results=[pr,pr];o=step(s);check(o.state.preparations[0].processed.length===1&&o.state.preparations[0].status==='delivered','duplicate not acceptance');
  s=o.state;s.preparation_results=[{...pr,event_id:'p2',independent:true,accepted:true}];o=step(s);check(o.state.preparations[0].status==='accepted'&&o.state.units[1].status==='running','prep acceptance not product acceptance');
  s=pb();o=step(s);s=o.state;s.preparation_results=[{...pr,revision:'old'}];check(step(s).state.preparations[0].status==='running','late revision ignored');
  s=pb();o=step(s);s=o.state;s.preparation_results=[{...pr,evidence:[]}];check(step(s).state.preparations[0].status==='failed','invalid prep incomplete');
  s=pb();s.units.forEach(u=>u.status='accepted');s.preparations.forEach(p=>p.attempt=2);o=step(s);check(o.actions[0].complete===false,'required prep prevents complete');
  s=pb();s.units.forEach(u=>u.status='accepted');s.preparations.forEach(p=>p.status='accepted');check(step(s).actions[0].complete===true,'all accepted completes');
  rejected=false;try{validateMemoryQualityActions({state:pb(),actions:[{id:'bad',type:'dispatch',preparation:true,preparation_id:'f',session_id:'clean-f',attempt:1,read_only:false}]},[]);}catch{rejected=true;}check(rejected,'validator denies prep writes');
  const sb=()=>{const x=pb();x.preparations.forEach(p=>p.status='accepted');x.authorization.parallel_implementation=true;x.parallel_stages=[{id:'safe',unit:'f',session_id:'safe-clean',status:'pending',authorized:true,required:true,attempt:0,depends_on:['f'],paths:['/authorized/sibling.rs']}];return x;};
  s=sb();o=step(s);check(o.actions[0].stage_id==='safe'&&o.state.consumed.agent_calls===1,'independent stage dispatch');
  s=sb();s.write_locks=[{paths:['/authorized/sibling.rs']}];check(step(s).actions[0].type==='wait','writer collision blocks');
  s=sb();s.authorization.parallel_implementation=false;check(step(s).actions[0].type==='wait','write stage explicit authorization');
  s=sb();s.preparations[0].status='delivered';check(step(s).actions[0].type==='wait','unaccepted preparation blocks stage');
  s=sb();s.consumed.agent_calls=24;check(step(s).actions[0].type!=='dispatch','stage budget stops');
  s=sb();s.limits.concurrency=1;check(step(s).actions[0].type==='wait','stage concurrency includes main writer');
  s=sb();o=step(s);s=o.state;s.now_ms=2000000;o=step(s);check(o.state.parallel_stages[0].status==='expired'&&o.state.parallel_stages[0].needs_reconciliation,'expired stage never silently restarts');
  s=sb();o=step(s);s=o.state;const sr={id:'safe',run_id:'test',attempt:1,revision:'r',event_id:'stage1',producer_stopped:true,outcome:'done',evidence:['source']};s.stage_results=[sr,sr];o=step(s);check(o.actions[0].type==='verify'&&o.state.parallel_stages[0].processed.length===1,'stage delivery separate from review');
  s=o.state;s.stage_results=[{...sr,event_id:'stage2',independent:true,accepted:true}];o=step(s);check(o.state.parallel_stages[0].status==='accepted'&&o.state.units[1].status==='running','stage acceptance never accepts parent product');
  s=sb();s.units.forEach(u=>u.status='accepted');s.parallel_stages[0].attempt=2;check(step(s).actions[0].complete===false,'incomplete stage prevents finish');
  rejected=false;try{s=sb();s.parallel_stages[0].paths=['/escape'];validateMemoryQualityActions(memoryQualityStep(s),['/authorized']);}catch{rejected=true;}check(rejected,'stage path validation');
  s=sb();o=step(s);s=o.state;s.parallel_stages[0].status='verifying';s.stage_continuations=[{id:'safe',attempt:1,revision:'r',producer_stopped:true,kind:'review-correction',session_id:'fresh-correction'}];o=step(s);check(o.actions[0].continuation&&o.actions[0].session_id==='fresh-correction'&&o.state.parallel_stages[0].attempt===1&&o.state.parallel_stages[0].correction_rounds===1,'stage correction clean session same attempt');
  s=sb();o=step(s);s=o.state;s.stage_continuations=[{id:'safe',attempt:1,revision:'r',producer_stopped:false,session_id:'fresh'}];check(step(s).actions[0].type==='wait','active producer continuation denied');
  s=sb();o=step(s);s=o.state;s.stage_continuations=[{id:'safe',attempt:1,revision:'old',producer_stopped:true,session_id:'fresh'}];check(step(s).actions[0].type==='wait','stale stage continuation denied');
  s=sb();o=step(s);s=o.state;s.consumed.agent_calls=24;s.stage_continuations=[{id:'safe',attempt:1,revision:'r',producer_stopped:true,session_id:'fresh'}];check(step(s).actions.every(a=>a.type!=='dispatch'),'stage continuation budget enforced');
  s=sb();o=step(s);s=o.state;s.parallel_stages[0].correction_rounds=3;s.stage_continuations=[{id:'safe',attempt:1,revision:'r',producer_stopped:true,kind:'review-correction',session_id:'fresh'}];check(step(s).actions[0].type==='wait','stage correction limit enforced');
  s=sb();o=step(s);s=o.state;s.stage_continuations=[{id:'safe',attempt:1,revision:'r',producer_stopped:true,kind:'test-receipt',session_id:'fresh-repair'}];o=step(s);check(o.actions[0].continuation&&o.state.parallel_stages[0].attempt===1&&!o.state.parallel_stages[0].correction_rounds,'test receipt resumes stage without extra round');
  return {passed:n};
};

// Load only the pure transition module, never the effectful workflow entrypoint.
import('node:test').then(({ test }) => {
  test('pure memory quality transitions', async t => {
    const { default: assert } = await import('node:assert/strict');
    await import('./workflow.js');
    await t.test('existing transition contracts', () => assert.ok(testMemoryQuality().passed > 0));
    const base = () => ({run_id:'pure',state_version:0,now_ms:10,deadline_ms:null,deadline_mode:'user-open-ended',code_revision:'r',approved:true,preflight_ok:true,route_verified:true,consumed:{agent_calls:0,no_progress_rounds:0,correction_rounds:0},limits:{agent_calls:30,no_progress_rounds:2,correction_rounds:3,attempts_per_unit:2,phase_ms:20,concurrency:4},units:[{id:'a',status:'pending',attempt:0,depends_on:[]}],results:[]});
    await t.test('normalized segment overlaps serialize held and batch writers', () => {
      for (const [left,right,conflict] of [
        ['/authorized/src','/authorized/src/a.rs',true],
        ['/authorized/src/a.rs','/authorized/src',true],
        ['/authorized/./src','/authorized//src/a.rs',true],
        ['/authorized/src/../lib','/authorized/lib/a.rs',true],
        ['/','/authorized/a.rs',true],
        ['/authorized/src','/authorized/src2/a.rs',false],
        ['/authorized/lib/a.rs','/authorized/lib/b.rs',false]
      ]) {
        for (const held of ['batch','running','lock']) {
          const s = base(); s.units=[]; s.authorization={parallel_implementation:true};
          s.parallel_stages=[{id:'first',unit:'first',status:'pending',authorized:true,required:true,attempt:0,depends_on:[],session_id:'one',paths:[left]},{id:'second',unit:'second',status:'pending',authorized:true,required:true,attempt:0,depends_on:[],session_id:'two',paths:[right]}];
          if (held === 'running') Object.assign(s.parallel_stages[0],{status:'running',attempt:1,deadline_ms:100});
          if (held === 'lock') { s.parallel_stages.shift(); s.write_locks=[{paths:[left]}]; }
          const o = memoryQualityStep(s);
          const expected = held === 'batch' ? (conflict ? ['first'] : ['first','second']) : (conflict ? [] : ['second']);
          assert.deepEqual(o.actions.filter(a=>a.type==='dispatch').map(a=>a.stage_id),expected,held+': '+left+' vs '+right);
          validateMemoryQualityActions(o,['/']);
          assert.equal(s.parallel_stages.at(-1).attempt,0,'input remains immutable');
        }
      }
    });
    await t.test('stage continuation retains locks against nested active or quarantined writers', () => {
      for (const owner of ['lock','running','expired']) {
        const s=base(); s.units=[]; s.authorization={parallel_implementation:true};
        s.parallel_stages=[{id:'p',unit:'p',session_id:'old',status:'delivered',authorized:true,required:true,attempt:1,depends_on:[],paths:['/authorized/src/a']}];
        if(owner==='lock') s.write_locks=[{paths:['/authorized/./src']}];
        else s.parallel_stages.push({id:'owner',unit:'owner',session_id:'owner',status:owner,authorized:true,required:true,attempt:1,depends_on:[],deadline_ms:100,needs_reconciliation:owner==='expired',paths:['/authorized//src']});
        s.stage_continuations=[{id:'p',attempt:1,revision:'r',producer_stopped:true,session_id:'fresh'}];
        const o=memoryQualityStep(s); assert.equal(o.actions.some(a=>a.type==='dispatch'),false);
      }
    });
    await t.test('lexical authorization normalizes roots and rejects boundary or traversal escapes', () => {
      for (const [root,path,allowed] of [
        ['/','/a',true], ['/authorized/','/authorized//./src',true],
        ['/authorized/./src','/authorized/src/a',true],
        ['/authorized/src','/authorized/src/../src/a',true],
        ['/authorized/src','/authorized/src/../outside',false],
        ['/authorized','/authorized2/a',false], ['/authorized','/authorized/../../authorized/a',false],
        ['/authorized','relative/a',false], ['/authorized','/authorized/a\0b',false],
        ['relative','/authorized/a',false]
      ]) {
        const s=base(); s.units=[]; s.authorization={parallel_implementation:true};
        s.parallel_stages=[{id:'p',unit:'p',session_id:'clean',status:'pending',authorized:true,required:true,attempt:0,depends_on:[],paths:[path]}];
        const o=memoryQualityStep(s);
        if (allowed) assert.doesNotThrow(()=>validateMemoryQualityActions(o,[root]));
        else assert.throws(()=>validateMemoryQualityActions(o,[root]),/unauthorized stage path/);
        const plain={state:base(),actions:[{id:'plain',type:'dispatch',path}]};
        if (allowed) assert.doesNotThrow(()=>validateMemoryQualityActions(plain,[root]));
        else assert.throws(()=>validateMemoryQualityActions(plain,[root]),/unauthorized path/);
      }
    });
    await t.test('missing unit result expires and quarantines before retry or global finish', () => {
      for (const globalDeadline of [null,25]) {
        let s=base(); s.deadline_ms=globalDeadline;
        let o=memoryQualityStep(s); assert.equal(o.actions[0].type,'dispatch'); assert.equal(o.state.units[0].phase_deadline_ms,30);
        s=o.state; s.now_ms=29; o=memoryQualityStep(s); assert.equal(o.actions[0].type,'wait');
        s=o.state; s.now_ms=30; o=memoryQualityStep(s);
        assert.equal(o.state.units[0].status,'expired'); assert.equal(o.state.units[0].needs_reconciliation,true);
        assert.deepEqual(o.actions.map(a=>[a.type,a.reason,a.active]),[['wait','phase-expired',['a']]]);
        s=o.state; s.now_ms=1000; o=memoryQualityStep(s); assert.equal(o.state.units[0].attempt,1); assert.equal(o.actions[0].type,'wait');
        s=o.state; s.continuations=[{unit:'a',attempt:1,revision:'r',producer_stopped:false}];
        o=memoryQualityStep(s); assert.equal(o.actions[0].type,'wait');
        s=o.state; s.continuations=[{unit:'a',attempt:1,revision:'old',producer_stopped:true}];
        o=memoryQualityStep(s); assert.equal(o.actions[0].type,'wait');
        s=o.state; s.continuations=[{unit:'a',attempt:1,revision:'r',producer_stopped:true}];
        o=memoryQualityStep(s);
        assert.equal(o.state.units[0].needs_reconciliation,false);
        assert.equal(o.actions[0].type,globalDeadline===null?'dispatch':'finish');
        assert.equal(o.state.units[0].attempt,globalDeadline===null?2:1);
        if(globalDeadline===null) assert.equal(o.state.units[0].phase_deadline_ms,1020);
      }
    });
    await t.test('late stopped delivery proceeds but stale writer cannot replace reviewer or retry', () => {
      const r={run_id:'pure',unit:'a',attempt:1,revision:'r',event_id:'delivery',producer_stopped:true,outcome:'done',evidence:['proof']};
      let s=memoryQualityStep(base()).state; s.now_ms=31;
      let o=memoryQualityStep(s); assert.equal(o.state.units[0].status,'expired');
      s=o.state; s.results=[r]; o=memoryQualityStep(s);
      assert.equal(o.actions[0].type,'verify'); assert.equal(o.state.units[0].needs_reconciliation,false); assert.equal(o.state.units[0].phase_deadline_ms,51);
      s=o.state; s.now_ms=40; s.results=[{...r,event_id:'writer-conflict',outcome:'blocked'}];
      o=memoryQualityStep(s); assert.equal(o.state.units[0].status,'verifying'); assert.equal(o.actions[0].type,'wait');
      s=o.state; s.now_ms=51; o=memoryQualityStep(s); assert.equal(o.state.units[0].expired_phase,'verifying');
      s=o.state; s.results=[{...r,event_id:'late-writer'}]; o=memoryQualityStep(s); assert.equal(o.state.units[0].status,'expired');
      s=o.state; s.continuations=[{unit:'a',attempt:1,revision:'r',producer_stopped:true,receipt:'stale-writer'}];
      o=memoryQualityStep(s); assert.equal(o.state.units[0].status,'expired'); assert.equal(o.actions[0].type,'wait');
      s=o.state; s.results=[{...r,event_id:'review',independent:true,accepted:true}];
      o=memoryQualityStep(s); assert.equal(o.state.units[0].status,'accepted'); assert.equal(o.actions[0].complete,true);
      s=o.state; s.results=[{...r,event_id:'after-accept',outcome:'failed'}]; assert.equal(memoryQualityStep(s).state.units[0].status,'accepted');
      s=memoryQualityStep(base()).state; s.now_ms=31; s=memoryQualityStep(s).state;
      s.continuations=[{unit:'a',attempt:1,revision:'r',producer_stopped:true}]; s=memoryQualityStep(s).state;
      assert.equal(s.units[0].attempt,2); s.results=[r]; o=memoryQualityStep(s);
      assert.equal(o.state.units[0].status,'running'); assert.equal(o.actions[0].type,'wait');
    });
    await t.test('reconciled final attempt finishes only after the stopped receipt', () => {
      const input=base(); input.limits.attempts_per_unit=1;
      let s=memoryQualityStep(input).state; s.now_ms=30; let o=memoryQualityStep(s);
      assert.equal(o.actions[0].type,'wait'); assert.equal(o.state.units[0].needs_reconciliation,true);
      s=o.state; s.continuations=[{unit:'a',attempt:1,revision:'r',producer_stopped:true}]; o=memoryQualityStep(s);
      assert.equal(o.actions[0].type,'finish'); assert.equal(o.actions[0].reason,'attempts-exhausted'); assert.equal(o.state.units[0].attempt,1);
    });
    await t.test('verification and resumed phases receive fresh bounded deadlines', () => {
      let s=memoryQualityStep(base()).state; s.now_ms=20;
      s.continuations=[{unit:'a',attempt:1,revision:'r',producer_stopped:true,receipt:'command'}];
      let o=memoryQualityStep(s); assert.equal(o.actions[0].resume,true); assert.equal(o.state.units[0].phase_deadline_ms,40);
      s=o.state; s.now_ms=25; s.results=[{run_id:'pure',unit:'a',attempt:1,revision:'r',event_id:'done',producer_stopped:true,outcome:'done',evidence:['proof']}];
      o=memoryQualityStep(s); assert.equal(o.actions[0].type,'verify'); assert.equal(o.state.units[0].phase_deadline_ms,45);
      s=o.state; s.now_ms=30; s.continuations=[{unit:'a',attempt:1,revision:'r',producer_stopped:true,kind:'review-correction'}];
      o=memoryQualityStep(s); assert.equal(o.actions[0].resume,true); assert.equal(o.state.units[0].phase_deadline_ms,50);
      s=o.state; s.now_ms=50; assert.equal(memoryQualityStep(s).state.units[0].status,'expired');
    });
    await t.test('required preparation and stage wait for dispatchable pending or failed dependencies', () => {
      for (const kind of ['preparation','stage']) for (const status of ['pending','failed']) {
        let s=base(); s.units[0].status=status; s.units[0].attempt=status==='failed'?1:0;
        s.authorization={parallel_preparation:true,parallel_implementation:true};
        if(kind==='preparation') s.preparations=[{id:'prep',unit:'prep',session_id:'clean',status:'pending',read_only:true,required:true,attempt:0,depends_on:['a']}];
        else s.parallel_stages=[{id:'stage',unit:'stage',session_id:'clean',status:'pending',authorized:true,required:true,attempt:0,depends_on:['a'],paths:['/authorized/a']}];
        const o=memoryQualityStep(s);
        assert.equal(o.actions[0].type,'dispatch'); assert.equal(o.actions[0].unit,'a');
        assert.equal(o.actions[0].attempt,status==='failed'?2:1);
      }
      let s=base(); s.authorization={parallel_preparation:true};
      s.preparations=[{id:'prep',unit:'prep',session_id:'clean',status:'pending',read_only:true,required:true,attempt:0,depends_on:['a']}];
      let o=memoryQualityStep(s); s=o.state;
      s.results=[{run_id:'pure',unit:'a',attempt:1,revision:'r',event_id:'invalid',producer_stopped:true,outcome:'done',evidence:[]}];
      o=memoryQualityStep(s); assert.equal(o.actions[0].type,'dispatch'); assert.equal(o.actions[0].attempt,2);
      for (const restriction of ['scope','attempts','dependency']) {
        s=base(); s.preparations=[{required:true,status:'pending'}];
        if(restriction==='scope') s.scope_units=['other'];
        if(restriction==='attempts') s.units[0].attempt=2;
        if(restriction==='dependency') s.units[0].depends_on=['missing'];
        assert.equal(memoryQualityStep(s).actions[0].reason,'preparation-incomplete');
      }
    });
  });
});
