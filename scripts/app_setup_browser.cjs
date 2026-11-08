'use strict';
// Offline UI contract tests. All HTTPS is fulfilled locally; no GitHub/App calls.
// PLAYWRIGHT_MODULE=/path/to/playwright CSB_CHROMIUM_EXECUTABLE=/path/to/chrome node scripts/app_setup_browser.cjs
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const {chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
const root=path.resolve(__dirname,'..'),output=path.join(root,'target/app-setup-browser');fs.mkdirSync(output,{recursive:true});
const origin='https://setup.example.test',checks=[],requests=[],unexpected=[];
const repo={installation_id:20,repository_id:30,owner:'fixture',name:'review-rs',state:'installed_unconfigured'};
const entries=[{path:'.env',kind:'file'},{path:'Cargo.toml',kind:'file'},{path:'README.md',kind:'file'},{path:'src',kind:'directory'},{path:'src/lib.rs',kind:'file'},{path:'src/<img src=x>.rs',kind:'file'},{path:'vendor',kind:'submodule'}].map(e=>({...e,oid:'c'.repeat(40)}));
const state={enabled:false,authenticated:true,saved:null,preview:null,commit:'a'.repeat(40),conflict:false,denied:false,failed:false,sourceAvailable:false,architecture:null,consent:null};
const inventory=()=>({repository_id:30,default_branch:'main',commit:state.commit,tree:'b'.repeat(40),entries});
const includePath=(p,roots)=>roots.some(r=>r==='.'||p===r||p.startsWith(r+'/'));
async function main(){
 const browser=await chromium.launch({headless:true,executablePath:process.env.CSB_CHROMIUM_EXECUTABLE});
 try {
 const context=await browser.newContext({viewport:{width:1280,height:900},reducedMotion:'reduce'});
 await context.route('**/*',async route=>{
   const request=route.request(),url=new URL(request.url());
   if(url.origin!==origin){unexpected.push(request.url());return route.abort();}
   const assets={'/github-app/setup':['src/github_app_setup/setup.html','text/html'],'/assets/app-setup.js':['src/github_app_setup/setup.js','text/javascript'],'/assets/app-setup.css':['src/onboarding.css','text/css']};
   if(assets[url.pathname]){const [file,type]=assets[url.pathname];return route.fulfill({contentType:type,body:fs.readFileSync(path.join(root,file),'utf8')+(type==='text/css'?fs.readFileSync(path.join(root,'src/github_app_setup/setup.css'),'utf8'):''),headers:{'content-security-policy':"default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"}});}
   if(url.pathname==='/favicon.ico')return route.fulfill({status:204});
   const data=request.postDataJSON();requests.push({path:url.pathname,method:request.method(),data});
   const send=(value,status=200)=>route.fulfill({status,contentType:'application/json',body:JSON.stringify(value)});
   const key=url.pathname.replace('/v1/github-app/','');
   if(key==='capability')return send({available:state.enabled,activation:'off',scope_confirmation_available:state.enabled,checks_available:false,source_preview_available:state.sourceAvailable,publication_consent_available:state.sourceAvailable});
   if(!state.authenticated)return send({error:'github_authentication_required'},401);
   if(key==='repositories'&&request.method()==='GET')return send({repositories:[repo],activation:'off'});
   if(key==='logout'){state.authenticated=false;return route.fulfill({status:204});}
   assert.equal(data.installation_id,20);assert.equal(data.repository_id,30);
   if(state.denied)return send({error:'repository_setup_not_authorized'},403);
   if(state.failed)return send({error:'github_setup_unavailable'},503);
   if(key==='repositories')return send({repository:repo,scan_started:false,activation:'off'});
   if(key==='inventory')return send({inventory:inventory(),saved:state.saved,automatically_excluded:['.env','vendor'],activation:'off',source_scanned:false});
   if(key==='architecture/status')return send({scope_revision:state.saved.revision,architecture_confirmed:!!state.architecture,activation:'off',publication_consent:false});
   if(key==='publication/status')return send({publication_consent:!!state.consent,consent_revision:state.consent?1:0,activation:'off'});
   if(key==='publication/preview')return send({document_digest:'f'.repeat(64),document:{expected_revision:state.consent?1:0,scope_revision:state.saved.revision,scope_commit:state.commit,policy:{include:state.saved.document.include,exclude:state.saved.document.exclude},operator:{report_origin:'https://reports.example.test',engine_digest:'a'.repeat(64)},source_visibility:'public authorized excerpts',retention_days:7,limits:{unique_blobs:64,job_rest_reservations:384,installation_daily_rest_reservations:2000},expires_at:Math.floor(Date.now()/1000)+600}});
   if(key==='publication/confirm'){assert.equal(data.allow_automatic_analysis,true);assert.equal(data.allow_public_source,true);assert.equal(data.allow_checks,true);assert.equal(data.document_digest,'f'.repeat(64));assert.ok(!('activate' in data));state.consent=data;return send({consent_revision:1,activation:'off'});}
   if(key==='publication/revoke'){state.consent=null;return send({publication_consent:false,activation:'off'});}
   if(key==='architecture/preview'){
     assert.equal(data.allow_selected_source_scan,true);assert.equal(data.scope_revision,state.saved.revision);
     return send({preview_id:'e'.repeat(64),scope_revision:state.saved.revision,expires_at_s:Date.now()/1000+900,activation:'off',result:{version:'csb-app-preview-v1',components:[{id:'component-fixture',name:'Rust API',roots:['src'],role:'backend candidate',stack_candidates:['Rust','Axum · static import'],evidence:[{path:'src/lib.rs',blob:'c'.repeat(40),start_line:1,end_line:5}],contradictions:['Static import does not prove an active server']}],providers:[],gaps:['Runtime and hosting unverified']}});
   }
   if(key==='architecture/confirm'){
     assert.equal(data.preview_id,'e'.repeat(64));assert.equal(data.accept_known_gaps,true);assert.equal(data.defer_integrations,true);assert.equal(data.components[0].decision,'corrected');assert.equal(data.components[0].name,'Checkout API');assert.ok(!('activate' in data));
     state.architecture=data;return send({scope_revision:state.saved.revision,architecture_confirmed:true,activation:'off',publication_consent:false});
   }
   if(key==='scope/preview'){
     assert.equal(data.commit,state.commit);assert.equal(data.expected_revision,state.saved?.revision||0);
     const files=entries.filter(e=>e.kind==='file'&&e.path!=='.env'&&includePath(e.path,data.include)&&!includePath(e.path,data.exclude)).map(e=>e.path);
     if(!files.length)return send({error:'invalid_setup_request'},400);
     state.preview={preview_id:'d'.repeat(64),expires_at_s:Date.now()/1000+900,expected_revision:data.expected_revision,activation:'off',document:{schema:'csb-app-scope-intent-v1',installation_id:20,repository_id:30,commit:state.commit,tree:'b'.repeat(40),default_branch:'main',include:data.include,exclude:data.exclude,files,rust_candidates:files.filter(p=>p.endsWith('.rs')).length,unsupported_files:files.filter(p=>!p.endsWith('.rs')&&p!=='Cargo.toml').length,automatically_excluded:0,source_scanned:false,architecture_confirmed:false,publication_consent:false}};
     return send(state.preview);
   }
   if(key==='scope/confirm'){
     assert.equal(data.preview_id,state.preview.preview_id);assert.equal(data.acknowledge_limits,true);assert.ok(!('activate' in data));
     if(state.conflict)return send({error:'scope_changed_review_again'},409);
     state.saved={revision:(state.saved?.revision||0)+1,actor_id:101,confirmed_at_s:Date.now()/1000,document:state.preview.document,activation:'off'};return send(state.saved);
   }
   unexpected.push(request.url());return route.abort();
 });
 const page=await context.newPage(),errors=[];page.on('pageerror',e=>errors.push(String(e)));
 await page.goto(origin+'/github-app/setup');await page.getByText('Setup unavailable · checks off',{exact:true}).waitFor();assert.equal(await page.locator('#login').isVisible(),false);assert.equal(requests.filter(r=>r.path.endsWith('/repositories')).length,0);checks.push('disabled host makes no repository request');
 state.enabled=true;await page.locator('#retry').click();await page.getByRole('button',{name:'fixture/review-rs',exact:true}).waitFor();await page.getByRole('button',{name:'fixture/review-rs',exact:true}).click();await page.locator('#scope').waitFor();await page.waitForFunction(()=>!document.getElementById('preview').disabled);
 assert.equal(await page.getByRole('checkbox',{name:'Include .env',exact:true}).isDisabled(),true);assert.equal(await page.locator('#paths img').count(),0);checks.push('repository selection loads safe path names only');
 await page.getByRole('checkbox',{name:'Include src',exact:true}).check();await page.getByRole('checkbox',{name:'Include Cargo.toml',exact:true}).check();
 await page.locator('#preview').click();await page.locator('#confirmation').waitFor();assert.equal(await page.locator('#confirm').isDisabled(),true);assert.match(await page.locator('#summary').innerText(),/3 selected.*2 Rust candidates/);
 await page.locator('#acknowledge').check();await page.locator('#edit').click();await page.locator('#include').fill('src');assert.equal(await page.locator('#confirmation').isVisible(),false);checks.push('editing paths invalidates confirmation');
 await page.locator('#preview').click();await page.locator('#confirmation').waitFor();await page.locator('#acknowledge').check();await page.screenshot({path:path.join(output,'scope-review.png'),fullPage:true});await page.locator('#confirm').click();await page.locator('#complete').waitFor();assert.equal(state.saved.activation,'off');assert.equal(state.saved.document.publication_consent,false);checks.push('explicit confirmation saves inactive version');
 await page.reload();await page.getByRole('button',{name:'fixture/review-rs',exact:true}).click();await page.waitForFunction(()=>!document.getElementById('preview').disabled);assert.equal(await page.locator('#include').inputValue(),'src');assert.match(await page.locator('#saved-status').innerText(),/revision 1/);checks.push('reload restores saved revision without activating');
 await page.locator('#preview').click();await page.locator('#confirmation').waitFor();await page.locator('#acknowledge').check();state.conflict=true;await page.locator('#confirm').click();await page.locator('#error').waitFor();assert.equal(await page.locator('#preview').isDisabled(),true);assert.match(await page.locator('#error').innerText(),/Reload paths/);assert.equal(state.saved.revision,1);checks.push('conflict requires reload and fresh confirmation');
 state.conflict=false;await page.locator('#refresh').click();await page.waitForFunction(()=>!document.getElementById('preview').disabled);
 state.failed=true;await page.locator('#preview').click();await page.getByText('Setup is unavailable. Retry or reload paths to check the saved revision.',{exact:true}).waitFor().catch(async error=>{console.error({errors,visible:await page.locator('body').innerText(),requests:requests.slice(-4)});throw error;});assert.equal(await page.locator('#confirmation').isVisible(),false);state.failed=false;checks.push('failed preview cannot look saved');
 await page.setViewportSize({width:390,height:844});await page.locator('#preview').click();await page.locator('#confirmation').waitFor();assert.equal(await page.locator('#confirm-title').evaluate(e=>e===document.activeElement),true);assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));await page.screenshot({path:path.join(output,'scope-mobile.png'),fullPage:true});checks.push('narrow layout and keyboard focus');
 await page.locator('#acknowledge').check();state.denied=true;await page.locator('#confirm').click();await page.locator('#error').waitFor();assert.equal(await page.locator('#scope').isVisible(),false);assert.equal(await page.locator('#summary').innerText(),'');assert.equal(await page.locator('#selected-files').innerText(),'');checks.push('access revocation clears repository and scope UI');
 state.denied=false;await page.locator('#retry').click();await page.locator('#logout').click();await page.locator('#login').waitFor();assert.equal(await page.locator('#picker').isVisible(),false);assert.equal(await page.evaluate(()=>localStorage.length+sessionStorage.length),0);checks.push('logout clears state; no browser credential storage');
 state.authenticated=true;state.sourceAvailable=true;await page.setViewportSize({width:1280,height:900});await page.locator('#retry').click();await page.getByRole('button',{name:'fixture/review-rs',exact:true}).click();await page.locator('#source-consent').waitFor();assert.equal(await page.locator('#scan-source').isDisabled(),true);assert.equal(requests.filter(r=>r.path.endsWith('/architecture/preview')).length,0);checks.push('saved scope reload never starts source scan implicitly');
 await page.locator('#scan-consent').check();await page.locator('#scan-source').click();await page.locator('#architecture-review').waitFor();assert.equal(await page.locator('#accept-architecture').isDisabled(),true);await page.locator('[data-field="name"]').fill('Checkout API');await page.locator('#architecture-gaps-ack').check();assert.equal(await page.locator('#accept-architecture').isDisabled(),true);await page.locator('#architecture-defer').check();await page.screenshot({path:path.join(output,'architecture-review.png'),fullPage:true});await page.locator('#accept-architecture').click();await page.locator('#architecture-saved').waitFor();checks.push('explicit scan, component correction and gap acceptance remain inactive');
 await page.locator('#publication-preview').click();await page.locator('#publication-review').waitFor();assert.equal(await page.locator('#publication-confirm').isDisabled(),true);await page.locator('#publication-analysis').check();await page.locator('#publication-source').check();assert.equal(await page.locator('#publication-confirm').isDisabled(),true);await page.locator('#publication-checks').check();await page.setViewportSize({width:390,height:844});assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));await page.screenshot({path:path.join(output,'publication-mobile.png'),fullPage:true});checks.push('publication consent fits narrow screens');await page.setViewportSize({width:1280,height:900});await page.screenshot({path:path.join(output,'publication-consent.png'),fullPage:true});await page.locator('#publication-confirm').click();await page.getByText('Consent revision 1 saved. Checks remain off pending operator activation.',{exact:true}).waitFor();checks.push('three separate permissions save final consent without activation');
 await page.reload();await page.getByRole('button',{name:'fixture/review-rs',exact:true}).click();await page.getByText('Consent revision 1 saved. Checks remain off pending operator activation.',{exact:true}).waitFor();checks.push('reload reads current authenticated consent status');
 await page.locator('#publication-revoke').click();await page.getByText('Consent revoked. Jobs cancelled and existing reports withdrawn. Checks remain off.',{exact:true}).waitFor();assert.equal(state.consent,null);checks.push('revocation clears consent and reports without enabling checks');
 await page.reload();await page.getByRole('button',{name:'fixture/review-rs',exact:true}).click();await page.locator('#architecture-saved').waitFor();assert.equal(requests.filter(r=>r.path.endsWith('/architecture/preview')).length,1);await page.locator('#architecture-back').click();await page.locator('#revise').click();await page.locator('#scope').waitFor();assert.equal(await page.locator('#include').inputValue(),'src');checks.push('architecture receipt reloads and paths remain editable');
 assert.deepEqual(errors,[]);assert.deepEqual(unexpected,[]);fs.writeFileSync(path.join(output,'result.json'),JSON.stringify({status:'passed',profile:'synthetic UI contract only',checks,api_requests:requests.length,external_requests:unexpected.length,activation:'off'},null,2));console.log(JSON.stringify({status:'passed',checks:checks.length,output}));
 }finally{await browser.close();}
}
main().catch(e=>{console.error(e);process.exitCode=1;});
