'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const {ReviewWorkspace,workspaceHttpError,clearHldEvidence,clearLldEvidence,clearReviewEvidence,LldCatalogView,LldSourcePairView,validateLldResponse,lldOutcome,lldCatalogRows,lldMermaid,lldEvidence,LLD_INITIAL_SCALE,diagramScale,encodeWorkspaceRoute,decodeWorkspaceRoute}=require('../src/workspace.js');

const identity={review_id:'r-1',scope_version:'scope-1',parent_hld:'hld-proposal',component:'api'};
const selection={parentHld:'hld-proposal',component:'api'};
const tick=()=>new Promise(resolve=>setImmediate(resolve));
function evidence(id,side='proposal',start=2,end=5){return {id,commit:(side==='baseline'?'a':'b').repeat(40),path:'src/api.rs',blob:(side==='baseline'?'1':'2').repeat(40),start_line:start,end_line:end};}
function entry(id='e-proposal',side='proposal'){return {id,name:'handle',kind:'function',logical_key:'pkg::api::impl Trait for Handler::handle',path:'src/api.rs',blob:(side==='baseline'?'1':'2').repeat(40),start_line:2,end_line:5,module:'pkg::api',container:'impl Trait for Handler',visibility:'private',signature:'fn handle(value: u32) -> u32',detail_state:'available',evidence_ids:[`source-${side}`],parent:{hld_design_id:side==='baseline'?'hld-baseline':'hld-proposal',component_id:'api',state:'matched'}};}
function catalog(){return {...identity,baseline:{id:'cat-base',snapshot:'a'.repeat(40)},proposal:{id:'cat-proposal',snapshot:'b'.repeat(40)},entries:[{baseline:entry('e-base','baseline'),proposal:entry(),status:'modified'}],delta:{outcome:'change_detected',changes:[]},next_cursor:null,can_export:true};}
function detail(id='e-proposal'){
  const make=side=>({id:`detail-${side}-${id}`,catalog_id:side==='baseline'?'cat-base':'cat-proposal',entry:entry(side==='baseline'?'e-base':id,side),label:'Static local control flow',graph:{nodes:[{id:`condition-${side}`,region_id:'outer',operation:'condition',label:'value > 1',resolution:'syntactic',evidence_id:`node-${side}`}],edges:[],regions:[{id:'outer',kind:'function'}],complete:true},evidence:[evidence(`source-${side}`,side),evidence(`node-${side}`,side,3,3)],coverage:{indexed:1,inventory_complete:true,structure_complete:true}});
  return {...identity,entry_id:id,baseline:make('baseline'),proposal:make('proposal'),delta:{id:'delta-1',outcome:'change_detected',changes:[{classification:'condition_changed',resolution:'observed_syntax'}]},can_accept:true,can_export:true};
}
function sourceFor(path){const query=new URLSearchParams(path.split('?')[1]),id=query.get('evidence'),side=id.endsWith('baseline')?'baseline':'proposal';return {...identity,evidence_id:id,commit:(side==='baseline'?'a':'b').repeat(40),path:'src/api.rs',blob_id:(side==='baseline'?'1':'2').repeat(40),start_line:Number(query.get('start_line')),end_line:Number(query.get('end_line')),text:side==='baseline'?'fn handle(v: u32) {\n  if v > 1 {}\n}':'fn handle(v: u32) {\n  if v > 2 {}\n}',truncated:false};}
function request(overrides={}) {return async(path,options={})=>{
  if(path.includes('/lld/source?'))return overrides.source?overrides.source(path,options):sourceFor(path);
  if(path.includes('/lld/catalog?'))return overrides.catalog?overrides.catalog(path,options):catalog();
  if(path.includes('/lld/detail?'))return overrides.detail?overrides.detail(path,options):detail(new URLSearchParams(path.split('?')[1]).get('entry'));
  if(path.includes('/lld/'))return overrides.write?overrides.write(path,options):{...identity,preview_id:'private-draft',expected_revision:1,input_fingerprint:'fp-1'};
  if(path.includes('/graph?'))return {kind:'children',paths:[]};
  if(path.includes('/files?')||path.includes('/findings?')||path.endsWith('/annotations'))return {review_id:'r-1',items:[]};
  return {review_id:path.includes('r-2')?'r-2':'r-1',proposal:'b'.repeat(40)};
};}
async function workspace(overrides={}) {const value=new ReviewWorkspace({request:request(overrides)});await value.selectReview('r-1');await value.loadLldCatalog(selection);return value;}

test('LLD stays lazy and component drill-down keeps the exact HLD and review',async()=>{
  const calls=[],base=request(),value=new ReviewWorkspace({request:async(path,options)=>{calls.push(path);return base(path,options);}});
  await value.selectReview('r-1');assert.ok(calls.every(path=>!path.includes('/lld')));
  await value.loadLldCatalog(selection);assert.match(calls.at(-1),/parent_hld=hld-proposal/);assert.match(calls.at(-1),/component=api/);
  assert.equal(value.state.lld.entries[0].proposal.id,'e-proposal');assert.equal(value.state.lldDetail,null);
});

test('LLD outcome keeps incomplete and unavailable separate from covered no change',()=>{
  assert.equal(lldOutcome({outcome:'incomplete'}),'LLD comparison incomplete');assert.equal(lldOutcome(null),'LLD unavailable');assert.match(lldOutcome({outcome:'no_change_detected_within_scope'}),/within covered scope/);
});

test('LLD starts at readable scale and short evidence cards do not inherit full file pane heights',()=>{
  assert.equal(diagramScale(1700,250,LLD_INITIAL_SCALE),1,'a narrow viewport must pan without shrinking local-flow labels');assert.ok(diagramScale(1700,250,'fit')<.2,'Fit all/width remains an explicit overview choice');
  const css=fs.readFileSync(require.resolve('../src/workspace.css'),'utf8');assert.match(css,/\.lld-signature,\.lld-exact-code,\.lld-box-preview pre\{min-height:0\}/);assert.match(css,/pre\{min-height:420px/,'ordinary code-pane defaults remain unchanged');
});

test('mismatched parent, component, scope and review response pins are rejected',()=>{
  for(const change of [{review_id:'other'},{parent_hld:'other'},{component:'other'},{scope_version:'other'}])assert.throws(()=>validateLldResponse({...identity,...change},{reviewId:'r-1',...selection,scopeVersion:'scope-1'}),/does not match/);
  assert.throws(()=>validateLldResponse({...identity,entry_id:'other'},{reviewId:'r-1',...selection,entry:'e-proposal'}),/does not match/);
});

test('a late component catalog cannot restore the previous drill-down',async()=>{
  let release;const value=await workspace();value.request=request({catalog:path=>path.includes('component=old')?new Promise(resolve=>{release=resolve;}):Promise.resolve({...catalog(),component:'new'})});
  const old=value.loadLldCatalog({...selection,component:'old'});await value.loadLldCatalog({...selection,component:'new'});release({...catalog(),component:'old'});await old;
  assert.equal(value.state.lld.component,'new');assert.equal(value.state.lldSelection.component,'new');
});

test('pagination refuses a different immutable catalog generation',async()=>{
  const value=await workspace({catalog:path=>({...catalog(),proposal:{id:path.includes('cursor=')?'different':'cat-proposal'},next_cursor:'opaque'})});
  await value.loadLldCatalog({...selection,cursor:'opaque'});assert.equal(value.state.lld,null);assert.match(value.state.lldError,/catalog changed/);
});

test('removed declarations keep their baseline occurrence and qualified name',()=>{
  assert.deepEqual(lldCatalogRows(null),[]);
  const rows=lldCatalogRows({...catalog(),entries:[{baseline:entry('old','baseline'),proposal:null,status:'removed_or_unmatched'}]});
  assert.equal(rows[0].id,'old');assert.equal(rows[0].entry.logical_key,'pkg::api::impl Trait for Handler::handle');assert.equal(rows[0].proposal,null);
});

test('obsolete selected detail cannot replace the new function',async()=>{
  let release;const value=await workspace({detail:path=>path.includes('entry=first')?new Promise(resolve=>{release=resolve;}):Promise.resolve(detail('second'))});
  const first=value.loadLldDetail('first');await value.loadLldDetail('second');release(detail('first'));await first;await tick();assert.equal(value.state.lldDetail.entry_id,'second');
});

test('selection cancels obsolete HTTP work and a late missing response cannot erase the new detail',async()=>{
  let rejectOld,signal;const base=request(),value=await workspace();value.request=(path,options)=>path.includes('entry=old')&&path.includes('/lld/detail?')?new Promise((resolve,reject)=>{rejectOld=reject;signal=options.signal;}):base(path,options);
  const old=value.loadLldDetail('old');await value.loadLldDetail('new');assert.equal(signal.aborted,true);rejectOld(Object.assign(new Error('Old missing detail'),{status:404}));await old;await tick();assert.equal(value.state.lldDetail.entry_id,'new');assert.ok(value.state.lld);
});

test('before and after source are bound to the selected detail and exact revisions',async()=>{
  const value=await workspace();await value.loadLldDetail('e-proposal');await tick();
  assert.equal(value.state.lldSourceComparison.baseline.commit,'a'.repeat(40));assert.equal(value.state.lldSourceComparison.proposal.commit,'b'.repeat(40));assert.notEqual(value.state.lldSourceComparison.baseline.text,value.state.lldSourceComparison.proposal.text);
});

test('fresh detail reauthorization reuses source pairs only for the same immutable detail IDs',async()=>{
  let calls=0,details=0;const value=await workspace({source:path=>{calls++;return sourceFor(path);},detail:()=>{details++;return detail();}});await value.loadLldDetail('e-proposal');await tick();assert.equal(calls,2);const pair=value.state.lldSourceComparison;await value.loadLldDetail('e-proposal');await tick();assert.equal(details,2);assert.equal(calls,2);assert.equal(value.state.lldSourceComparison,pair);
  value.request=request({detail:()=>{throw Object.assign(new Error('Revoked'),{status:403});}});await value.loadLldDetail('e-proposal');assert.equal(value.state.lldSourceComparison,null);assert.equal(value.state.lldDetail,null);
});

test('source requests include the selected entry and bounded evidence span',async()=>{
  const calls=[],value=await workspace({source:path=>{calls.push(path);return sourceFor(path);}});await value.loadLldDetail('e-proposal');await tick();
  await value.loadLldSource(evidence('source-proposal','proposal',2,500));const query=new URLSearchParams(calls.at(-1).split('?')[1]);assert.equal(query.get('entry'),'e-proposal');assert.equal(query.get('start_line'),'2');assert.equal(query.get('end_line'),'201');
});

test('wrong blob or evidence IDs cannot substitute source from another revision',async()=>{
  const value=await workspace({source:path=>({...sourceFor(path),blob_id:'wrong'})});await value.loadLldDetail('e-proposal');await tick();await value.loadLldSource(evidence('node-proposal','proposal',3,3));
  assert.equal(value.state.lldSource,null);assert.match(value.state.lldError,/immutable evidence/);assert.ok(value.state.lldSourceComparison.errors.length);
});

test('late hover source does not replace the next selected function',async()=>{
  let release;const value=await workspace();await value.loadLldDetail('e-proposal');await tick();value.request=request({source:path=>path.includes('evidence=node-proposal')?new Promise(resolve=>{release=resolve;}):Promise.resolve(sourceFor(path))});
  const hover=value.loadLldSource(evidence('node-proposal','proposal',3,3));await value.loadLldDetail('next');release(sourceFor('/source?evidence=node-proposal&start_line=3&end_line=3'));await hover;await tick();assert.equal(value.state.lldDetail.entry_id,'next');assert.equal(value.state.lldSource,null);
});

test('revocation clears catalogs, details, source pairs and pending assertions',async()=>{
  const value=await workspace();await value.loadLldDetail('e-proposal');await tick();value.state.lldPreview={response:{preview_id:'private'}};value.request=request({source:()=>{throw Object.assign(new Error('Revoked'),{status:403});}});await value.loadLldSource(evidence('node-proposal','proposal',3,3));
  for(const field of ['lld','lldDetail','lldSource','lldSourceComparison','lldPreview','details','graph'])assert.equal(value.state[field],null);
});

test('missing LLD evidence clears stale LLD visuals while preserving ordinary source review',async()=>{
  let emitted;const value=await workspace();value.onState=state=>{emitted=state;};await value.loadLldDetail('e-proposal');await tick();value.request=request({source:()=>{throw Object.assign(new Error('Not retained'),{status:404});}});await value.loadLldSource(evidence('node-proposal','proposal',3,3));
  assert.equal(value.state.lld,null);assert.equal(value.state.lldDetail,null);assert.equal(emitted.lldSourceComparison,null);assert.ok(value.state.details);assert.equal(value.state.lldError,'Not retained');
});

test('nondisclosing LLD current-authority 404 clears all prior code and HLD evidence',async()=>{
  const value=await workspace();await value.loadLldDetail('e-proposal');await tick();value.state.source={text:'previous code'};value.state.sourceComparison={baseline:{text:'previous baseline'}};value.state.hld={baseline:{id:'previous-hld'}};value.state.hldPreview={response:{private:'draft'}};value.request=request({source:()=>{throw Object.assign(new Error('lld_unavailable_in_current_context'),{status:404});}});await value.loadLldSource(evidence('node-proposal','proposal',3,3));
  for(const field of ['lld','lldDetail','lldSource','lldSourceComparison','lldPreview','lldHistory','details','graph','source','sourceComparison','hld','hldPreview'])assert.equal(value.state[field],null);assert.equal(value.state.lldError,'lld_unavailable_in_current_context');
});

test('a late current-authority 404 from an obsolete detail cannot erase a newer selection',async()=>{
  let rejectOld;const base=request(),value=await workspace();value.request=(path,options)=>path.includes('entry=old')&&path.includes('/lld/detail?')?new Promise((resolve,reject)=>{rejectOld=reject;}):base(path,options);const old=value.loadLldDetail('old');await value.loadLldDetail('new');rejectOld(Object.assign(new Error('lld_unavailable_in_current_context'),{status:404}));await old;await tick();assert.equal(value.state.lldDetail.entry_id,'new');assert.ok(value.state.details);
});

test('LLD assertions require maintainer capability and a matching current preview',async()=>{
  const value=await workspace();await value.loadLldDetail('e-proposal');await tick();value.state.lldDetail.can_accept=false;await assert.rejects(value.previewLld({entry:'e-proposal'}),/maintainer/);await assert.rejects(value.acceptLld({entry:'e-proposal'}),/Preview/);
});

test('an assertion edited during preview cannot become approved by its late response',async()=>{
  let release;const value=await workspace({write:()=>new Promise(resolve=>{release=resolve;})});await value.loadLldDetail('e-proposal');await tick();const preview=value.previewLld({entry:'e-proposal',design_id:'detail-proposal-e-proposal',corrections:[]});value.invalidateLldPreview();release({...identity,preview_id:'private',expected_revision:1,input_fingerprint:'fp'});await assert.rejects(preview,/assertion changed/);assert.equal(value.state.lldPreview,null);
});

test('LLD confirmation sends immutable preview pins with default activation explicit',async()=>{
  const writes=[],value=await workspace({write:(path,options)=>{writes.push(JSON.parse(options.body));return {...identity,preview_id:'private',expected_revision:3,input_fingerprint:'fp-current'};}});await value.loadLldDetail('e-proposal');await tick();const body={entry:'e-proposal',design_id:'detail-proposal-e-proposal',interpretation:'intended',corrections:[{kind:'lld_invariant',value:'Return an acknowledged result'}]};await value.previewLld(body);await assert.rejects(value.acceptLld({...body,design_id:'other'}),/Preview/);await value.acceptLld(body,{knownGapsAccepted:true});assert.equal(writes[1].input_fingerprint,'fp-current');assert.equal(writes[1].expected_revision,3);assert.equal(writes[1].activate_default,false);assert.equal(writes[1].parent_hld,'hld-proposal');assert.equal(writes[1].entry,'e-proposal');
});

test('LLD Mermaid draws only observed local edges and separate nested exit scopes',()=>{
  const value=detail();value.proposal.graph.nodes.push({id:'closure-return',region_id:'closure',operation:'return',label:'return 0',resolution:'syntactic'});value.proposal.graph.regions.push({id:'closure',kind:'closure'});value.proposal.graph.edges=[{from:'condition-proposal',to:'missing',kind:'invented'},{from:'condition-proposal',to:'closure-return',kind:'syntax_relation',label:'observed edge'}];const output=lldMermaid(value);assert.match(output.source,/closure · separate exit scope/);assert.match(output.source,/observed edge/);assert.doesNotMatch(output.source,/invented/);assert.equal(output.nodes.length,3);
});

test('LLD labels are escaped and unresolved call effects remain unresolved',()=>{
  const value=detail();value.proposal.graph.nodes[0].label='x"]\nclick evil "javascript:alert(1)"\n<script>bad</script>';value.proposal.graph.nodes[0].resolution='target_and_effect_unresolved';value.proposal.graph.nodes[0].operation='call';const output=lldMermaid(value,'proposal');assert.match(output.source,/:::unknown/);assert.doesNotMatch(output.source,/<script>|javascript:|\nclick /);assert.doesNotMatch(output.source,/network effect verified|database operation/);
});

test('condition labels preserve comparison operators without injecting markup',()=>{
  const value=detail();value.proposal.graph.nodes[0].label='value < 2 && value > 0';const output=lldMermaid(value,'proposal');assert.match(output.source,/value &lt; 2 &amp;&amp; value &gt; 0/);
});

test('module diagrams expose source declarations without claiming they are executed flows',()=>{
  const value=detail();value.proposal.graph.nodes=[];value.proposal.entry.kind='module';value.proposal.declarations=[entry('child')];const output=lldMermaid(value,'proposal');assert.match(output.source,/Declaration structure/);assert.match(output.source,/contains declaration/);assert.equal(output.nodes[1].entry.id,'child');assert.doesNotMatch(output.source,/Static local control flow/);
});

test('node source selects its exact evidence instead of a neighboring function excerpt',()=>{
  const value=detail().proposal;assert.equal(lldEvidence(value,value.graph.nodes[0])[0].id,'node-proposal');assert.equal(lldEvidence(value)[0].id,'source-proposal');assert.deepEqual(lldEvidence(value,{evidence_id:'not-retained'}),[]);
});

test('LLD routes restore exact function, node and pin without changing legacy routes',()=>{
  const route=encodeWorkspaceRoute({reviewId:'r-1',layout:'architecture',designLevel:'lld',designElement:'api',parentHld:'hld-proposal',component:'api',detail:'flow',lldElement:'e-proposal',lldNode:'condition-proposal',lldPinned:true});const decoded=decodeWorkspaceRoute(route);assert.equal(decoded.parentHld,'hld-proposal');assert.equal(decoded.lldElement,'e-proposal');assert.equal(decoded.lldNode,'condition-proposal');assert.equal(decoded.lldPinned,true);assert.equal(decodeWorkspaceRoute('#review=r-1&layout=connected&side=diff').layout,'connected');assert.equal(decodeWorkspaceRoute(route.replace('lld_pin=1','lld_pin=yes')),null);
});

test('explicit correspondence restores immutable counterpart IDs and never transfers a previous preview',async()=>{
  const value=await workspace({detail:()=>({...detail(),baseline:{...detail().baseline,entry:entry('renamed-base','baseline')}})});
  await value.loadLldDetail('e-proposal',{baselineEntry:'renamed-base'});await tick();assert.equal(value.state.lldDetail.baseline.entry.id,'renamed-base');assert.equal(value.state.lldBaselineEntry,'renamed-base');assert.equal(value.state.lldPreview,null);
  const route=encodeWorkspaceRoute({reviewId:'r-1',layout:'architecture',designLevel:'lld',parentHld:'hld-proposal',component:'api',lldElement:'e-proposal',lldBaselineEntry:'renamed-base',lldCorrespondence:'move'}),decoded=decodeWorkspaceRoute(route);assert.equal(decoded.lldBaselineEntry,'renamed-base');assert.equal(decoded.lldCorrespondence,'move');assert.equal(decodeWorkspaceRoute(route.replace('lld_correspondence=move','lld_correspondence=equal')),null);
});

test('explicit counterpart mismatch is rejected before it can expose a false paired graph',async()=>{
  const value=await workspace();await value.loadLldDetail('e-proposal',{baselineEntry:'different-base'});assert.equal(value.state.lldDetail,null);assert.match(value.state.lldError,/correspondence does not match/);
});

test('disabled detail leaves authorized immutable assertion history separate from current behavior',async()=>{
  const base=request(),value=await workspace();value.request=async(path,options)=>path.includes('/lld/detail?')?Promise.reject(Object.assign(new Error('new_lld_generation_disabled'),{status:503})):path.includes('/lld/history?')?{...identity,entry_id:'e-proposal',current_behavior:'unavailable_without_new_detail_generation',accepted:{accepted_id:'accepted-old',observed_design:{snapshot:'c'.repeat(40)}},generation_enabled:false}:base(path,options);
  await value.loadLldDetail('e-proposal');assert.equal(value.state.lldDetail,null);assert.match(value.state.lldError,/disabled/);await value.loadLldHistory('e-proposal');assert.equal(value.state.lldHistory.accepted.observed_design.snapshot,'c'.repeat(40));assert.equal(value.state.lldDetail,null);value.clearReview();assert.equal(value.state.lldHistory,null);
});

test('local graph omits an unused unknown frontier but retains every observed unknown edge',()=>{
  const value=detail();value.proposal.graph.nodes.push({id:'unused',operation:'unknown_exit',label:'Unknown frontier'});assert.doesNotMatch(lldMermaid(value,'proposal').source,/Unknown frontier/);value.proposal.graph.edges.push({from:'condition-proposal',to:'unused',label:'unsupported exit (unknown)'});const output=lldMermaid(value,'proposal');assert.match(output.source,/Unknown frontier/);assert.match(output.source,/\|"unsupported exit \(unknown\)"\|/);
});


test('real JSON authority404 decoding clears the complete cached review rather than only LLD',async()=>{
  const value=await workspace();await value.loadLldDetail('e-proposal');await tick();value.state.source={text:'cached ordinary source'};value.state.hld={proposal:{id:'hld-proposal'}};
  value.request=request({detail:async()=>{throw await workspaceHttpError(new Response(JSON.stringify({error:'lld_unavailable_in_current_context'}),{status:404,headers:{'Content-Type':'application/json; charset=utf-8'}}));}});
  await value.loadLldDetail('e-proposal');assert.equal(value.state.details,null);assert.equal(value.state.source,null);assert.equal(value.state.hld,null);assert.equal(value.state.lld,null);assert.equal(value.state.lldDetail,null);assert.equal(value.state.loading.size,0);
});

test('real optional-frontier JSON404 preserves ordinary diff and stale authority404 cannot erase a new selection',async()=>{
  const value=await workspace();const ordinary={text:'ordinary exact source'};value.state.source=ordinary;
  value.request=request({detail:async()=>{throw await workspaceHttpError(new Response('{"error":"lld_not_generated_regenerate_review"}',{status:404,headers:{'Content-Type':'application/json'}}));}});await value.loadLldDetail('e-proposal');assert.equal(value.state.source,ordinary);assert.ok(value.state.details);assert.equal(value.state.lld,null);
  await value.loadLldCatalog(selection);const base=request();
  let rejectOld;value.request=(path,options)=>path.includes('entry=old')&&path.includes('/lld/detail?')?new Promise((resolve,reject)=>{rejectOld=reject;}):base(path,options);const old=value.loadLldDetail('old');await value.loadLldDetail('new');rejectOld(await workspaceHttpError(new Response('{"error":"lld_unavailable_in_current_context"}',{status:404,headers:{'Content-Type':'application/json'}})));await old;assert.equal(value.state.lldDetail.entry_id,'new');assert.ok(value.state.details);
});

test('transport errors expose only whitelisted local404 codes with a bounded body',async()=>{
  for(const response of [new Response('<html>provider secret</html>',{status:404,headers:{'Content-Type':'text/html'}}),new Response('{"error":"provider-secret"}',{status:404,headers:{'Content-Type':'application/json'}}),new Response('{bad JSON',{status:404,headers:{'Content-Type':'application/json'}}),new Response('{"error":"repository_unavailable"}',{status:403,headers:{'Content-Type':'application/json'}})]){const error=await workspaceHttpError(response);assert.match(error.message,/^Request failed \((404|403)\)$/);assert.equal(error.code,undefined);}
  assert.equal((await workspaceHttpError(new Response('private text',{status:401}))).message,'Session expired or invalid');let cancelled=false;const body=new ReadableStream({start(controller){controller.enqueue(new Uint8Array(4097));},cancel(){cancelled=true;}});const error=await workspaceHttpError(new Response(body,{status:404,headers:{'Content-Type':'application/json'}}));assert.equal(error.message,'Request failed (404)');assert.equal(cancelled,true);
});


test('an aborted404 body preserves AbortError and cannot invalidate a newer hover in the same detail',async()=>{
  const value=await workspace();await value.loadLldDetail('e-proposal');await tick();const base=request();let started;
  value.request=async(path,options)=>{if(path.includes('evidence=node-proposal')){let stream;const body=new ReadableStream({start(controller){stream=controller;started=true;}});options.signal.addEventListener('abort',()=>stream.error(new DOMException('Body cancelled','AbortError')));throw await workspaceHttpError(new Response(body,{status:404,headers:{'Content-Type':'application/json'}}));}return base(path,options);};
  const obsolete=value.loadLldSource(evidence('node-proposal','proposal',3,3));await tick();assert.equal(started,true);await value.loadLldSource(evidence('source-proposal','proposal',2,5));await obsolete;assert.ok(value.state.lld);assert.equal(value.state.lldDetail.entry_id,'e-proposal');assert.equal(value.state.lldSource.evidence_id,'source-proposal');assert.equal(value.state.lldError,null);
});


test('authority loss removes hidden HLD diagrams, exports, corrections and dynamic evidence options',()=>{
  const fields=new Map();const getElementById=id=>{if(!fields.has(id))fields.set(id,{textContent:'private architectural assertion',value:'private/boundary',children:['private architecture'],firstElementChild:{textContent:'private Mermaid or receipt'},dataset:{designId:'same-immutable-design'},replaceChildren(){this.children=[];this.textContent='';}});return fields.get(id);};
  const staticInterpretation=getElementById('hld-interpretation');clearHldEvidence({getElementById});
  for(const id of ['hld-diagrams','hld-inspector','hld-element-list','hld-correction-element','hld-document-evidence','hld-correspondence-sources'])assert.deepEqual(getElementById(id).children,[]);
  for(const id of ['hld-full-identity','hld-drift-status','hld-correspondence-source-identity'])assert.equal(getElementById(id).textContent,'');
  for(const id of ['hld-mermaid-source','hld-confirmation-preview']){assert.equal(getElementById(id).firstElementChild.textContent,'');assert.equal(getElementById(id).hidden,true);}
  for(const id of ['hld-source','hld-export','hld-confirm','hld-accepted'])assert.equal(getElementById(id).disabled,true);
  assert.equal(getElementById('hld-correction-element').dataset.designId,undefined,'same-ID reauthorization must repopulate cleared confirmation options');assert.equal(getElementById('hld-responsibility').value,'');assert.equal(staticInterpretation.textContent,'private architectural assertion','fixed form choices are not rewritten by permission loss');
});

test('authority loss clears inactive LLD and structural exports, private scope options and source identities',()=>{
  const fields=new Map();const getElementById=id=>{if(!fields.has(id))fields.set(id,{textContent:'private path or design',value:'src/private.rs',children:['private evidence'],firstElementChild:{textContent:'private source or receipt'},dataset:{path:'src/private.rs',designId:'same-design'},checked:true,replaceChildren(){this.children=[];this.textContent='';}});return fields.get(id);};
  const nav=getElementById('layout-architecture'),interpretation=getElementById('lld-interpretation');clearReviewEvidence({getElementById});
  for(const id of ['hld-diagrams','lld-diagrams','lld-inspector','lld-entry-list','lld-history','graph','architecture-diagrams','baseline-revision','proposal-revision','scope-contexts'])assert.deepEqual(getElementById(id).children,[],id);
  for(const id of ['architecture-mermaid-source','lld-mermaid-source','lld-confirmation-preview']){assert.equal(getElementById(id).hidden,true);assert.equal(getElementById(id).firstElementChild.textContent,'');}
  for(const id of ['architecture-source','lld-mermaid','lld-export','lld-confirm','save-scope'])assert.equal(getElementById(id).disabled,true);
  for(const id of ['lld-full-identity','lld-parent-identity','graph-selection','selected-file-name','selected-file-delta','scope-preview','revisions'])assert.equal(getElementById(id).textContent,'',id);
  assert.equal(getElementById('baseline-source-label').textContent,'Baseline');assert.equal(getElementById('proposal-source-label').textContent,'Proposal');
  assert.equal(getElementById('scope-primary-exclude').value,'');assert.equal(getElementById('lld-invariant').value,'');assert.equal(getElementById('lld-known-gaps').checked,false);
  assert.equal(getElementById('toggle-file-reviewed').dataset.path,undefined);assert.equal(nav.textContent,'private path or design');assert.equal(interpretation.textContent,'private path or design','fixed navigation and interpretation options survive');
});

test('scope and revision continuations are canceled by current authority loss even when the transport ignores abort',async()=>{
  for(const pendingPath of ['/v1/prs/99/revisions?limit=100','/v1/reviews/r-1/scope/preview']){
    let finish;const base=request();const value=new ReviewWorkspace({request:async(path,options)=>path===pendingPath?new Promise(resolve=>{finish=resolve;}):path.endsWith('/deny')?Promise.reject(Object.assign(new Error('Denied'),{status:403})):base(path,options)});
    await value.selectReview('r-1');const generation=value.generation;const pending=value.fetch(pendingPath,generation);const rejected=assert.rejects(pending,{name:'AbortError'});
    await assert.rejects(value.fetch('/v1/reviews/r-1/deny',generation),{status:403});finish({items:[{base_commit:'private commit'}],added_files:['src/private.rs'],changes_identity:true});await rejected;
    assert.ok(value.generation>generation);assert.equal(value.state.details,null);assert.equal(value.state.hld,null);assert.equal(value.state.lld,null);
  }
});

// A small DOM implementation that models focus loss on actual child replacement.
function presentationDom(){
  const document={activeElement:null};
  class Element{
    constructor(tag,className='',text=''){this.tagName=tag;this.className=className;this.ownText=text;this.children=[];this.dataset={};this.attributes={};this.events={};this.parentElement=null;this.replacements=0;}
    append(...items){for(const item of items){item.parentElement=this;this.children.push(item);}}
    contains(item){return this===item||this.children.some(child=>child.contains(item));}
    replaceChildren(...items){if(document.activeElement&&this.contains(document.activeElement))document.activeElement=null;this.replacements++;this.children=[];this.append(...items);}
    set textContent(value){this.ownText=value;this.children=[];}
    get textContent(){return this.ownText+this.children.map(item=>item.textContent).join('');}
    setAttribute(key,value){this.attributes[key]=value;}
    getAttribute(key){return this.attributes[key];}
    addEventListener(name,listener){this.events[name]=listener;}
    focus(){document.activeElement=this;}
  }
  const node=(...args)=>new Element(...args);return {document,node};
}

test('unchanged catalog presentation preserves keyboard focus, module folds and button listeners during fresh detail/source reads',()=>{
  const {document,node}=presentationDom(),list=node('div'),status=node('p'),more=node('button'),opened=[];
  const view=new LldCatalogView({list,status,more,node,onOpen:(...args)=>opened.push(args)}),payload=catalog();
  payload.entries.push(...[1,2,3].map(n=>({baseline:null,proposal:{...entry(`next-${n}`),path:`src/module${n}.rs`,name:`next_${n}`},status:'added'})));
  view.render(payload,{selection:'e-proposal'});const group=list.children[0],button=group.children[1],listener=button.events.click;
  button.focus();group.open=false;const replacements=list.replacements;
  view.render(payload,{selection:'e-proposal',loading:true});view.render(payload,{selection:'e-proposal',loading:false});
  assert.equal(list.replacements,replacements);assert.equal(group.children[1],button);assert.equal(document.activeElement,button);assert.equal(group.open,false,'user-collapsed module remains collapsed');assert.equal(more.disabled,false);
  button.events.click();assert.deepEqual(opened[0],['e-proposal',payload.entries[0].proposal]);assert.equal(button.events.click,listener);
  view.render(payload,{selection:'next-1'});assert.equal(list.children[0],group);assert.equal(button.getAttribute('aria-current'),'false');assert.equal(view.buttons.get('next-1').button.getAttribute('aria-current'),'true');assert.equal(view.buttons.get('next-1').group.open,true);
});

test('catalog filters, pagination and current-authority null invalidate reused presentation and release private entries',()=>{
  const {node}=presentationDom(),list=node('div'),status=node('p'),more=node('button'),view=new LldCatalogView({list,status,more,node,onOpen:()=>{}}),payload=catalog();
  view.render(payload);const first=list.children[0];view.render(payload,{query:'unmatched'});assert.equal(list.children.length,0);assert.match(status.textContent,/0 of 1/);
  view.render(payload,{query:''});assert.notEqual(list.children[0],first);const next={...payload,entries:[...payload.entries,{baseline:null,proposal:{...entry('new-page-entry'),path:'src/new.rs',name:'new_function'}}],next_cursor:'more'};
  view.render(next);assert.equal(view.buttons.size,2);assert.equal(more.hidden,false);assert.match(status.textContent,/2 of 2/);
  view.render(null);assert.equal(view.buttons.size,0);assert.equal(list.children.length,0);assert.equal(view.payload,null);assert.equal(more.hidden,true);
  view.render(payload);assert.equal(view.buttons.size,1);view.clear();assert.equal(list.children.length,0);assert.equal(view.buttons.size,0);assert.equal(view.payload,undefined);
  view.render(payload);assert.equal(view.buttons.size,1,'same immutable catalog repopulates after authorization loss');
});

test('unchanged exact-source pair preserves DOM and selection until code, absence or inspector host changes',()=>{
  const {node,document}=presentationDom(),host=node('div'),view=new LldSourcePairView({node}),payload=detail(),comparison={baseline:{...sourceFor('/lld/source?evidence=node-baseline&start_line=2&end_line=4')},proposal:{...sourceFor('/lld/source?evidence=node-proposal&start_line=2&end_line=4')},errors:[]};
  view.render(host,comparison,payload);const baseline=host.children[0],proposal=host.children[1],replacements=host.replacements;proposal.focus();
  view.render(host,comparison,{...payload});assert.equal(host.replacements,replacements);assert.equal(host.children[1],proposal);assert.equal(document.activeElement,proposal);assert.match(host.textContent,/if v > 1/);assert.match(host.textContent,/if v > 2/);
  const changed={...comparison,proposal:{...comparison.proposal,text:'fn newly_authorized() {}'}};view.render(host,changed,payload);assert.notEqual(host.children[0],baseline);assert.match(host.textContent,/newly_authorized/);assert.equal(document.activeElement,null);
  const replacementHost=node('div');view.render(replacementHost,changed,payload);assert.match(replacementHost.textContent,/newly_authorized/);assert.equal(replacementHost.children.length,2,'a recreated inspector is never skipped merely because its source object is unchanged');
  view.render(replacementHost,null,payload);assert.equal(replacementHost.children.length,0);assert.equal(view.comparison,null);
  view.render(replacementHost,{baseline:null,proposal:null,errors:[]},{baseline:null,proposal:{}});assert.match(replacementHost.textContent,/Absent in baseline/);assert.match(replacementHost.textContent,/No retained source/);
  view.clear();assert.equal(replacementHost.children.length,0);assert.equal(view.host,null);assert.equal(view.comparison,undefined,'authority clearing releases retained code references');
});

test('presentation reuse never skips fresh detail authorization and current denial clears reused inventory',async()=>{
  const {node}=presentationDom(),list=node('div'),view=new LldCatalogView({list,status:node('p'),more:node('button'),node,onOpen:()=>{}}),calls=[],base=request();let denied=false;
  const value=new ReviewWorkspace({request:async(path,options)=>{calls.push(path);if(denied&&path.includes('/lld/detail?'))throw Object.assign(new Error('Denied'),{status:403});return base(path,options);},onState:state=>view.render(state.lld,{selection:'e-proposal',loading:state.loading.has('lld')})});
  await value.selectReview('r-1');await value.loadLldCatalog(selection);const button=view.buttons.get('e-proposal').button;
  await value.loadLldDetail('e-proposal');await value.loadLldDetail('e-proposal');assert.equal(view.buttons.get('e-proposal').button,button);assert.equal(calls.filter(path=>path.includes('/lld/detail?')).length,2,'each user reload still calls the authorization boundary');
  denied=true;await value.loadLldDetail('e-proposal');assert.equal(calls.filter(path=>path.includes('/lld/detail?')).length,3);assert.equal(value.state.details,null);assert.equal(view.buttons.size,0);assert.equal(list.children.length,0);
});
