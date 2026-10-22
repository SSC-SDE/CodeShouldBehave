'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const {ReviewWorkspace,hldOutcome,hldExportAllowed,hldComparison,hldDiagramElements,hldMermaid,hldEvidenceFor,hldCorrection,encodeWorkspaceRoute,decodeWorkspaceRoute}=require('../src/workspace.js');

function version(id,commit,name='API') {
  return {id,snapshot:commit,scope_version:'scope-1',generator_version:'hld-v1',review_state:'partial',coverage:{complete:false,gaps:['Runtime behavior is unverified']},elements:[
    {id:'z-store',kind:'dependency',name:'Storage client library',roots:['src/store'],facts:[],evidence_ids:[],unknowns:['Data ownership unknown']},
    {id:'a-api',kind:'component',name,roots:['src/api'],responsibility:'Handles API requests',facts:[{kind:'import',value:'storage client',resolution:'observed_structure',evidence_ids:['e-api']}],evidence_ids:['e-api'],unknowns:[]},
  ],relations:[{id:'rel-store',from:'a-api',to:'z-store',kind:'dependency',resolution:'observed_structure',protocol:null,synchrony:'unknown',evidence_ids:['e-api']}],evidence:[{id:'e-api',commit,path:'src/api/mod.rs',blob:`blob-${id}`,start_line:1,end_line:4,relationship_type:'import',resolution:'observed_structure'}]};
}
function payload() {
  return {review_id:'r-1',can_accept:true,can_export:false,baseline:version('d-base','a'.repeat(40)),proposal:version('d-proposal','b'.repeat(40)),accepted:version('d-accepted','c'.repeat(40),'Accepted API'),delta:{id:'delta-1',outcome:'change_detected',changes:[{id:'change-api',kind:'modified',element_id:'a-api',reason:'API contract changed'},{id:'change-rel',kind:'added',relation_id:'rel-store',reason:'Dependency introduced'}],possible_impacts:[{component_id:'z-store',via_relation_ids:['rel-store'],reason:'Selected consumer may be affected'}],coverage:{complete:false,gaps:['Runtime behavior is unverified']}},accepted_drift:{outcome:'incomplete'}};
}
function requestForHld(extra={}) {
  return async (path,options={})=>{
    if(path.includes('/hld'))return extra.hld ? extra.hld(path,options) : payload();
    if(path.includes('/graph?'))return {kind:'children',paths:[]};
    if(path.endsWith('/progress'))return {review_id:'r-1'};
    if(path.includes('/files?')||path.includes('/findings?')||path.endsWith('/annotations'))return {review_id:'r-1',items:[]};
    return {review_id:path.includes('r-2')?'r-2':'r-1',proposal:'b'.repeat(40)};
  };
}
async function workspaceWithHld(extra) {const workspace=new ReviewWorkspace({request:requestForHld(extra)});await workspace.selectReview('r-1');await workspace.loadHld();return workspace;}

test('HLD outcome never upgrades incomplete coverage to no impact',()=>{
  assert.equal(hldOutcome(null),'Unable to determine completely');
  assert.equal(hldOutcome({outcome:'incomplete'}),'Unable to determine completely');
  assert.equal(hldOutcome({outcome:'no_change_detected_within_scope'}),'No HLD change detected within covered scope');
  assert.equal(hldOutcome({outcome:'change_detected'}),'HLD change detected');
});

test('HLD comparison uses stable logical IDs and server semantic classifications',()=>{
  const data=payload();data.proposal.elements.reverse();const comparison=hldComparison(data);
  assert.deepEqual(comparison.elements.map(item=>item.id),['a-api','z-store']);
  assert.equal(comparison.status.get('a-api'),'modified');
  data.delta.changes=[];data.proposal.elements[0].name='Same logical component in a different layout';
  assert.equal(hldComparison(data).status.size,0);
  assert.equal(hldComparison(data,'accepted').elements[0].name,'Accepted API');
});

test('HLD diagrams show evidence direction and uncertainty without inventing runtime guarantees',()=>{
  const data=payload();data.proposal.relations[0].resolution='unknown';const mermaid=hldMermaid(data);
  assert.match(mermaid,/hld0 -\.->\|\+ \? dependency\| hld1/);
  assert.match(mermaid,/hld0\["API/);assert.match(mermaid,/hld1\["Storage client library/);
  assert.doesNotMatch(mermaid,/production|guaranteed|low latency|database service/);
  assert.match(mermaid,/hld1\[.*:::impacted/);
});

test('HLD repository labels cannot inject Mermaid statements or executable HTML',()=>{
  const data=payload();data.proposal.elements.find(item=>item.id==='a-api').name='x"]\nclick evil "javascript:alert(1)"\n<script>alert(1)</script>|&';
  const mermaid=hldMermaid(data);
  assert.doesNotMatch(mermaid,/<script>|javascript:[^\n]*"\nclick|\nclick /);
  const source=hldComparison(data).elements.find(item=>item.id==='a-api');
  assert.ok(source.name.includes('<script>'),'immutable input is preserved while rendered label is sanitized');
});

test('HLD diagram bounds loaded elements and drops relations outside the bounded map',()=>{
  const data=payload();data.proposal.elements=Array.from({length:130},(_,index)=>({id:`x-${String(index).padStart(3,'0')}`,kind:'component',name:`Component ${index}`,roots:[],facts:[],evidence_ids:[]}));
  data.baseline.elements=[];data.baseline.relations=[];data.proposal.relations=[{id:'outside',from:'x-000',to:'x-129',kind:'dependency',resolution:'observed_structure'}];
  assert.equal(hldComparison(data).elements.length,100);assert.equal(hldComparison(data).omitted,30);
  assert.doesNotMatch(hldMermaid(data),/Component 129|hld0 -->/);
});

test('accepted HLD evidence keeps its original design and source commit',()=>{
  const data=payload();const evidence=hldEvidenceFor(data,'a-api','accepted');
  assert.equal(evidence.length,1);assert.equal(evidence[0].design_id,'d-accepted');assert.equal(evidence[0].commit,'c'.repeat(40));
  const paired=hldEvidenceFor(data,'a-api');assert.equal(paired.length,2);assert.deepEqual(paired.map(item=>item.commit),['a'.repeat(40),'b'.repeat(40)]);
  assert.deepEqual(hldEvidenceFor(data,'missing'),[]);
});

test('HLD is loaded lazily and never adds a scan or acceptance to PR selection',async()=>{
  const paths=[];const base=requestForHld();const workspace=new ReviewWorkspace({request:async(path,options)=>{paths.push([path,options?.method||'GET']);return base(path,options);}});
  await workspace.selectReview('r-1');assert.ok(paths.every(([path])=>!path.includes('/hld')));
  await workspace.loadHld();assert.equal(workspace.state.hld.review_id,'r-1');assert.deepEqual(paths.filter(([path])=>path.includes('/hld')).map(item=>item[1]),['GET']);
});

test('obsolete HLD responses cannot replace another selected PR',async()=>{
  let finish;const workspace=new ReviewWorkspace({request:requestForHld({hld:()=>new Promise(resolve=>{finish=resolve;})})});
  await workspace.selectReview('r-1');const load=workspace.loadHld();await workspace.selectReview('r-2');finish(payload());await load;
  assert.equal(workspace.state.reviewId,'r-2');assert.equal(workspace.state.hld,null);assert.equal(workspace.state.hldError,null);
});

test('mismatched HLD review identity is denied without replacing structural review',async()=>{
  const workspace=await workspaceWithHld({hld:async()=>({...payload(),review_id:'r-other'})});
  assert.equal(workspace.state.hld,null);assert.match(workspace.state.hldError,/does not match/);assert.equal(workspace.state.details.review_id,'r-1');
});

test('missing optional HLD clears design evidence while preserving usable code review',async()=>{
  const workspace=await workspaceWithHld({hld:async()=>{throw Object.assign(new Error('Not generated'),{status:404});}});
  assert.equal(workspace.state.hld,null);assert.equal(workspace.state.details.review_id,'r-1');assert.match(workspace.state.hldError,/Not generated/);
});

test('revoked HLD access clears cached design and code evidence',async()=>{
  let deny=false;const workspace=await workspaceWithHld({hld:async()=>{if(deny)throw Object.assign(new Error('Revoked'),{status:403});return payload();}});
  workspace.state.hldPreview={request:'draft',response:{preview_id:'private'}};deny=true;await workspace.loadHld();
  assert.equal(workspace.state.hld,null);assert.equal(workspace.state.details,null);assert.equal(workspace.state.graph,null);assert.equal(workspace.state.hldPreview,null);
});

test('HLD preview requires maintainer capability and explicit confirmation',async()=>{
  let writes=0;const workspace=await workspaceWithHld({hld:async(path,options)=>{if(options.method)writes++;return {...payload(),can_accept:false};}});
  await assert.rejects(workspace.previewHld({design_id:'d-base'}),/maintainer/);
  await assert.rejects(workspace.acceptHld({design_id:'d-base'}),/Preview/);assert.equal(writes,0);
});

test('HLD draft updates preserve optimistic pins and default activation stays explicit',async()=>{
  const writes=[];let revision=0;const workspace=await workspaceWithHld({hld:async(path,options)=>{
    if(path.endsWith('/preview')){const body=JSON.parse(options.body);writes.push(body);return {review_id:'r-1',preview_id:'private-draft',expected_revision:++revision,input_fingerprint:`fp-${revision}`,review:{state:'awaiting_confirmation'}};}
    if(path.endsWith('/accept')){writes.push(JSON.parse(options.body));return {review_id:'r-1',accepted:true};}
    return payload();
  }});
  const initial={design_id:'d-base',interpretation:'intended',corrections:[]};await workspace.previewHld(initial);
  assert.equal(writes[0].expected_revision,0);assert.equal(writes[0].preview_id,undefined);
  const updated={...initial,corrections:[{id:'c1',kind:'responsibility',target_id:'a-api',value:'Intended responsibility',paths:[],related_ids:[],constraint_kind:null}]};
  workspace.invalidateHldPreview();await assert.rejects(workspace.acceptHld(updated),/Preview/);await workspace.previewHld(updated);
  assert.equal(writes[1].expected_revision,1);assert.equal(writes[1].preview_id,'private-draft');
  await workspace.acceptHld(updated,{knownGapsAccepted:true});
  assert.equal(writes[2].expected_revision,2);assert.equal(writes[2].input_fingerprint,'fp-2');assert.equal(writes[2].known_gaps_accepted,true);assert.equal(writes[2].activate_default,false);
  assert.equal(workspace.state.hldPreview,null);
});

test('changed correction bodies cannot reuse approval previews',async()=>{
  let accepts=0;const workspace=await workspaceWithHld({hld:async(path)=>path.endsWith('/preview')?{review_id:'r-1',preview_id:'draft',expected_revision:1,input_fingerprint:'fp'}:path.endsWith('/accept')?(accepts++,{review_id:'r-1'}):payload()});
  const body={design_id:'d-base',interpretation:'as_built',corrections:[]};await workspace.previewHld(body);
  await assert.rejects(workspace.acceptHld({...body,interpretation:'intended'}),/Preview/);assert.equal(accepts,0);
});

test('stale HLD preview and invalid server pins block acceptance',async()=>{
  const workspace=await workspaceWithHld({hld:async(path)=>{if(path.endsWith('/preview'))throw Object.assign(new Error('Design changed'),{status:409});return payload();}});
  await assert.rejects(workspace.previewHld({design_id:'d-base'}),/Design changed/);assert.equal(workspace.state.hldPreview,null);
  await assert.rejects(workspace.acceptHld({design_id:'d-base'}),/Preview/);
  workspace.request=requestForHld({hld:async()=>({review_id:'r-1',preview_id:'draft',input_fingerprint:'fp'})});
  await assert.rejects(workspace.previewHld({design_id:'d-base'}),/does not match/);
});

test('typed HLD corrections cannot broaden boundaries or refer to unrelated elements',()=>{
  const design=version('d','a');const base={id:'c1',kind:'boundary',targetId:'a-api',value:'Narrow responsibility',paths:['src/api/mod.rs'],relatedIds:[]};
  assert.deepEqual(hldCorrection(base,design).paths,['src/api/mod.rs']);
  assert.throws(()=>hldCorrection({...base,paths:['src/private']},design),/only narrow/);
  assert.throws(()=>hldCorrection({...base,kind:'contract_ownership',relatedIds:['outside']},design),/belong/);
  assert.throws(()=>hldCorrection({...base,kind:'constraint',relatedIds:[],constraintKind:'prohibited_dependency'},design),/one related/);
  assert.equal(hldCorrection({...base,kind:'constraint',relatedIds:['a-api'],constraintKind:'prohibited_dependency'},design).constraint_kind,'prohibited_dependency');
  assert.throws(()=>hldCorrection({...base,kind:'responsibility',value:'x'.repeat(2049)},design),/2048/);
});

test('HLD deep links preserve PR scope identity and exact comparison without changing existing routes',()=>{
  const route={reviewId:'r-1',layout:'architecture',side:'diff',selection:null,designLevel:'hld',designMode:'accepted',designElement:'a-api'};
  const hash=encodeWorkspaceRoute(route);assert.deepEqual(decodeWorkspaceRoute(hash),route);assert.match(hash,/review=r-1/);
  assert.equal(encodeWorkspaceRoute({reviewId:'r-1',layout:'architecture'}),'#review=r-1&layout=architecture&side=diff');
  assert.equal(decodeWorkspaceRoute('#review=r-1&design=hld&design_view=invalid'),null);
});

test('corrections edited during an in-flight preview cannot become accepted by the late response',async()=>{
  let finish;const workspace=await workspaceWithHld({hld:async(path)=>path.endsWith('/preview')?new Promise(resolve=>{finish=resolve;}):payload()});
  const body={design_id:'d-base',interpretation:'as_built',corrections:[]};const preview=workspace.previewHld(body);
  body.corrections.push({id:'late-correction',value:'not yet previewed'});workspace.invalidateHldPreview();
  finish({review_id:'r-1',preview_id:'draft',expected_revision:1,input_fingerprint:'fp'});
  await assert.rejects(preview,/changed while/);assert.equal(workspace.state.hldPreview,null);
  assert.equal(workspace.state.hldDraft.expected_revision,1,'draft revision remains usable for a new explicit preview');
  await assert.rejects(workspace.acceptHld(body),/Preview/);
});

test('revocation cancels parallel previously authorized responses before they can restore evidence',async()=>{
  const finishes=[];const workspace=new ReviewWorkspace({request:async(path)=>{
    if(path.endsWith('/annotations'))throw Object.assign(new Error('Access revoked'),{status:403});
    if(path.includes('/graph?'))return new Promise(resolve=>{finishes.push(resolve);});
    return {review_id:'r-1',items:[]};
  }});
  const selected=workspace.selectReview('r-1');await Promise.resolve();await Promise.resolve();
  assert.equal(workspace.state.details,null);assert.equal(workspace.state.hld,null);
  // Both graph requests are completed independently; their results remain obsolete.
  workspace.request=requestForHld();await workspace.selectReview('r-2');for(const finish of finishes)finish({kind:'children',paths:[['private','component']]});await selected;
  assert.equal(workspace.state.reviewId,'r-2');assert.equal(workspace.state.details.review_id,'r-2');assert.deepEqual(workspace.state.graph.paths,[]);
});

test('explicit HLD correspondence links the exact other snapshot with matching cardinality and kinds',()=>{
  const current=version('new-design','b'),source=version('old-design','a');current.elements.find(item=>item.id==='a-api').id='new-api';source.elements.find(item=>item.id==='a-api').id='old-api';
  const body={id:'correspondence-1',kind:'correspondence',targetId:'new-api',value:'move',relatedIds:['old-api'],referenceDesignId:'old-design',correspondenceTargets:['new-api']};
  const correction=hldCorrection(body,current,source);assert.equal(correction.reference_design_id,'old-design');assert.deepEqual(correction.correspondence_targets,['new-api']);
  assert.throws(()=>hldCorrection(body,current,null),/exact generated design/);
  assert.throws(()=>hldCorrection({...body,relatedIds:['z-store']},current,source),/same architectural kind/);
  assert.throws(()=>hldCorrection({...body,value:'split'},current,source),/matching multiple/);
  current.elements.push({...current.elements.find(item=>item.id==='new-api'),id:'new-api-two'});
  assert.deepEqual(hldCorrection({...body,value:'split',correspondenceTargets:['new-api','new-api-two']},current,source).correspondence_targets,['new-api','new-api-two']);
});


test('HLD export checks the selected immutable design policy as well as current policy',()=>{
  const data={can_export:true,export_views:{baseline:true,proposal:true,accepted:false}};
  assert.equal(hldExportAllowed(data,'combined'),true);assert.equal(hldExportAllowed(data,'compare'),true);
  assert.equal(hldExportAllowed(data,'accepted'),false);assert.equal(hldExportAllowed(data,'baseline'),true);
  assert.equal(hldExportAllowed({...data,can_export:false},'proposal'),false);
  assert.equal(hldExportAllowed({can_export:true},'baseline'),false);
  assert.equal(hldExportAllowed({...data,export_views:{baseline:true,proposal:false}},'combined'),false);
});


test('large HLD inventories keep component diagrams above public declaration details',()=>{
  const data=payload();for(const design of [data.baseline,data.proposal])design.elements.push(...Array.from({length:150},(_,index)=>({id:`0-contract-${index}`,kind:'contract',name:`public_function_${index}`,component_id:'a-api',roots:[],facts:[],evidence_ids:[]})));
  data.delta.changes=[{kind:'modified',element_id:'0-contract-149',reason:'Public signature changed'}];
  const comparison=hldComparison(data);assert.equal(comparison.elements[0].id,'a-api');assert.equal(comparison.elements[1].id,'z-store');assert.equal(comparison.omitted,52);
  assert.deepEqual(hldDiagramElements(data).map(item=>item.id),['a-api','z-store']);assert.equal(comparison.status.get('a-api'),'modified');
  assert.doesNotMatch(hldMermaid(data),/public_function_/);assert.match(hldMermaid(data),/hld0\[.*:::modified/);
});

test('LLD drill-down links preserve the exact HLD parent, component and PR without fabricating detail',()=>{
  const route={reviewId:'r-1',layout:'architecture',side:'diff',selection:null,designLevel:'lld',designMode:'proposal',designElement:'a-api',parentHld:'design-proposal',component:'a-api',detail:'module',lldElement:null};
  assert.deepEqual(decodeWorkspaceRoute(encodeWorkspaceRoute(route)),route);
  assert.throws(()=>encodeWorkspaceRoute({...route,component:'other'}),/mapping/);
  assert.equal(decodeWorkspaceRoute('#review=r-1&design=lld&element=a&component=b'),null);
});

test('uncertain contract and relation deltas remain unknown at the owning HLD component',()=>{
  const data=payload();for(const design of [data.baseline,data.proposal])design.elements.push({id:'contract-1',kind:'contract',name:'handle',component_id:'a-api',facts:[],evidence_ids:[]});
  data.delta.changes=[{kind:'uncertain',element_id:'contract-1',reason:'Source parse failed'},{kind:'uncertain',relation_id:'rel-store',reason:'Incomplete resolution'}];
  assert.equal(hldComparison(data).status.get('a-api'),'uncertain');assert.match(hldMermaid(data),/hld0\[.*:::unknown/);assert.match(hldMermaid(data),/-\.->\|\? dependency/);
  data.delta.changes.push({kind:'modified',element_id:'a-api',reason:'Known boundary change'});assert.equal(hldComparison(data).status.get('a-api'),'modified');
});

test('design documents are explicit pinned intent selections and cannot become source facts',()=>{
  const design=version('d-1','a'),document={id:'doc-1',title:'Design intent',evidence_id:'doc-evidence'};design.document_candidates=[document];
  design.evidence.push({id:'doc-evidence',commit:'a',path:'docs/design.md',blob:'doc-blob',start_line:1,end_line:2});
  const correction=hldCorrection({id:'c-doc',kind:'document_reference',targetId:'a-api',value:'Reviewer confirmed the documented intent requires verification',documentEvidenceId:'doc-evidence'},design);
  assert.equal(correction.document_evidence_id,'doc-evidence');assert.deepEqual(correction.related_ids,[]);assert.equal(correction.reference_design_id,null);
  assert.throws(()=>hldCorrection({...correction,id:'bad',kind:'document_reference',targetId:'a-api',documentEvidenceId:'other',value:'Intent'},design),/exact generated snapshot/);
  design.decision_records=[{component_id:'a-api',evidence_ids:['doc-evidence'],resolution:'user_assertion',verification:'requires_reviewer_verification'}];
  assert.ok(hldEvidenceFor({baseline:design},'a-api','baseline').some(item=>item.path==='docs/design.md'));
  assert.equal(design.elements.find(item=>item.id==='a-api').facts.length,1);
});

test('HLD parallel package evidence is concise without merging uncertainty or mutating evidence',()=>{
  const data=payload();data.delta.changes=[];
  data.proposal.relations=[
    {id:'a',from:'a-api',to:'z-store',kind:'declares_package_dependency',resolution:'observed_structure'},
    {id:'b',from:'a-api',to:'z-store',kind:'imports_declared_package',resolution:'observed_structure'},
    {id:'c',from:'a-api',to:'z-store',kind:'imports_declared_package',resolution:'unknown'}
  ];
  const before=JSON.stringify(data),source=hldMermaid(data,'proposal');
  assert.match(source,/declares \+ imports/);
  assert.match(source,/\? imports/);
  assert.match(source,/Code component/);assert.match(source,/Package dependency/);
  assert.equal((source.match(/\| hld/g)||[]).length,2);
  assert.equal(JSON.stringify(data),before);
});
