'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const desk=require('../src/workspace-standard.js');
const model=require('../src/workspace-focus-model.js');
const {ReviewWorkspace}=require('../src/workspace.js');

const component=(id,name=id)=>({id,name,kind:'component'});
const evidence={id:'evidence-a',design_id:'design-before',path:'src/owner.rs',commit:'before',blob:'blob-before',start_line:10,end_line:18};
const source={review_id:'review-a',path:evidence.path,commit:evidence.commit,blob_id:evidence.blob,start_line:10,end_line:18,text:'authorized retained text'};

// These are contract fixtures, not content shipped in the review surface.
test('summaries retain supplied uncertainty and never infer changes from presence',()=>{
  const payload={baseline:{elements:[]},proposal:{elements:[component('owner')]},delta:{changes:[{kind:'uncertain',element_id:'owner',reason:'Coverage does not establish equivalence.'}]}};
  const rows=desk.changeRows(payload);
  assert.equal(rows.length,1);assert.equal(rows[0].kind,'uncertain');
  assert.equal(rows[0].reason,payload.delta.changes[0].reason);
  assert.equal(rows[0].elementId,'owner');
  payload.delta.changes=[];
  assert.deepEqual(desk.changeRows(payload),[]);
  assert.deepEqual(desk.changeRows(null),[]);
});

test('relation-only deltas resolve only through their authorized relation identity',()=>{
  const payload={proposal:{elements:[component('caller'),component('callee')],relations:[{id:'edge',from:'caller',to:'callee',kind:'local_package_dependency'}]},delta:{changes:[{kind:'modified',relation_id:'edge',reason:'Declared reference changed.'},{kind:'future_kind',relation_id:'missing'}]}};
  const rows=desk.changeRows(payload);
  assert.equal(rows[0].elementId,'callee');assert.equal(rows[0].relationId,'edge');
  assert.equal(rows[1].kind,'uncertain');assert.equal(rows[1].elementId,null);
  assert.equal(rows[1].reason,'No explanatory evidence supplied.');
});

test('file fallback is factual and does not manufacture architectural intent',()=>{
  const [row]=desk.changeRows(null,[{path:'src/lib.rs',kind:'modified',additions:3,deletions:2}]);
  assert.equal(row.path,'src/lib.rs');assert.equal(row.title,'src/lib.rs');
  assert.match(row.reason,/\+3 \/ −2/);assert.match(row.reason,/not established/);
  const [unknown]=desk.changeRows(null,[{path:'src/unmeasured.rs'}]);
  assert.doesNotMatch(unknown.reason,/\+0|−0/);assert.equal(unknown.kind,'uncertain');
});

test('uncertain contracts remain reachable without broadening the unchanged Focus boundary model',()=>{
  const owner=component('owner'),contract={id:'contract',name:'Public API',kind:'contract',component_id:'owner'};
  const version={elements:[owner,contract],relations:[{id:'edge',from:'owner',to:'contract',kind:'declares_public_contract'}]};
  const payload={baseline:structuredClone(version),proposal:version,delta:{changes:[{kind:'uncertain',element_id:'contract',reason:'Contract comparison is incomplete.'}]}};
  const status=new Map([['contract','uncertain']]);
  assert.ok(!model.boundaries(payload,{status}).some(scope=>scope.target.id==='contract'));
  const scopes=desk.boundaries(payload,status,model),scope=scopes.find(item=>item.target.id==='contract');
  assert.ok(scope);assert.equal(model.state(scope),'uncertain');
  assert.equal(model.description(scope,'combined'),'Contract comparison is incomplete.');
});

test('bounded architecture pages leave every authorized boundary reachable',()=>{
  const origin=component('origin'),targets=Array.from({length:17},(_,index)=>({id:`dependency-${index}`,name:`Dependency ${index}`,kind:'dependency'}));
  const payload={baseline:{elements:[origin],relations:[]},proposal:{elements:[origin,...targets],relations:targets.map(target=>({id:target.id,from:origin.id,to:target.id,kind:'imports_declared_package'}))},delta:{changes:[]}};
  const scopes=desk.boundaries(payload,new Map(),model),seen=new Set();
  for(const scope of scopes){const page=model.view(scopes,scope.key,false);assert.ok(page.scopes.length<=5);assert.ok(page.scopes.includes(scope));page.scopes.forEach(item=>seen.add(item.key));assert.equal(model.state(scope),'uncertain');}
  assert.equal(seen.size,scopes.length);
});

test('LLD mappings pin the exact side and ID, never a name or guessed path',()=>{
  const payload={baseline:{id:'baseline-design',elements:[component('old-owner','Owner'),{id:'old-contract',kind:'contract',component_id:'old-owner'}]},proposal:{id:'proposal-design',elements:[component('new-owner','Owner'),{id:'unmapped',kind:'dependency',roots:['src/owner.rs']}]}};
  const mapping=desk.exactComponent(payload,'old-contract','baseline');
  assert.equal(mapping.version.id,'baseline-design');assert.equal(mapping.component.id,'old-owner');
  assert.equal(desk.exactComponent(payload,'old-contract','proposal'),null);
  assert.equal(desk.exactComponent(payload,'unmapped','proposal'),null);
  assert.equal(desk.exactComponent(payload,'Owner','proposal'),null);
  assert.equal(desk.exactComponent(null,'old-owner'),null);
});

test('source acceptance binds review, commit, path, blob and bounded exact range',()=>{
  assert.equal(desk.sourceMatches(source,evidence,'review-a'),true);
  assert.equal(desk.sourceMatches({...source,end_line:15},evidence,'review-a'),true);
  for(const altered of [{review_id:'review-b'},{commit:'newer-head'},{path:'src/other.rs'},{blob_id:'another-blob'},{start_line:9},{end_line:19},{end_line:9},{end_line:10.5},{text:null}])assert.equal(desk.sourceMatches({...source,...altered},evidence,'review-a'),false,JSON.stringify(altered));
  assert.equal(desk.sourceMatches(null,evidence,'review-a'),false);
});

test('per-review positions are independent and clear with the authorization context',()=>{
  const positions=new desk.Positions();
  positions.save('review-a',{section:'source',boundary:'owner\0contract',sourceId:'design-before:evidence-a',sourceReturn:'lld',changePage:2});
  positions.save('review-b',{section:'hld',boundary:'another\0boundary'});
  assert.equal(positions.get('review-a').sourceReturn,'lld');
  assert.equal(positions.get('review-b').section,'hld');
  const detached=positions.get('review-a');detached.section='changes';
  assert.equal(positions.get('review-a').section,'source');
  assert.equal(positions.get('never-opened').section,'changes');
  positions.clear();assert.equal(positions.get('review-a').section,'changes');
});

test('an obsolete source authorization failure cannot clear the newly selected review',async()=>{
  let rejectRequest;
  const workspace=new ReviewWorkspace({request:()=>new Promise((resolve,reject)=>{rejectRequest=reject;})});
  workspace.state.reviewId='review-a';workspace.state.details={proposal:'before'};
  const old=workspace.fetch('/v1/reviews/review-a/hld/source?design_id=old',workspace.generation);
  workspace.generation++;workspace.state.reviewId='review-b';workspace.state.details={proposal:'current'};
  const denied=new Error('Denied');denied.status=403;rejectRequest(denied);
  await assert.rejects(old,/Denied/);
  assert.equal(workspace.state.reviewId,'review-b');assert.equal(workspace.state.details.proposal,'current');
});

test('current exact-source authorization loss clears all review evidence',async()=>{
  const denied=new Error('Denied');denied.status=403;
  const workspace=new ReviewWorkspace({request:async()=>{throw denied;}});
  workspace.state.reviewId='review-a';workspace.state.details={proposal:'before'};workspace.state.hld={proposal:{id:'design-before'}};workspace.state.lldSource=source;
  await assert.rejects(workspace.fetch('/v1/reviews/review-a/hld/source?design_id=old',workspace.generation),/Denied/);
  assert.equal(workspace.state.details,null);assert.equal(workspace.state.hld,null);assert.equal(workspace.state.lldSource,null);
});
