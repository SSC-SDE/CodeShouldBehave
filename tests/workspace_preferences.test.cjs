const {test}=require('node:test');const assert=require('node:assert/strict');
const {defaults,validate,resolve,Store}=require('../src/workspace-preferences.js');const registry=require('../src/workspace-registry.json');
test('registry has unique IDs and real category/default/dependency contracts',()=>{assert.equal(new Set(registry.map(x=>x.id)).size,registry.length);for(const row of registry){assert.ok(row.label);assert.ok(['left','main','right'].includes(row.region));for(const dep of row.dependencies)assert.ok(registry.some(x=>x.id===dep));}assert.equal(new Set(registry.map(x=>x.category)).size,13);validate(defaults(registry),registry);});
test('Standard and Focus independent, full configurations validated',()=>{const doc=defaults(registry);doc.modes.standard.visibility.files=false;assert.equal(doc.modes.focus.visibility.files,false);doc.modes.focus.visibility.files=true;assert.equal(doc.modes.standard.visibility.files,false);doc.modes.standard.visibility.behavior=false;assert.throws(()=>validate(doc,registry));});
test('unknown keys, credentials, invalid widths and duplicate order rejected',()=>{for(const mutate of [d=>d.token='x',d=>d.modes.standard.widths.left=421,d=>d.modes.focus.order.main[0]=d.modes.focus.order.main[1],d=>d.modes.standard.filters.changedHld='yes']){const doc=defaults(registry);mutate(doc);assert.throws(()=>validate(doc,registry));}});
test('hidden dependencies suppress child; deep links reveal temporarily without mutation',()=>{const doc=defaults(registry);doc.modes.standard.visibility.source=false;const before=JSON.stringify(doc);assert.equal(resolve(registry,doc,'standard').visibility.expansion,false);assert.equal(resolve(registry,doc,'standard',null,['expansion']).visibility.source,true);assert.equal(JSON.stringify(doc),before);});
test('guest save remains local and temporary changes do not save implicitly',async()=>{const memory=new Map(),storage={getItem:k=>memory.get(k),setItem:(k,v)=>memory.set(k,v)},store=new Store(registry,{storage,request:()=>{throw Error('No network allowed');}});await store.load();store.temporary=defaults(registry);store.temporary.defaultMode='focus';assert.equal(memory.size,0);await store.save(store.temporary);const second=new Store(registry,{storage});await second.load();assert.equal(second.document.defaultMode,'focus');store.clear();assert.equal(store.document.defaultMode,'standard');});
test('cross-device revision conflicts preserve unsaved draft, owner determined by service',async()=>{let revision=0,document=defaults(registry);const request=async(path,options)=>{assert.equal(path,'/v1/me/workspace-preferences');if(options){const body=JSON.parse(options.body);assert.deepEqual(Object.keys(body),['revision','preferences']);if(body.revision!==revision)throw Object.assign(Error('conflict'),{status:409});revision++;document=body.preferences;}return {revision,preferences:document};};const a=new Store(registry,{request}),b=new Store(registry,{request});await a.load();await b.load();const changed=defaults(registry);changed.defaultMode='focus';await a.save(changed);await assert.rejects(b.save(changed),{status:409});assert.equal(b.revision,0);await b.load();assert.equal(b.document.defaultMode,'focus');});
test('in-flight loads cannot apply after logout or account change',async()=>{let finish;const store=new Store(registry,{request:()=>new Promise(r=>finish=r)});const pending=store.load();store.clear();const document=defaults(registry);document.defaultMode='focus';finish({revision:7,preferences:document});assert.equal(await pending,false);assert.equal(store.document.defaultMode,'standard');});
test('save account switch cannot import the old account draft',async()=>{let account='github-user:1';const store=new Store(registry,{request:async(path,options)=>{assert.equal(options,undefined);return {account,revision:0,preferences:defaults(registry)};}});await store.load();account='github-user:2';await assert.rejects(store.save(defaults(registry)),/Account changed/);assert.equal(store.account,null);});
test('optional evidence loads only on demand and at most once after failure',async()=>{const {ReviewWorkspace}=require('../src/workspace.js');const requests=[];const workspace=new ReviewWorkspace({optional:()=>false,request:async path=>{requests.push(path);if(path.includes('/graph?'))return {paths:[]};if(path.endsWith('/annotations'))throw Error('offline');return {items:[]};}});await workspace.selectReview('example');assert.ok(!requests.some(p=>p.includes('/annotations')||p.includes('/findings')));await workspace.loadOptional('annotations');await workspace.loadOptional('annotations');assert.equal(requests.filter(p=>p.endsWith('/annotations')).length,1);});
test('application themes survive storage and legacy preferences acquire Graphite',async()=>{
 const legacy=defaults(registry);delete legacy.theme;validate(legacy,registry);assert.equal(legacy.theme,'graphite');
 const memory=new Map(),storage={getItem:k=>memory.get(k),setItem:(k,v)=>memory.set(k,v)};
 for(const theme of ['graphite','blueprint','terminal','contrast']){const store=new Store(registry,{storage});const doc=defaults(registry);doc.theme=theme;await store.save(doc);const other=new Store(registry,{storage});await other.load();assert.equal(other.document.theme,theme);}
 const bad=defaults(registry);bad.theme='arbitrary-css';assert.throws(()=>validate(bad,registry));
});

test('HLD appearance defaults migrate independently and reject unknown styles',()=>{const doc=defaults(registry);assert.equal(doc.modes.focus.hldAppearance,'studio');assert.equal(doc.modes.standard.hldAppearance,'classic');delete doc.modes.focus.hldAppearance;validate(doc,registry);assert.equal(doc.modes.focus.hldAppearance,'studio');doc.modes.focus.hldAppearance='classic';validate(doc,registry);assert.equal(doc.modes.standard.hldAppearance,'classic');doc.modes.focus.hldAppearance='custom-css';assert.throws(()=>validate(doc,registry));});

test('legacy backdrop visibility and ordering migrate without resetting personal settings',async()=>{
 const legacy=defaults(registry);legacy.theme='terminal';legacy.modes.focus.widths.right=380;
 for(const mode of Object.values(legacy.modes)){delete mode.visibility.focusBackdrop;mode.order.main=mode.order.main.filter(id=>id!=='focusBackdrop').reverse();}
 const previous=structuredClone(legacy);validate(legacy,registry);
 for(const name of ['standard','focus']){
  assert.equal(legacy.modes[name].visibility.focusBackdrop,name==='focus');
  assert.deepEqual(legacy.modes[name].order.main.slice(0,-1),previous.modes[name].order.main);
 }
 assert.equal(legacy.theme,'terminal');assert.equal(legacy.modes.focus.widths.right,380);
 const once=structuredClone(legacy);validate(legacy,registry);assert.deepEqual(legacy,once);
 legacy.modes.focus.visibility.focusBackdrop=false;validate(legacy,registry);assert.equal(legacy.modes.focus.visibility.focusBackdrop,false);
 const memory=new Map([['csb.workspace-preferences.v1',JSON.stringify(previous)]]);
 const store=new Store(registry,{storage:{getItem:k=>memory.get(k),setItem:(k,v)=>memory.set(k,v)}});
 await store.load();assert.equal(store.document.modes.focus.visibility.focusBackdrop,true);
 await store.save(store.document);assert.equal(JSON.parse(memory.get('csb.workspace-preferences.v1')).modes.focus.visibility.focusBackdrop,true);
 delete legacy.modes.standard.visibility.files;assert.throws(()=>validate(legacy,registry));
});
test('Moria artwork remains validated colored text with relative same-origin assets',()=>{
 const {assetURL,validateArtwork}=require('../src/workspace-moria.js');
 const artwork=require('../src/workspace-moria.json');assert.equal(validateArtwork(artwork),artwork);
 assert.equal(assetURL('https://example.test/hosted/assets/workspace-moria.js','https://example.test/hosted/workspace').href,'https://example.test/hosted/assets/workspace-moria.json');
 assert.throws(()=>assetURL('https://other.test/workspace-moria.js','https://example.test/workspace'));
 for(const change of [a=>a.columns=1,a=>a.rows.pop(),a=>a.rows[0][0][0]=128,a=>a.rows[0][0][1]='\n']){
  const invalid=structuredClone(artwork);change(invalid);assert.throws(()=>validateArtwork(invalid));
 }
});

test('Ring v1 migration keeps Focus, ordering, and the existing browser storage key',async()=>{
 const legacy=defaults(registry);legacy.theme='blueprint';legacy.defaultMode='focus';
 legacy.modes.standard.widths.left=280;legacy.modes.focus.visibility.focusBackdrop=false;
 for(const mode of Object.values(legacy.modes)){delete mode.visibility.ringBackdrop;mode.order.main=mode.order.main.filter(id=>id!=='ringBackdrop').reverse();}
 const before=structuredClone(legacy);validate(legacy,registry);
 for(const name of ['standard','focus']){
  assert.equal(legacy.modes[name].visibility.ringBackdrop,name==='standard');
  const expected=structuredClone(before.modes[name]);expected.visibility.ringBackdrop=name==='standard';expected.order.main.push('ringBackdrop');
  assert.deepEqual(legacy.modes[name],expected);
 }
 assert.equal(legacy.theme,'blueprint');assert.equal(legacy.defaultMode,'focus');
 const once=structuredClone(legacy);validate(legacy,registry);assert.deepEqual(legacy,once);
 legacy.modes.standard.visibility.ringBackdrop=false;legacy.modes.focus.visibility.ringBackdrop=true;
 validate(legacy,registry);assert.equal(legacy.modes.standard.visibility.ringBackdrop,false);assert.equal(legacy.modes.focus.visibility.ringBackdrop,true);
 const memory=new Map([['csb.workspace-preferences.v1',JSON.stringify(before)]]);
 const store=new Store(registry,{storage:{getItem:k=>memory.get(k),setItem:(k,v)=>memory.set(k,v)}});
 await store.load();await store.save(store.document);
 assert.deepEqual([...memory.keys()],['csb.workspace-preferences.v1']);
 assert.deepEqual(JSON.parse(memory.get('csb.workspace-preferences.v1')),once);
 for(const invalid of ['yes',null,1]){const bad=defaults(registry);bad.modes.standard.visibility.ringBackdrop=invalid;assert.throws(()=>validate(bad,registry));}
 const duplicate=defaults(registry);duplicate.modes.standard.order.main.push('ringBackdrop');assert.throws(()=>validate(duplicate,registry));
});
test('oldest v1 preferences acquire both backdrops without resetting or accepting new versions',()=>{
 const legacy=defaults(registry);
 for(const mode of Object.values(legacy.modes)){for(const id of ['focusBackdrop','ringBackdrop'])delete mode.visibility[id];mode.order.main=mode.order.main.filter(id=>!['focusBackdrop','ringBackdrop'].includes(id));}
 validate(legacy,registry);
 assert.deepEqual(legacy,defaults(registry));
 legacy.version=2;assert.throws(()=>validate(legacy,registry));
});
