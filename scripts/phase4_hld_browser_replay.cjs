#!/usr/bin/env node
'use strict';
/* Optional offline browser QA; no application npm dependency or browser download.
 * Prerequisites: built Rust binaries, scripts/phase4_browser_fixture.py fixture,
 * fixture-only scripts/github_mvp_server.py running on loopback port 3201,
 * installed Playwright (validated with 1.62.1) and installed Chromium/Chrome.
 * PLAYWRIGHT_MODULE=/absolute/path/to/playwright \
 * CSB_CHROMIUM_EXECUTABLE=/absolute/path/to/chrome \
 * CSB_QA_MODE=lifecycle node scripts/phase4_hld_browser_replay.cjs target/NEW_FIXTURE/fixture.json
 * Modes: lifecycle (mutates only private synthetic accepted records), sample,
 * performance (3 seeds; each has 25 sequential sessions, 100 graph and 100 source actions).
 * Use a fresh fixture for a reproducible lifecycle. This is not hosted capacity QA.
 */
const fs=require('fs');const path=require('path');const crypto=require('crypto');const assert=require('assert/strict');const {execFileSync}=require('child_process');
const playwrightModule=process.env.PLAYWRIGHT_MODULE||'playwright';
const {chromium}=require(playwrightModule);
const playwrightVersion=JSON.parse(fs.readFileSync(require.resolve(path.join(playwrightModule,'package.json')),'utf8')).version;
const repo=path.resolve(__dirname,'..'),fixturePath=process.argv[2],origin=process.env.CSB_QA_ORIGIN||'http://127.0.0.1:3201',mode=process.env.CSB_QA_MODE||'lifecycle';
assert.ok(fixturePath,'Usage: node scripts/phase4_hld_browser_replay.cjs target/NEW_FIXTURE/fixture.json');
assert.ok(['lifecycle','sample','performance'].includes(mode),'Unknown QA mode');
let address;try{address=new URL(origin);}catch{throw new Error('Invalid fixture origin; expected loopback HTTP without credentials');}
assert.equal(address.protocol,'http:','Only an offline loopback HTTP fixture is allowed');
assert.ok(['127.0.0.1','localhost','[::1]'].includes(address.hostname),'Only loopback fixtures are allowed');
assert.ok(address.port && !['3102','3195','3196','3197'].includes(address.port),'Refusing user preview/application ports; default fixture port is 3201');
assert.ok(address.pathname==='/'&&!address.username&&!address.password&&!address.search&&!address.hash,'Fixture origin cannot contain credentials, paths or query parameters');
const targetRoot=fs.realpathSync(path.join(repo,'target'));
const inside=(file,root)=>{const relative=path.relative(root,fs.realpathSync(file));assert.ok(relative && !relative.startsWith('..'+path.sep) && relative!=='..' && !path.isAbsolute(relative),'Only private synthetic paths inside target are allowed');};
inside(fixturePath,targetRoot);
const fixture=JSON.parse(fs.readFileSync(fixturePath,'utf8')),output=path.join(repo,'target/phase4-qa');
assert.match(fixture.slug,/^fixture\/[a-z0-9-]+$/,'Only a synthetic fixture repository is allowed');
inside(fixture.root,targetRoot);for(const field of ['repo','output','bin'])inside(fixture[field],fs.realpathSync(fixture.root));
assert.ok(execFileSync('git',['-C',fixture.repo,'remote','get-url','origin'],{encoding:'utf8'}).trim()===`https://github.com/${fixture.slug}.git`,'Refusing a non-fixture repository');
const fixtureArtifact=JSON.parse(fs.readFileSync(path.join(fixture.output,'pr-99.json'),'utf8'));
const syntheticIdentity=fixtureArtifact.hld?.proposal?.repository;
assert.equal(`${syntheticIdentity?.owner}/${syntheticIdentity?.name}`,fixture.slug);
fs.mkdirSync(output,{recursive:true});
let browser,page;const errors=[],calls=[];
const qa={schema:'csb-phase4-native-browser-qa-v1',mode,fixture_path:fixturePath,origin,playwright_version:playwrightVersion,node_version:process.version,transport:'Actual loopback Python host + built native Rust engine + private SQLite state; fixture-only gh CLI stub; no GitHub or provider network',checks:[],errors,timings:[],source_pins:[]};
const rafs=async()=>page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
const percentile=(values,p)=>[...values].sort((a,b)=>a-b)[Math.max(0,Math.ceil(p*values.length)-1)];
async function openReview(number,view='combined') {
 const item=fixture.reviews[String(number)];assert.ok(item,`Missing fixture PR${number}`);
 await page.goto(`${origin}/workspace#review=${encodeURIComponent(item.review_id)}&layout=architecture&side=diff&design=hld&design_view=${view}`);
 await page.locator('#hld-diagrams svg').waitFor();await page.waitForFunction(()=>!document.getElementById('hld-reload').disabled);await rafs();return item;
}
async function hldJson(number) {
 const token=await page.locator('meta[name="csb-local-session"]').getAttribute('content');const id=fixture.reviews[String(number)].review_id;
 const result=await page.request.get(`${origin}/v1/reviews/${encodeURIComponent(id)}/hld`,{headers:{Authorization:`Bearer ${token}`}});assert.equal(result.status(),200);const data=await result.json();assert.equal(`${data.proposal?.repository?.owner}/${data.proposal?.repository?.name}`,fixture.slug,'Host must serve this synthetic repository');assert.equal(data.review_id,id);return data;
}
async function chooseComponent(id) {
 await page.locator(`#hld-element-list button`).filter({has:page.locator('span')}).count();
 const index=await page.locator('#hld-element-list button').evaluateAll((buttons,id)=>buttons.findIndex(button=>button.dataset.designElement===id),id);
 if(index>=0)await page.locator('#hld-element-list button').nth(index).click();else{const found=page.locator(`#hld-diagrams g.node[data-design-element="${id}"]`);await found.first().click();}
 await rafs();
}
async function previewAccept({interpretation='as_built',design='proposal',responsibility,document=false,activate=true}={}) {
 if(!(await page.locator('#hld-acceptance').evaluate(element=>element.open)))await page.locator('#hld-acceptance summary').click();
 await page.locator('#hld-confirm-design').selectOption(design);await page.locator('#hld-interpretation').selectOption(interpretation);
 if(responsibility){await page.locator('#hld-correction-kind').selectOption('responsibility');await page.locator('#hld-responsibility').fill(responsibility);await page.locator('#hld-add-correction').click();assert.match(await page.locator('#hld-corrections-list').innerText(),new RegExp(responsibility));}
 if(document){await page.locator('#hld-correction-kind').selectOption('document_reference');assert.ok(await page.locator('#hld-document-evidence option').count());await page.locator('#hld-responsibility').fill('Documented intent requires reviewer verification against the current public contract');await page.locator('#hld-add-correction').click();assert.match(await page.locator('#hld-corrections-list').innerText(),/document_reference/);}
 await page.locator('#hld-known-gaps').check();await page.locator('#hld-activate-default').setChecked(activate);
 await page.locator('#hld-preview-acceptance').click();await page.waitForFunction(()=>!document.getElementById('hld-confirm').disabled||document.getElementById('hld-confirmation-status').textContent.includes('unavailable'));
 assert.equal(await page.locator('#hld-confirm').isDisabled(),false,await page.locator('#hld-confirmation-status').innerText());const preview=JSON.parse(await page.locator('#hld-confirmation-preview').innerText());assert.equal(preview.review.interpretation,interpretation);
 assert.ok(preview.design.id);await page.locator('#hld-confirm').click();await page.waitForFunction(()=>document.getElementById('hld-confirmation-status').textContent.startsWith('Accepted')||document.getElementById('hld-confirmation-status').textContent.includes('unavailable'));
 assert.match(await page.locator('#hld-confirmation-status').innerText(),/^Accepted/);return preview;
}
async function lifecycle() {
 await openReview(99);const data=await hldJson(99);assert.equal(data.delta.outcome,'change_detected');assert.ok(data.baseline.document_candidates?.length,'Design-document seam must be present');
 qa.checks.push('actual-native-HLD-change-detected');const component=data.proposal.elements.find(element=>element.kind==='component');assert.ok(component);await chooseComponent(component.id);
 assert.equal(await page.locator('#hld-diagrams g.node').count(),data.proposal.elements.filter(element=>element.kind!=='contract').length);
 assert.ok(await page.locator('#hld-inspector .hld-evidence button').count());const sourceResponse=page.waitForResponse(response=>response.url().includes('/hld/source?')&&response.status()===200);await page.locator('#hld-inspector .hld-evidence button').first().click();const source=await (await sourceResponse).json();await page.locator('#hld-source-text:not([hidden])').waitFor();assert.ok([fixture.baseline,fixture.proposal].includes(source.commit));assert.equal(source.review_id,fixture.reviews['99'].review_id);qa.source_pins.push({commit:source.commit,path:source.path,blob_id:source.blob_id,start_line:source.start_line,end_line:source.end_line});qa.checks.push('exact-native-source-preview');
 for(const [view,wanted,expected]of [['baseline','api/src/lib.rs',fixture.baseline],['proposal','api/src/lib.rs',fixture.proposal],['proposal','api/Cargo.toml',fixture.proposal]]){await page.locator('#hld-'+view).click();await page.locator('#hld-diagrams svg').waitFor();await chooseComponent(component.id);const response=page.waitForResponse(response=>response.url().includes('/hld/source?')&&response.status()===200);await page.locator('#hld-inspector .hld-evidence button').filter({hasText:wanted+':'}).first().click();const wire=await(await response).json();await page.locator('#hld-source-text:not([hidden])').waitFor();assert.equal(wire.commit,expected);assert.equal(wire.path,wanted);qa.source_pins.push({kind:wanted==='api/Cargo.toml'?'unchanged-manifest':'changed-path-revision-pin',commit:wire.commit,path:wire.path,blob_id:wire.blob_id,start_line:wire.start_line,end_line:wire.end_line});}qa.checks.push('baseline-proposal-path-revision-pins-and-unchanged-manifest-source');
 for(const [view,expected,signature]of [['baseline',fixture.baseline,'pub fn handle() -> u32 { charge() }'],['proposal',fixture.proposal,"pub fn handle() -> Result<u32, &'static str> { charge() }"]]){
  const version=data[view],contract=version.elements.find(element=>element.kind==='contract'&&element.name==='handle');assert.ok(contract,'Fixture public contract handle must be present');
  const evidence=version.evidence.find(item=>contract.evidence_ids.includes(item.id)&&item.relationship_type==='rust_public_contract');assert.ok(evidence);assert.equal(evidence.start_line,2);assert.equal(evidence.end_line,2);
  await page.locator('#hld-'+view).click();await page.locator('#hld-diagrams svg').waitFor();
  await page.locator('#hld-element-list .hld-element').filter({has:page.locator('span').filter({hasText:/^handle$/})}).click();
  const response=page.waitForResponse(result=>result.url().includes('/hld/source?')&&result.status()===200);
  await page.locator('#hld-inspector .hld-evidence button').filter({hasText:'rust_public_contract'}).click();
  const wire=await(await response).json();await page.locator('#hld-source-text:not([hidden])').waitFor();await rafs();
  assert.equal(wire.commit,expected);assert.equal(wire.path,'api/src/lib.rs');assert.equal(wire.blob_id,evidence.blob);assert.equal(wire.start_line,2);assert.equal(wire.end_line,2);assert.equal(wire.text.trim(),signature);
  assert.ok((await page.locator('#hld-source-text').innerText()).includes(signature));
  qa.source_pins.push({kind:'changed-public-contract-signature',design_id:version.id,evidence_id:evidence.id,relationship_type:evidence.relationship_type,commit:wire.commit,path:wire.path,blob_id:wire.blob_id,start_line:wire.start_line,end_line:wire.end_line,observed_signature:signature});
 }qa.checks.push('changed-public-contract-handle-baseline-u32-proposal-Result-exact-line2-blob-source');
 await chooseComponent(component.id);
 await page.locator('#hld-compare').click();await page.waitForFunction(()=>document.querySelectorAll('#hld-diagrams svg').length===2);await page.locator('#hld-baseline').click();await page.waitForFunction(()=>document.querySelectorAll('#hld-diagrams svg').length===1);await page.locator('#hld-proposal').click();await page.locator('#hld-diagrams svg').waitFor();await page.locator('#hld-combined').click();await page.locator('#hld-diagrams svg').waitFor();qa.checks.push('baseline-proposal-diff-before-after');
 await page.locator('#hld-zoom-lld').click();assert.match(await page.locator('#lld-unavailable').innerText(),/unavailable/);const lldHash=await page.evaluate(()=>location.hash);assert.match(lldHash,/design=lld/);assert.match(lldHash,/parent_hld=/);assert.match(lldHash,/component=/);assert.match(lldHash,/detail=module/);await page.locator('#lld-back-hld').click();await page.locator('#hld-diagrams svg').waitFor();qa.checks.push('HLD-component-LLD-pinned-route-honest-unavailable');
 await page.screenshot({path:path.join(output,'hld-native-desktop.png'),fullPage:true});
 if(!(await page.locator('#hld-acceptance').evaluate(element=>element.open)))await page.locator('#hld-acceptance summary').click();await page.locator('#hld-confirm-design').selectOption('proposal');await page.locator('#hld-correction-kind').selectOption('constraint');await page.locator('#hld-correction-element').selectOption(component.id);const other=data.proposal.elements.find(element=>element.kind==='component'&&element.id!==component.id);await page.locator('#hld-related-elements').selectOption(other.id);await page.locator('#hld-responsibility').fill('Review this declared component dependency rule before adopting it');await page.locator('#hld-add-correction').click();await page.locator('#hld-correction-kind').selectOption('correspondence');await page.locator('#hld-correction-element').selectOption(component.id);await page.locator('#hld-correspondence-sources').selectOption(component.id);await page.locator('#hld-add-correction').click();await page.locator('#hld-preview-acceptance').click();await page.waitForFunction(()=>!document.getElementById('hld-confirm').disabled||document.getElementById('hld-confirmation-status').textContent.includes('unavailable'));assert.equal(await page.locator('#hld-confirm').isDisabled(),false,await page.locator('#hld-confirmation-status').innerText());const typed=JSON.parse(await page.locator('#hld-confirmation-preview').innerText());assert.ok(typed.review.corrections.some(item=>item.kind==='constraint'));assert.ok(typed.review.corrections.some(item=>item.kind==='correspondence'));qa.checks.push('native-typed-constraint-and-cross-snapshot-correspondence-preview-no-autoaccept');await page.locator('#hld-clear-corrections').click();
 const asBuilt=await previewAccept({responsibility:'Owns reviewer-confirmed API contract validation',document:true});qa.checks.push('native-responsibility-and-document-preview-gap-acknowledgement-explicit-default-accept');
 await page.reload();await page.locator('#hld-diagrams svg').waitFor();const after=await hldJson(99);assert.ok(after.accepted);assert.equal(after.accepted.snapshot,fixture.proposal);assert.ok(after.accepted.elements.some(element=>element.responsibility==='Owns reviewer-confirmed API contract validation'));assert.ok(after.accepted.decision_records?.some(record=>record.verification==='requires_reviewer_verification'));assert.equal(after.accepted.view,'accepted_as_built');qa.checks.push('accepted-record-persists-after-browser-reload');
 await page.locator('#hld-accepted').click();await page.locator('#hld-diagrams svg').waitFor();await page.locator('#hld-provenance').count();assert.equal(await page.locator('#hld-export').isDisabled(),false);const download=page.waitForEvent('download');await page.locator('#hld-export').click();const exported=await download;await exported.saveAs(path.join(output,'accepted-hld.md'));const markdown=fs.readFileSync(path.join(output,'accepted-hld.md'),'utf8');assert.ok(markdown.includes(after.accepted.snapshot));assert.ok(markdown.includes('```mermaid'));qa.checks.push('authorized-native-selected-view-Markdown-export');
 await page.locator('#hld-proposal').click();await page.locator('#hld-diagrams svg').waitFor();await previewAccept({interpretation:'intended',responsibility:'Intended API evolution remains separate from observed code'});const intended=await hldJson(99);assert.equal(intended.accepted.id,after.accepted.id);assert.equal(intended.accepted_intended.view,'intended');assert.notEqual(intended.accepted_intended.id,intended.accepted.id);qa.checks.push('intended-and-as-built-defaults-separate');
 await openReview(100);const helper=await hldJson(100);assert.equal(helper.delta.changes.length,0);assert.equal(helper.delta.outcome,'incomplete');assert.ok(helper.accepted_drift?.changes.length);assert.match(await page.locator('#hld-drift-status').innerText(),/before this PR/);qa.checks.push('private-helper-qualified-incomplete-with-pre-existing-drift-separated');
 if(fixture.reviews['101']){await openReview(101);const merged=await hldJson(101);assert.ok(merged.proposal.snapshot!==fixture.proposal);assert.equal(merged.accepted.id,after.accepted.id);qa.checks.push('actual-merged-snapshot-candidate-does-not-mutate-default');}
 await page.setViewportSize({width:390,height:844});await page.screenshot({path:path.join(output,'hld-native-mobile.png'),fullPage:true});qa.mobile_overflow=await page.evaluate(()=>({scroll:document.documentElement.scrollWidth,width:innerWidth}));assert.ok(qa.mobile_overflow.scroll<=qa.mobile_overflow.width);qa.checks.push('390px-no-page-overflow');
 qa.acceptance_pins={preview_id:asBuilt.preview_id,input_fingerprint:asBuilt.input_fingerprint,expected_revision:asBuilt.expected_revision,accepted_id:after.accepted.id,intended_id:intended.accepted_intended.id};
}
async function warmup(){
 const started=Date.now();await openReview(99);qa.cold_entry_ms=Date.now()-started;qa.warmup=[];
 for(let i=0;i<10;i++){const start=await page.evaluate(()=>performance.now());const done=page.waitForResponse(response=>response.url().endsWith(`/v1/reviews/${fixture.reviews['99'].review_id}/hld`)&&response.request().method()==='GET');await page.locator('#hld-reload').click();await done;await page.waitForFunction(()=>document.querySelector('#hld-diagrams svg')&&!document.getElementById('hld-reload').disabled);await rafs();const graph=await page.evaluate(start=>performance.now()-start,start);const sourceStart=await page.evaluate(()=>performance.now());const sourceDone=page.waitForResponse(response=>response.url().includes('/hld/source?'));await page.locator('#hld-inspector .hld-evidence button').first().click();assert.equal((await sourceDone).status(),200);await page.locator('#hld-source-text:not([hidden])').waitFor();await rafs();const source=await page.evaluate(start=>performance.now()-start,sourceStart);qa.warmup.push({graph_ms:graph,source_ms:source});console.log(JSON.stringify({kind:'warmup',action:i+1,graph_ms:Number(graph.toFixed(2)),source_ms:Number(source.toFixed(2))}));}
}
async function measure(runs=3,sessions=25,actions=4){
 await warmup();
 for(let run=0;run<runs;run++){
  let seed=1729+run*7919;const random=()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed;};const graph=[],source=[];
  for(let session=0;session<sessions;session++){
   await openReview(session%2?100:99);const number=session%2?100:99;
   for(let action=0;action<actions;action++){
    const started=await page.evaluate(()=>performance.now());const response=page.waitForResponse(response=>response.url().endsWith(`/v1/reviews/${fixture.reviews[String(number)].review_id}/hld`)&&response.request().method()==='GET');
    await page.locator('#hld-reload').click();await response;await page.waitForFunction(()=>document.querySelector('#hld-diagrams svg')&&!document.getElementById('hld-reload').disabled);await rafs();const graphDuration=await page.evaluate(start=>performance.now()-start,started);graph.push(graphDuration);
    const buttons=await page.locator('#hld-element-list .hld-element').count();const index=random()%Math.min(buttons,2);await page.locator('#hld-element-list .hld-element').nth(index).click();await rafs();assert.ok(await page.locator('#hld-inspector .hld-evidence button').count());
    const sourceStarted=await page.evaluate(()=>performance.now());const sourceResponse=page.waitForResponse(response=>response.url().includes('/hld/source?')&&response.request().method()==='GET');await page.locator('#hld-inspector .hld-evidence button').first().click();const wire=await sourceResponse;assert.equal(wire.status(),200);await page.locator('#hld-source-text:not([hidden])').waitFor();await rafs();const sourceDuration=await page.evaluate(start=>performance.now()-start,sourceStarted);source.push(sourceDuration);
    if(run===0&&session<2)console.log(JSON.stringify({kind:'early_sample',run:run+1,session:session+1,action:action+1,graph_ms:Number(graphDuration.toFixed(2)),source_ms:Number(sourceDuration.toFixed(2))}));
   }
   if((session+1)%5===0)console.log(JSON.stringify({kind:'progress',run:run+1,sessions:session+1,graph_count:graph.length,source_count:source.length}));
  }
  qa.timings.push({seed:1729+run*7919,sequential_sessions:sessions,graph_interactions:graph.length,source_interactions:source.length,graph_ms:graph,source_ms:source,graph_p95_ms:percentile(graph,.95),source_p95_ms:percentile(source,.95),graph_target_ms:300,source_target_ms:500,targets_passed:percentile(graph,.95)<300&&percentile(source,.95)<500});console.log(JSON.stringify({kind:'run_completed',...qa.timings.at(-1),graph_ms:undefined,source_ms:undefined}));
 }
 qa.checks.push(runs===3&&sessions===25&&actions===4?'three-independent-seeds-25-sequential-sessions-100-graph-and-source-actions-each-native-http-two-frames':`native-sample-${runs}-run-${sessions}-sequential-sessions-${sessions*actions}-graph-and-source-actions-http-two-frames`);qa.performance_profile='Fresh native HLD reauthorization followed by usable current immutable diagram (reused when exact design IDs match); separately cold-rendered on first session entry. Local manual-review, one active primary repository analysis; generated synthetic repository with fixture gh CLI subprocess overhead, warm native read-only SQLite; not hosted provider network/capacity';
}
(async()=>{
 browser=await chromium.launch({headless:true,executablePath:process.env.CSB_CHROMIUM_EXECUTABLE||'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',chromiumSandbox:true});qa.browser_version=browser.version();qa.chromium_sandbox=true;page=await browser.newPage({viewport:{width:1440,height:1100},acceptDownloads:true});page.setDefaultTimeout(20000);page.on('pageerror',error=>errors.push(error.message));page.on('response',response=>{if(response.url().startsWith(origin)&&response.url().includes('/hld'))calls.push({path:new URL(response.url()).pathname,status:response.status(),method:response.request().method()});});
 if(mode==='lifecycle')await lifecycle();else if(mode==='sample')await measure(1,2,4);else await measure();
 assert.deepEqual(errors,[]);qa.status=qa.timings.some(run=>!run.targets_passed)?'latency_target_failed':'passed';qa.head_commit=execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim();qa.working_tree_paths=execFileSync('git',['status','--porcelain'],{encoding:'utf8'}).trimEnd().split('\n').filter(Boolean).map(line=>line.slice(3));qa.working_tree_dirty=qa.working_tree_paths.length>0;qa.uncommitted_implementation=qa.working_tree_paths.some(file=>!file.startsWith('docs/'));qa.command={script:'scripts/phase4_hld_browser_replay.cjs',mode,fixture_path:fixturePath};qa.source_sha256=Object.fromEntries(['src/workspace.js','src/workspace.html','src/workspace.css','scripts/github_mvp_server.py','scripts/phase4_hld_browser_replay.cjs','target/debug/csb-onboarding','target/debug/flowdiff'].map(file=>[file,crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')]));qa.calls=calls;fs.writeFileSync(path.join(output,`hld-native-${mode}.json`),JSON.stringify(qa,null,2)+'\n');console.log(JSON.stringify({status:qa.status,report:path.join(output,`hld-native-${mode}.json`),checks:qa.checks.length}));await browser.close();if(qa.status!=='passed')process.exitCode=1;
})().catch(async error=>{qa.status='failed';qa.failure=error.message;if(page){qa.visible_error=await page.locator('#hld-confirmation-status').innerText().catch(()=>null);qa.hld_status=await page.locator('#hld-summary').innerText().catch(()=>null);await page.screenshot({path:path.join(output,'hld-native-failure.png'),fullPage:true}).catch(()=>{});}qa.calls=calls;fs.writeFileSync(path.join(output,`hld-native-${mode}.json`),JSON.stringify(qa,null,2)+'\n');console.error(JSON.stringify({failure:qa.failure,visible_error:qa.visible_error,hld_status:qa.hld_status}));if(browser)await browser.close();process.exit(1);});
