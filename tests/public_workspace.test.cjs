'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const {createHash,webcrypto}=require('node:crypto');
const {PublicStaticTransport,PublicWorkspaceError}=require('../src/public-workspace.js');
const {ReviewWorkspace,encodeWorkspaceRoute,decodeWorkspaceRoute,resolvePublicWorkspaceRoute}=require('../src/workspace.js');
const sha=value=>createHash('sha256').update(value).digest('hex');
const canonical=value=>Array.isArray(value)?value.map(canonical):value&&typeof value==='object'?Object.fromEntries(Object.keys(value).sort().map(key=>[key,canonical(value[key])])):value;
const json=value=>JSON.stringify(canonical(value));
const clone=value=>JSON.parse(JSON.stringify(value));
const tick=()=>new Promise(resolve=>setImmediate(resolve));
function fixture({mutate,second=false}={}) {
  const baseline='a'.repeat(40),proposal='b'.repeat(40),analysis='c'.repeat(64),scope='scope-1',path='src/api.rs';
  const source=side=>({evidence_id:`source-${side}`,commit:side==='baseline'?baseline:proposal,path,blob_id:(side==='baseline'?'1':'2').repeat(40),start_line:1,end_line:3,text:`fn handle() {\n  return ${side==='baseline'?1:2};\n}`,truncated:false});
  const evidence=side=>{const s=source(side);return {id:s.evidence_id,commit:s.commit,path,blob:s.blob_id,start_line:1,end_line:3};};
  const entry=side=>({id:`entry-${side}`,logical_key:'crate::api::handle',name:'handle',kind:'function',module:'crate::api',container:null,parent_entry_id:null,child_entry_ids:[],path,blob:source(side).blob_id,start_byte:0,end_byte:30,start_line:1,end_line:3,visibility:'private',signature:'fn handle()',signature_fingerprint:'signature',body_fingerprint:side,projections:[],detail_state:'published',evidence_ids:[`source-${side}`],parent:{hld_design_id:`hld-${side}`,component_id:'api',candidate_component_ids:[],state:'matched'},limitations:[]});
  const catalog=side=>({id:`catalog-${side}`,snapshot:side==='baseline'?baseline:proposal,scope_version:scope,hld_design_id:`hld-${side}`,entries:[entry(side)],evidence:[evidence(side)],coverage:{indexed:1,inventory_complete:true,structure_complete:true,gaps:[]}});
  const detail=side=>({id:`detail-${side}`,catalog_id:`catalog-${side}`,entry:entry(side),declarations:[],label:'Static local control flow',coverage:{indexed:1,inventory_complete:true,structure_complete:true},graph:{nodes:[{id:`node-${side}`,operation:'return',label:'return literal',evidence_id:`source-${side}`,resolution:'syntactic'}],edges:[],regions:[],complete:true},evidence:[evidence(side)],mermaid:'flowchart TD\nA[return]',markdown:'# Static local control flow\n'});
  const design=side=>({id:`hld-${side}`,snapshot:side==='baseline'?baseline:proposal,scope_version:scope,elements:[{id:'api',kind:'component',name:'src',roots:['src'],facts:[],unknowns:[]}],relations:[],sections:[],coverage:{complete:true},evidence:[evidence(side)],mermaid:'flowchart TD\nA[src]',markdown:'# Architecture\n'});
  const chunks={
    'artifact.json':{artifact_version:'csb-public-artifact-v1',analysis_review_id:analysis,selected_baseline:baseline,baseline,proposal,scope_version:scope,scope_manifest:{primary_repository:{provider:'github',owner:'SSC-SDE',name:'codeshouldbehave'},include:['src'],exclude:['src/private'],contexts:[],export_allowed:true},files:[{path,old_path:null,kind:'modified',additions:1,deletions:1}],architecture:{nodes:[{path,status:'modified',blobBefore:source('baseline').blob_id,blobAfter:source('proposal').blob_id,definitionsBefore:['handle'],definitionsAfter:['handle'],callsBefore:[],callsAfter:[],sourceBefore:null,sourceAfter:null}],edges:[]},warnings:[],coverage:{indexed:1,unsupported:0,excluded:0,failed:0,deferred:0,enumeration_complete:true},hld_status:'changed',lld_status:'changed'},
    'hld.json':{baseline:design('baseline'),proposal:design('proposal'),delta:{outcome:'change_detected',changes:[]},export_views:{baseline:'# Before',proposal:'# After',diff:'# Diff'}},
    'lld/catalog.json':{baseline:catalog('baseline'),proposal:catalog('proposal'),entries:[{baseline:entry('baseline'),proposal:entry('proposal'),status:'modified'}],delta:{outcome:'change_detected',changes:[]}},
    'lld/detail/one.json':{entry_id:'entry-proposal',baseline:detail('baseline'),proposal:detail('proposal'),delta:{outcome:'change_detected',changes:[]}},
    'evidence/baseline.json':source('baseline'),'evidence/proposal.json':source('proposal'),
    'exports/review.md':'# Public architecture review\n'
  };
  const routes={artifact:'artifact.json',hld:'hld.json',lld_catalog:'lld/catalog.json',lld_details:[],sources:[],file_sources:[],exports:[{view:'diff',path:'exports/review.md'}]};
  for(const parent_hld of ['hld-baseline','hld-proposal'])for(const component of [null,'api'])for(const entry of ['entry-baseline','entry-proposal']){const pin={entry,parent_hld,component,baseline_entry:null,correspondence_kind:null};routes.lld_details.push({...pin,path:'lld/detail/one.json'});for(const side of ['baseline','proposal'])routes.sources.push({...pin,evidence:`source-${side}`,path:`evidence/${side}.json`});}
  for(const side of ['baseline','proposal']){routes.sources.push({entry:null,parent_hld:`hld-${side}`,component:null,baseline_entry:null,correspondence_kind:null,evidence:`source-${side}`,path:`evidence/${side}.json`});routes.file_sources.push({file_path:path,side,parent_hld:`hld-${side}`,path:`evidence/${side}.json`});}
  const manifest={schema_version:'csb-public-report-v1',review_id:'',public_report_id:'',analysis_review_id:analysis,repository:{id:1,owner:'SSC-SDE',name:'codeshouldbehave'},pr_number:99,selected_baseline:baseline,baseline,proposal,scope_version:scope,policy_digest:'d'.repeat(64),engine:{package:'codeshouldbehave',version:'0.1.0',hld_generator:'3',lld_generator:'1',lld_parser:'1',public_projection:'1'},files:[],routes,coverage:{complete:true,gaps:[],deferred_details:0},capabilities:{hld:true,lld:true,source:true,export:true,confirmation:false}};
  mutate?.({chunks,manifest});
  const files=new Map();for(const [path,value] of Object.entries(chunks))files.set(path,typeof value==='string'?value:json(value));
  manifest.files=[...files].sort(([a],[b])=>a.localeCompare(b)).map(([path,value])=>({path,sha256:sha(value),bytes:Buffer.byteLength(value),kind:path.endsWith('.md')?'markdown':'json'}));
  const identity={...manifest};delete identity.review_id;delete identity.public_report_id;const id=sha(json(identity));manifest.review_id=id;manifest.public_report_id=id;
  const manifestText=json(manifest),record={public_report_id:id,analysis_review_id:analysis,repository:clone(manifest.repository),pr_number:99,selected_baseline:baseline,baseline,proposal,scope_version:scope,manifest:`reviews/${id}/manifest.json`,manifest_sha256:sha(manifestText),manifest_bytes:Buffer.byteLength(manifestText)};
  const index={schema_version:'csb-public-index-v1',generated_at:'2026-10-17T00:00:00Z',reports:[record],expired:[]};
  if(second)index.reports.push({...record,public_report_id:'e'.repeat(64),manifest:`reviews/${'e'.repeat(64)}/manifest.json`});
  const resources=new Map([['https://example.test/project/index.json',json(index)],[`https://example.test/project/${record.manifest}`,manifestText],...[...files].map(([path,text])=>[`https://example.test/project/reviews/${id}/${path}`,text])]);
  const calls=[],fetcher=async(url,options)=>{calls.push({url,options});if(options.signal?.aborted)throw new DOMException('Aborted','AbortError');return new Response(resources.get(url)||'missing',{status:resources.has(url)?200:404});};
  const transport=new PublicStaticTransport({indexURL:'https://example.test/project/index.json',fetch:fetcher,crypto:webcrypto});
  return {transport,resources,calls,manifest,index,id,analysis,path,record,chunks,base:`/v1/reviews/${id}`};
}

test('published DTOs drive the existing controller with verified project-relative, credential-free reads',async()=>{
  const f=fixture(),workspace=new ReviewWorkspace({request:(...args)=>f.transport.request(...args)});await workspace.selectReview(f.id);
  assert.equal(workspace.state.error,null);assert.equal(workspace.state.details.profile,'public-ci-v1');assert.deepEqual(workspace.state.files.items.map(file=>[file.path,file.additions,file.deletions]),[['src/api.rs',1,1]]);
  await workspace.loadHld();assert.equal(workspace.state.hld.review_id,f.id);assert.equal(workspace.state.hld.can_accept,false);assert.equal(workspace.state.hld.markdown_export,false);
  assert.equal(f.transport.assetURL('mermaid.js'),'https://example.test/project/assets/mermaid.js');
  for(const call of f.calls){assert.ok(call.url.startsWith('https://example.test/project/'));assert.equal(call.options.credentials,'omit');assert.equal(call.options.redirect,'error');assert.equal(call.options.cache,'no-store');assert.ok(!('Authorization' in call.options.headers));}
});

test('site publication time never becomes an invented PR observation time or provider state',async()=>{
  const f=fixture(),page=await f.transport.request('/v1/prs?limit=100');assert.equal(page.items[0].state,'unknown');assert.equal(page.items[0].observed_at_ms,0);const record=f.transport.index.reports[0];record.observed_at='2026-10-17T12:00:00Z';record.state='draft';const observed=f.transport.revision(record);assert.equal(observed.state,'draft');assert.equal(observed.observed_at_ms,Date.parse(record.observed_at));
});

test('canonical deep links bind domain review and public bundle without selecting a second projection',async()=>{
  const f=fixture({second:true});await f.transport.bootstrap();assert.throws(()=>f.transport.resolveRoute({reviewId:f.analysis}),/ambiguous/);
  const route={reviewId:f.analysis,bundleId:f.id,layout:'architecture',side:'diff',designLevel:'lld',parentHld:'hld-proposal',component:'api',lldElement:'entry-proposal',lldNode:'node-proposal',lldPinned:true};
  const decoded=decodeWorkspaceRoute(encodeWorkspaceRoute(route));assert.equal(decoded.bundleId,f.id);assert.equal(f.transport.resolveRoute(decoded).reviewId,f.id);assert.deepEqual(f.transport.routeFor(f.id),{reviewId:f.analysis,bundleId:f.id});
  assert.throws(()=>f.transport.resolveRoute({...route,reviewId:'f'.repeat(64)}),/unavailable/);assert.equal(decodeWorkspaceRoute('#review=x&bundle=wrong'),null);
  assert.equal(decodeWorkspaceRoute(encodeWorkspaceRoute({reviewId:'native'})).bundleId,undefined,'native routes are unchanged');
});

test('invalid and duplicate public hash pins cannot retain a prior selected comparison',async()=>{
  const f=fixture();await f.transport.bootstrap();const valid=`#review=${f.analysis}&bundle=${f.id}`;assert.equal(resolvePublicWorkspaceRoute(f.transport,valid).reviewId,f.id);
  for(const hash of [`#review=${f.analysis}&bundle=wrong`,valid+'&bundle='+f.id,valid+'&review='+f.analysis,'#bundle='+f.id,'#review='])assert.throws(()=>resolvePublicWorkspaceRoute(f.transport,hash),/link is invalid/);
  assert.equal(resolvePublicWorkspaceRoute(f.transport,''),null);assert.equal(resolvePublicWorkspaceRoute(f.transport,'#review-content'),null);
});

test('hash or canonical identity failure clears current evidence and never falls back to native HTTP',async()=>{
  const f=fixture();f.resources.set(`https://example.test/project/reviews/${f.id}/artifact.json`,json({...f.chunks['artifact.json'],files:[]}));
  const workspace=new ReviewWorkspace({request:(...args)=>f.transport.request(...args)});await workspace.selectReview(f.id);assert.equal(workspace.state.details,null);assert.equal(workspace.state.graph,null);assert.match(workspace.state.error,/integrity/);assert.ok(f.calls.every(call=>!call.url.includes('/v1/')));
  const other=fixture();const bad=clone(other.manifest);bad.coverage.complete=false;const text=json(bad);other.resources.set(`https://example.test/project/${other.record.manifest}`,text);other.index.reports[0].manifest_sha256=sha(text);other.index.reports[0].manifest_bytes=Buffer.byteLength(text);other.resources.set('https://example.test/project/index.json',json(other.index));await assert.rejects(other.transport.review(other.id),/integrity/);
});

test('private operator data, unscoped source and nonliteral manifest paths are rejected',async()=>{
  for(const mutate of [({chunks})=>{chunks['artifact.json'].inputs=[];},({chunks})=>{chunks['artifact.json'].files[0].path='src/private/key.rs';},({manifest})=>{manifest.routes.artifact='../artifact.json';}]){const f=fixture({mutate});await assert.rejects(f.transport.review(f.id),PublicWorkspaceError);}
});

test('catalog, detail and bounded source preserve exact HLD/entry/evidence pins through the existing controller',async()=>{
  const f=fixture(),workspace=new ReviewWorkspace({request:(...args)=>f.transport.request(...args)});await workspace.selectReview(f.id);await workspace.loadLldCatalog({parentHld:'hld-proposal',component:'api'});assert.equal(workspace.state.lld.entries[0].proposal.id,'entry-proposal');
  await workspace.loadLldDetail('entry-baseline');await tick();assert.equal(workspace.state.lldDetail.entry_id,'entry-baseline');assert.equal(workspace.state.lldDetail.baseline.entry.id,'entry-baseline');assert.equal(workspace.state.lldDetail.proposal.entry.id,'entry-proposal');
  const evidence=workspace.state.lldDetail.proposal.evidence[0];await workspace.loadLldSource(evidence);assert.equal(workspace.state.lldSource.text,'fn handle() {\n  return 2;\n}');assert.equal(workspace.state.lldSource.blob_id,'2'.repeat(40));assert.equal(workspace.state.lldSourceComparison.baseline.commit,'a'.repeat(40));
  const before=workspace.state.details;await workspace.loadLldDetail('not-published');assert.equal(workspace.state.lldDetail,null);assert.equal(workspace.state.details,before);assert.match(workspace.state.lldError,/not_generated/);
});

test('a same-review LLD catalogue with another scope cannot enter the controller',async()=>{
  const f=fixture(),workspace=new ReviewWorkspace({request:async(path,options)=>{const value=await f.transport.request(path,options);return path.includes('/lld/catalog?')?{...value,scope_version:'another-published-scope'}:value;}});await workspace.selectReview(f.id);await workspace.loadLldCatalog({parentHld:'hld-proposal',component:'api'});assert.equal(workspace.state.lld,null);assert.match(workspace.state.lldError,/does not match/);assert.equal(workspace.state.details.scope_version,'scope-1');assert.ok(workspace.state.files);
});

test('public catalogue cursors and source URLs cannot override comparison or filesystem boundaries',async()=>{
  const f=fixture();await assert.rejects(f.transport.request(`${f.base}/lld/catalog?parent_hld=wrong`),/parent_unavailable/);
  await assert.rejects(f.transport.request(`${f.base}/lld/catalog?parent_hld=hld-proposal&cursor=other:1`),/cursor/);
  for(const suffix of ['/source?side=proposal&path=src%2Fapi.rs&blob=wrong&start_line=1&end_line=3','/lld/source?parent_hld=hld-proposal&component=api&entry=entry-proposal&evidence=source-proposal&path=secret','/files?limit=100&limit=100','/files?file=../../secrets'])await assert.rejects(f.transport.request(f.base+suffix),PublicWorkspaceError);
  const data=await f.transport.fileDescriptors(f.id,f.path);const source=await f.transport.request(`${f.base}/source?${new URLSearchParams({side:'proposal',path:f.path,blob:data.proposal.blob_id,start_line:'1',end_line:'200'})}`);assert.equal(source.end_line,3);assert.equal(source.text,data.proposal.text);
});

test('missing static spans are explicit frontiers and preserve unrelated reviewed evidence',async()=>{
  const f=fixture({mutate:({manifest})=>{manifest.routes.file_sources=[];}}),workspace=new ReviewWorkspace({request:(...args)=>f.transport.request(...args)});await workspace.selectReview(f.id);const details=workspace.state.details;await workspace.loadSource({path:f.path,blob:'2'.repeat(40),side:'proposal',startLine:1,endLine:3});assert.equal(workspace.state.details,details);assert.match(workspace.state.error,/not_published/);assert.ok(f.calls.every(call=>!call.url.includes('/v1/')));
});

test('only bounded page-memory reviewed files, questions and bookmarks can be written',async()=>{
  const f=fixture(),workspace=new ReviewWorkspace({request:(...args)=>f.transport.request(...args)});await workspace.selectReview(f.id);
  const write={id:'tick',kind:'reviewed',visibility:'private',target_path:f.path,target_key:f.path,body:'',status:'active',expected_version:0};await workspace.writeAnnotation(write);assert.equal(workspace.state.annotations.items[0].storage,'page_memory');
  await workspace.writeAnnotation({...write,status:'resolved',expected_version:1});assert.equal(workspace.state.annotations.items[0].status,'resolved');
  await assert.rejects(workspace.writeAnnotation({...write,id:'shared',visibility:'shared'}),/page-memory/);await assert.rejects(workspace.writeAnnotation({...write,id:'handoff',kind:'handoff'}),/page-memory/);await assert.rejects(workspace.writeAnnotation({...write,id:'outside',target_key:'src/private/key',target_path:'src/private/key'}),/page-memory/);
  await assert.rejects(f.transport.request(`${f.base}/hld/accept`,{method:'POST',body:'{}'}),/new published analysis/);
  const again=fixture();assert.deepEqual((await again.transport.request(`${again.base}/annotations`)).items,[],'another page transport does not persist annotations');
});

test('a delayed invalid public read cannot clear a newer review generation',async()=>{
  const f=fixture();let release;const workspace=new ReviewWorkspace({request:async(path,options)=>{if(path.includes('/lld/detail'))return new Promise((_,reject)=>{release=reject;});return f.transport.request(path,options);}});await workspace.selectReview(f.id);await workspace.loadLldCatalog({parentHld:'hld-proposal',component:'api'});const old=workspace.loadLldDetail('entry-proposal');await workspace.selectReview(f.id);release(new PublicWorkspaceError('old integrity failure',{invalidateReview:true}));await old;assert.ok(workspace.state.details);assert.equal(workspace.state.error,null);
});

test('source parsing cancellation is preserved instead of becoming a public integrity failure',async()=>{
  const f=fixture(),abort=new AbortController();abort.abort();await assert.rejects(f.transport.review(f.id,abort.signal),error=>error.name==='AbortError');assert.equal(f.calls.length,0);
});

test('the browser native fetch receiver is preserved instead of invoking it on the transport',async()=>{
  const f=fixture();let receiver;const transport=new PublicStaticTransport({indexURL:'https://example.test/project/index.json',crypto:webcrypto,fetch:async function(url){receiver=this;return new Response(f.resources.get(url));}});await transport.bootstrap();assert.equal(receiver,globalThis);
});

test('long immutable LLD breadcrumbs cannot expand the reviewer grid beyond a narrow viewport',()=>{
  const css=fs.readFileSync(require.resolve('../src/workspace.css'),'utf8');assert.match(css,/\.lld-review\{grid-template-columns:minmax\(0,1fr\);min-width:0\}/);assert.match(css,/\.design-breadcrumbs>span\{min-width:0;max-width:100%;overflow-wrap:anywhere\}/);assert.match(css,/\.architecture-svg\{[^}]*overflow:auto/,'graphs keep their own pan surface instead of shrinking text');
});

test('PR verdict never treats a completed changed comparison or missing coverage as green',()=>{
  const {publicReviewVerdict}=require('../src/public-workspace.js');
  const complete={complete:true,gaps:[],deferred_details:0};
  const unchanged={hld_status:'no_change_detected_within_scope',lld_status:'no_change_detected_within_scope'};
  assert.equal(publicReviewVerdict(complete,unchanged).kind,'no_change_detected');
  assert.equal(publicReviewVerdict(complete,{...unchanged,lld_status:'change_detected'}).kind,'change_detected');
  assert.equal(publicReviewVerdict({...complete,gaps:['unresolved']},unchanged).kind,'incomplete');
  assert.equal(publicReviewVerdict(complete,{}).kind,'incomplete');
});

test('repository dashboard filters published reports before pagination',async()=>{
  const f=fixture();
  const present=await f.transport.request('/v1/prs?repository_id=1');
  assert.equal(present.items.length,1);
  const absent=await f.transport.request('/v1/prs?repository_id=2');
  assert.equal(absent.items.length,0);
  const all=await f.transport.request('/v1/prs');
  assert.equal(all.items.length,1);
});
