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

async function moriaHarness({saved=null,reduced=false,forced=false,hidden=false,fail=false,storageBlocked=false}={}){
 const {readFileSync}=require('node:fs'),{join}=require('node:path'),vm=require('node:vm');
 const source=readFileSync(join(__dirname,'../src/workspace-moria.js'),'utf8');
 class Element{
  constructor(tag){this.tagName=tag;this.children=[];this.dataset={};this.attributes={};this.handlers={};this.classes=new Set();this.hidden=false;this.renders=0;this.classList={contains:name=>this.classes.has(name),toggle:(name,on)=>on?this.classes.add(name):this.classes.delete(name)};}
  set className(value){this.classes=new Set(value.split(/\s+/));}
  setAttribute(name,value){this.attributes[name]=value;}
  addEventListener(name,handler){this.handlers[name]=handler;}
  append(...nodes){for(const node of nodes)this.children.push(...(node.tagName==='#fragment'?node.children:[node]));}
  prepend(node){this.children.unshift(node);}
  replaceChildren(...nodes){this.renders++;this.children=[];this.append(...nodes);}
 }
 const body=new Element('body'),topbar=new Element('header');body.append(topbar);
 const find=(node,id)=>node.id===id?node:node.children.map(child=>find(child,id)).find(Boolean);
 const document={body,hidden,currentScript:{src:'https://example.test/assets/workspace-moria.js'},handlers:{},getElementById:id=>find(body,id),querySelector:selector=>selector==='.topbar'?topbar:null,createElement:tag=>new Element(tag),createDocumentFragment:()=>new Element('#fragment'),createTextNode:text=>Object.assign(new Element('#text'),{textContent:text}),addEventListener(name,handler){this.handlers[name]=handler;}};
 const media=new Map();
 for(const [name,matches] of [['(prefers-reduced-motion: reduce)',reduced],['(forced-colors: active)',forced]])media.set(name,{matches,addEventListener(_,handler){this.change=handler;}});
 const memory=new Map(saved===null?[]:[['csb.moria-motion.v1',saved]]),requests=[],observations=[];
 let refresh;
 const sandbox={document,URL,MutationObserver:class{constructor(callback){refresh=callback;}observe(target,options){observations.push({target,options});}},fetch:async(url,options)=>{requests.push({url,options});return {ok:!fail,json:async()=>require('../src/workspace-moria.json')};},window:{location:{href:'https://example.test/workspace'},matchMedia:name=>media.get(name),localStorage:{getItem:key=>{if(storageBlocked)throw Error('Storage blocked');return memory.get(key);},setItem:(key,value)=>{if(storageBlocked)throw Error('Storage blocked');memory.set(key,value);}}}};
 const execute=()=>vm.runInNewContext(source,sandbox);execute();
 const host=document.getElementById('workspace-moria'),select=document.getElementById('workspace-moria-mode'),control=topbar.children[0],grid=host.children[0].children[0];
 const settle=()=>new Promise(resolve=>setImmediate(resolve));
 return {document,body,topbar,host,select,control,grid,memory,requests,observations,execute,settle,
  async activate(mode='focus-desk-active',visible=true){body.classes.delete('focus-desk-active');body.classes.delete('focus-lld-active');if(mode)body.classes.add(mode);host.classList.toggle('preference-hidden',!visible);refresh();await settle();},
  choose(value){select.value=value;select.handlers.change();},
  media(name,value){const query=media.get(name);query.matches=value;query.change();},
  visibility(value){document.hidden=value;document.handlers.visibilitychange();},
  refresh:()=>refresh()};
}

test('Moria renders original ASCII once and shares one control across Focus HLD and LLD',async()=>{
 const h=await moriaHarness();assert.equal(h.requests.length,0);assert.equal(h.control.hidden,true);
 await h.activate();assert.equal(h.requests.length,1);assert.equal(h.host.dataset.visible,'true');assert.equal(h.host.dataset.running,'true');
 assert.equal(h.select.attributes['aria-label'],'Backdrop');assert.equal(h.host.attributes['aria-hidden'],'true');assert.ok('inert' in h.host.attributes);
 assert.equal(h.requests[0].url.href,'https://example.test/assets/workspace-moria.json');
 assert.deepEqual({...h.requests[0].options},{credentials:'same-origin',mode:'same-origin',redirect:'error'});
 const expected=require('../src/workspace-moria.json').rows.map(row=>row.map(run=>run[1]).join('')).join('\n');
 assert.equal(h.grid.children.map(node=>node.textContent).join(''),expected);
 assert.equal(h.grid.renders,1);assert.equal(h.host.children[0].children.length,5);assert.equal(h.host.children[0].children[4].children.length,6);
 await h.activate('focus-lld-active');h.refresh();h.execute();
 assert.equal(h.host.dataset.running,'true');assert.equal(h.requests.length,1);assert.equal(h.grid.renders,1);assert.equal(h.topbar.children.length,1);
 await h.activate(null);assert.equal(h.host.dataset.visible,'false');assert.equal(h.host.dataset.running,'false');assert.equal(h.control.hidden,true);
 assert.equal(h.observations.length,2);
 for(const observation of h.observations){assert.equal(observation.options.attributes,true);assert.deepEqual([...observation.options.attributeFilter],['class']);assert.equal(observation.options.subtree,undefined);}
});

test('Moria Live Still Off choices persist without mutating profile visibility',async()=>{
 const h=await moriaHarness();await h.activate('focus-lld-active');
 h.choose('still');assert.equal(h.host.dataset.visible,'true');assert.equal(h.host.dataset.running,'false');assert.equal(h.host.dataset.motion,'still');
 h.choose('off');assert.equal(h.host.dataset.visible,'false');assert.equal(h.host.classList.contains('preference-hidden'),false);
 assert.equal(h.memory.get('csb.moria-motion.v1'),'off');
 const restored=await moriaHarness({saved:h.memory.get('csb.moria-motion.v1')});await restored.activate();
 assert.equal(restored.select.value,'off');assert.equal(restored.requests.length,0);assert.equal(restored.control.hidden,false);
 restored.choose('live');await restored.settle();assert.equal(restored.requests.length,1);assert.equal(restored.host.dataset.running,'true');
 await restored.activate('focus-desk-active',false);assert.equal(restored.host.dataset.visible,'false');assert.equal(restored.host.dataset.running,'false');assert.equal(restored.control.hidden,true);
 const blocked=await moriaHarness({saved:'invalid',storageBlocked:true});await blocked.activate();blocked.choose('still');assert.equal(blocked.host.dataset.motion,'still');
 const invalid=await moriaHarness({saved:'invalid'});await invalid.activate();assert.equal(invalid.select.value,'live');
});

test('Moria reduced motion forced colors and hidden tabs prevent live work',async()=>{
 const h=await moriaHarness({saved:'live',reduced:true,hidden:true});await h.activate('focus-lld-active');
 assert.equal(h.requests.length,0);assert.equal(h.select.value,'still');assert.equal(h.select.children[0].disabled,true);
 h.visibility(false);await h.settle();assert.equal(h.host.dataset.running,'false');assert.equal(h.host.dataset.visible,'true');
 h.media('(prefers-reduced-motion: reduce)',false);assert.equal(h.host.dataset.running,'true');assert.equal(h.select.value,'live');
 h.visibility(true);assert.equal(h.host.dataset.running,'false');h.visibility(false);assert.equal(h.host.dataset.running,'true');
 h.media('(forced-colors: active)',true);assert.equal(h.host.dataset.visible,'false');assert.equal(h.host.dataset.running,'false');assert.equal(h.control.hidden,true);
 h.media('(forced-colors: active)',false);assert.equal(h.host.dataset.running,'true');assert.equal(h.grid.renders,1);
 const forced=await moriaHarness({forced:true});await forced.activate();assert.equal(forced.requests.length,0);
 const hiddenPreference=await moriaHarness();await hiddenPreference.activate('focus-lld-active',false);assert.equal(hiddenPreference.requests.length,0);
});

test('Moria unavailable artwork leaves review usable without repeated fetches',async()=>{
 const h=await moriaHarness({fail:true});await h.activate();
 const status=h.document.getElementById('workspace-moria-status');
 assert.equal(status.hidden,false);assert.equal(status.attributes.role,'status');assert.equal(h.host.dataset.visible,'false');
 await h.activate('focus-lld-active');assert.equal(h.requests.length,1);
 h.choose('off');assert.equal(status.hidden,true);await h.activate(null);assert.equal(h.control.hidden,true);
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
