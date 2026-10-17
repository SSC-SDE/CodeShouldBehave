const test=require('node:test');const assert=require('node:assert/strict');
const {boundaryPaths,inventoryPage,evidenceCommit}=require('../src/onboarding.js');
test('component provenance uses the selected context commit rather than the primary repo commit',()=>{
 const session={commit:'primary',contexts:[{commit:'library'},{commit:'backend'}]};
 assert.equal(evidenceCommit(session,{repository_index:0}),'primary');assert.equal(evidenceCommit(session,{repository_index:1}),'library');assert.equal(evidenceCommit(session,{repository_index:2}),'backend');assert.equal(evidenceCommit(session,{repository_index:3}),null);
});
test('onboarding boundary keeps explicit paths and stable distinct entries',()=>{assert.deepEqual(boundaryPaths('src\napps/web\n\n src\n'),['src','apps/web']);});
test('large inventory navigation filters and clamps pages after a query change',()=>{
 const items=Array.from({length:205},(_,i)=>({path:`src/file${i}`}));
 assert.equal(inventoryPage(items,'',1).items.length,100);assert.equal(inventoryPage(items,'',2).items.length,5);
 const narrowed=inventoryPage(items,'file204',2);assert.equal(narrowed.page,0);assert.equal(narrowed.items[0].path,'src/file204');
 assert.equal(inventoryPage(items,'missing',5).total,0);
});

const {componentMutation,providerMutation,integrationMutation,confirmationReadiness,draftMatches}=require('../src/onboarding.js');
const fixtureSession=()=>({
 id:'onboarding-review',revision:4,state:'awaiting_confirmation',commit:'a'.repeat(40),input_fingerprint:'input-1',proposal_fingerprint:'proposal-4',repository:{security_domain:'local',owner:'example',name:'repo'},include:['src'],exclude:['src/private'],contexts:[],
 inventory:{complete:true,items:[{repository_index:0,path:'src/main.rs',kind:'source',status:'indexed'}],gaps:[],unsupported:0,deferred:0,failed:0,excluded:1,sensitive_excluded:0},candidate_roots:[[0,'src']],gaps:['Runtime deployment is unverified.'],
 components:[{id:'api',name:'API',role:'backend candidate',repository_index:0,roots:['src'],stack_candidates:['Rust'],signals:[],evidence:[{repository_index:0,path:'src/main.rs',start_line:1,end_line:4,blob:'c'.repeat(40),producer_version:'fixture-parser'}],confidence:'candidate',contradictions:[],decision:'pending'}],
 providers:[{component_id:'api',provider:'Vercel',project_id:null,service_id:null,environment:'unknown',repository_index:0,source_root:'src',observed_commit:null,verified_at_ms:null,verification:'not_requested',evidence:[],notes:['Dashboard ownership is unknown.'],decision:'pending'}],
 integrations:[{id:'http',component_id:'api',kind:'external-client',name:'reqwest',reason:'An import is not a resolved network edge.',evidence:[],test_only:false,decision:'pending',answer:null}],questions:[],confirmation:null,review_defaults_active:false,
});
function refreshContext(session) {
 const active=session.components.filter(item=>['confirmed','corrected'].includes(item.decision));
 session.confirmation_readiness={components_pending:session.components.filter(item=>item.decision==='pending').length,providers_pending:session.providers.filter(item=>active.some(component=>component.id===item.component_id)&&item.decision==='pending').length,integrations_pending:session.integrations.filter(item=>active.some(component=>component.id===item.component_id)&&item.decision==='pending').length,known_gaps_required:true};
 session.confirmation_readiness.ready=session.state==='awaiting_confirmation'&&!session.confirmation_readiness.components_pending&&!session.confirmation_readiness.providers_pending&&!session.confirmation_readiness.integrations_pending;
 session.accepted_context={commit:session.commit,usable:!!active.length,components:active,providers:session.providers.filter(item=>item.decision==='confirmed'),integrations:session.integrations.filter(item=>item.decision==='include'),include:active.flatMap(item=>item.roots),exclude:session.exclude,contexts:[],contracts:[],gaps:session.gaps};
 return session;
}
test('component and hosting corrections preserve observation and never verify provider access',()=>{
 const session=fixtureSession(),component=session.components[0],mapping=session.providers[0];
 const corrected=componentMutation(session,component,{name:'Billing API',role:'backend',roots:'src\nsrc\n',decision:'confirmed'});
 assert.equal(corrected.component.decision,'corrected');assert.deepEqual(corrected.component.evidence,component.evidence);assert.deepEqual(corrected.component.signals,component.signals);assert.equal(component.name,'API');
 const hosting=providerMutation(session,mapping,0,{provider:'Self-hosted',environment:'production',source_root:'src',decision:'confirmed'});
 assert.equal(hosting.provider.decision,'corrected');assert.equal(hosting.provider.verification,'not_requested');assert.equal(hosting.provider.observed_commit,null);assert.equal(hosting.provider.project_id,null);
 assert.throws(()=>componentMutation(session,component,{name:'API',role:'backend',roots:'../secrets',decision:'confirmed'}),/repository-relative/);
 assert.throws(()=>providerMutation(session,mapping,0,{provider:'Vercel',environment:'preview',source_root:'/outside',decision:'confirmed'}),/repository-relative/);
 assert.throws(()=>componentMutation(session,component,{name:'API',role:'backend',roots:'src',decision:'pending'}),/Choose/);
 assert.throws(()=>integrationMutation(session,session.integrations[0],{decision:'include',answer:'é'.repeat(129)}),/printable bytes/);
});
test('confirmation fails closed for unknown, incomplete, unusable and unsaved contexts',()=>{
 const session=refreshContext(fixtureSession());assert.equal(confirmationReadiness(session,0,true).ready,false);
 session.components[0].decision='confirmed';session.providers[0].decision='deferred';session.integrations[0].decision='exclude';refreshContext(session);
 assert.equal(confirmationReadiness(session,0,false).ready,false);assert.equal(confirmationReadiness(session,0,true).ready,true);assert.equal(confirmationReadiness(session,1,true).ready,false);
 session.accepted_context.usable=false;assert.equal(confirmationReadiness(session,0,true).ready,false);assert.equal(confirmationReadiness({state:'awaiting_confirmation'},0,true).ready,false);
 session.state='confirmed';assert.equal(confirmationReadiness(session,0,true).ready,false);
});
test('draft conflict comparison includes evidence changes without changing display labels',()=>{
 const before={name:'API',evidence:[{blob:'old'}]},record={base:JSON.stringify(before),values:{name:'Billing'}};
 assert.equal(draftMatches(record,before),true);assert.equal(draftMatches(record,{name:'API',evidence:[{blob:'new'}]}),false);
});

// Native DOM event tests keep the save/refresh/revoke journey independent of a browser package.
const {readFileSync}=require('node:fs'),{join}=require('node:path'),vm=require('node:vm');
function uiHarness(initial=fixtureSession(),storage=new Map()) {
 class Element {
  constructor(tag='div'){this.tagName=tag;this.children=[];this.value='';this.checked=false;this.hidden=false;this.disabled=false;this.attributes={};this.textContent='';}
  append(...nodes){for(const node of nodes){node.parentNode=this;this.children.push(node);}}
  prepend(...nodes){for(const node of nodes)node.parentNode=this;this.children.unshift(...nodes);}
  replaceChildren(...nodes){this.children=[];this.append(...nodes);}
  get lastChild(){return this.children.at(-1);}
  setAttribute(key,value){this.attributes[key]=value;}
  querySelector(selector){return this.descendants().find(item=>selector.startsWith('.')?item.className?.split(' ').includes(selector.slice(1)):item.tagName===selector)||null;}
  descendants(){return this.children.flatMap(item=>[item,...item.descendants()]);}
  remove(){this.parentNode.children=this.parentNode.children.filter(item=>item!==this);}
  focus(){}scrollIntoView(){}
 }
 const elements=new Map(),get=id=>{if(!elements.has(id))elements.set(id,new Element());return elements.get(id);};
 const requests=[],context={session:refreshContext(structuredClone(initial)),failure:0,drift:null};
 const sandbox={TextEncoder,URLSearchParams,console,crypto:{randomUUID:()=> 'fixture-id'},matchMedia:()=>({matches:true}),location:{hash:'#session=onboarding-review',pathname:'/onboarding'},history:{replaceState(){}},sessionStorage:{getItem:key=>storage.get(key)||null,setItem:(key,value)=>storage.set(key,value),removeItem:key=>storage.delete(key)},document:{getElementById:get,querySelector:()=>({content:'SESSION_TOKEN_NEVER_STORED'}),createElement:tag=>new Element(tag)},fetch:async(path,options)=>{
  const body=options.body?JSON.parse(options.body):null;requests.push({path,body});if(context.failure){const status=context.failure;context.failure=0;return {ok:false,status};}
  if(path==='/api/status')return {ok:true,json:async()=>({authenticated:true,repository:'example/repo'})};
  if(body&&path.endsWith('/drift'))return {ok:true,json:async()=>structuredClone(context.drift)};
  if(body){const session=context.session;if(path.endsWith('/decisions'))session.components[0]=body.component;if(path.endsWith('/provider-decisions'))session.providers[body.provider_index]=body.provider;if(path.endsWith('/integration-decisions')){const target=session.integrations.find(item=>item.id===body.integration_id);target.decision=body.decision;target.answer=body.answer||null;}if(path.endsWith('/inspect'))session.state='awaiting_confirmation';if(path.endsWith('/activate'))session.review_defaults_active=true;if(path.endsWith('/confirm')){session.state='confirmed';session.confirmation={actor:'reviewer'};session.review_defaults_active=body.activate_defaults;}session.revision++;session.proposal_fingerprint=`proposal-${session.revision}`;refreshContext(session);}
  return {ok:true,json:async()=>structuredClone(context.session)};
 }};
 get('include').value='.';get('drift-reference').value='origin/main';vm.runInNewContext(readFileSync(join(__dirname,'../src/onboarding.js'),'utf8'),sandbox);
 const forms=()=>get('components').descendants().filter(item=>item.tagName==='form'),fields=form=>form.children.filter(item=>item.tagName==='label').map(item=>item.lastChild);
 const change=(input,value)=>{input.value=value;input.oninput();},content=element=>[element.textContent,...element.children.map(content)].join(' ');
 return {get,forms,fields,change,content,requests,context,storage};
}
const settle=async()=>{for(let count=0;count<3;count++)await new Promise(resolve=>setImmediate(resolve));};
test('review events save explicit decisions then confirm exact context without automatic PR defaults',async()=>{
 const h=uiHarness();await settle();assert.equal(h.requests.filter(item=>item.body).length,0);assert.equal(h.get('activate-defaults').checked,false);assert.equal(h.get('confirm').disabled,true);
 let form=h.forms()[0],fields=h.fields(form);h.change(fields[1],'backend');h.change(fields[3],'confirmed');form.onsubmit({preventDefault(){}});await settle();
 let saved=h.requests.find(item=>item.path.endsWith('/decisions'));assert.equal(saved.body.component.decision,'corrected');assert.equal(saved.body.component.role,'backend');assert.equal(h.get('confirm').disabled,true);
 form=h.forms()[1];fields=h.fields(form);h.change(fields[3],'confirmed');form.onsubmit({preventDefault(){}});await settle();assert.equal(h.context.session.providers[0].verification,'not_requested');
 form=h.forms()[2];fields=h.fields(form);h.change(fields[0],'exclude');h.change(fields[1],'Do not retrieve this integration.');form.onsubmit({preventDefault(){}});await settle();assert.equal(h.get('confirm').disabled,true);
 h.get('known-gaps').checked=true;h.get('known-gaps').onchange();assert.equal(h.get('confirm').disabled,false);assert.equal(h.requests.some(item=>item.path.endsWith('/confirm')),false);
 await h.get('confirm').onclick();await settle();const confirmation=h.requests.find(item=>item.path.endsWith('/confirm'));assert.equal(confirmation.body.expected_revision,7);assert.equal(confirmation.body.proposal_fingerprint,'proposal-7');assert.equal(confirmation.body.activate_defaults,false);assert.equal(confirmation.body.known_gaps_accepted,true);assert.equal(h.get('confirm').hidden,true);assert.match(h.get('confirmation-status').textContent,/defaults inactive/);
});
test('derived accepted-context gaps remain visible and require acknowledgement without session-level gaps',async()=>{
 const session=fixtureSession();session.gaps=[];session.components[0].decision='confirmed';session.providers[0].decision='deferred';session.integrations[0].decision='exclude';
 const h=uiHarness(session);await settle();const gap='A selected hosting mapping was omitted because its supporting scope is outside the accepted context.';
 h.context.session.confirmation_readiness.known_gaps_required=false;h.context.session.accepted_context.gaps=[gap];await h.get('refresh').onclick();await settle();
 assert.deepEqual(h.context.session.gaps,[]);assert.match(h.content(h.get('context-summary')),/Known context gaps · 1/);assert.ok(h.content(h.get('context-summary')).includes(gap));assert.equal(h.get('known-gaps-label').hidden,false);assert.equal(h.get('known-gaps').checked,false);assert.equal(h.get('confirm').disabled,true);
 await h.get('confirm').onclick();await settle();assert.equal(h.requests.some(item=>item.path.endsWith('/confirm')),false);assert.match(h.get('error').textContent,/explicitly accept the listed gaps/);
 h.get('known-gaps').checked=true;h.get('known-gaps').onchange();assert.equal(h.get('confirm').disabled,false);await h.get('confirm').onclick();await settle();assert.equal(h.requests.find(item=>item.path.endsWith('/confirm')).body.known_gaps_accepted,true);
});
test('reload preserves unsaved corrections; stale save blocks mutations until explicit refresh and discard',async()=>{
 const storage=new Map(),first=uiHarness(fixtureSession(),storage);await settle();first.change(first.fields(first.forms()[0])[0],'Billing API');assert.equal([...storage.values()].join('').includes('SESSION_TOKEN'),false);
 const resumed=uiHarness(fixtureSession(),storage);await settle();assert.equal(resumed.fields(resumed.forms()[0])[0].value,'Billing API');assert.match(resumed.get('draft-status').textContent,/survive reload/);
 resumed.change(resumed.fields(resumed.forms()[0])[3],'confirmed');resumed.context.failure=409;resumed.forms()[0].onsubmit({preventDefault(){}});await settle();assert.match(resumed.get('error').textContent,/Refresh/);assert.equal(resumed.get('confirm').disabled,true);assert.equal(resumed.forms()[0].querySelector('button').disabled,true);
 resumed.context.session.components[0].name='Server API';resumed.context.session.revision++;await resumed.get('refresh').onclick();await settle();assert.match(resumed.content(resumed.get('components')),/older saved decision/);assert.equal(resumed.fields(resumed.forms()[0])[0].value,'Billing API');assert.equal(resumed.forms()[0].querySelector('button').disabled,true);
 const discard=resumed.forms()[0].descendants().find(item=>item.textContent==='Discard draft');discard.onclick();assert.equal(resumed.fields(resumed.forms()[0])[0].value,'Server API');assert.equal(storage.size,1);
});
test('revocation clears private draft and visible scoped evidence',async()=>{
 const h=uiHarness();await settle();h.change(h.fields(h.forms()[0])[0],'Private draft');h.context.failure=403;await h.get('refresh').onclick();await settle();assert.equal(h.get('candidates').hidden,true);assert.equal(h.get('confirmation').hidden,true);assert.equal(h.get('components').children.length,0);assert.equal(h.get('items').children.length,0);assert.equal(h.storage.size,0);assert.match(h.get('error').textContent,/access is unavailable/);
});
test('PR defaults activate only through a separate opt-in on the final confirmation',async()=>{
 const session=fixtureSession();session.components[0].decision='confirmed';session.providers[0].decision='deferred';session.integrations[0].decision='exclude';
 const h=uiHarness(session);await settle();assert.equal(h.get('activate-defaults').checked,false);assert.equal(h.requests.filter(item=>item.body).length,0);
 h.get('activate-defaults').checked=true;h.get('known-gaps').checked=true;h.get('known-gaps').onchange();await h.get('confirm').onclick();await settle();
 assert.equal(h.requests.find(item=>item.path.endsWith('/confirm')).body.activate_defaults,true);assert.match(h.get('confirmation-status').textContent,/defaults active/);
});
test('question batches stay bounded and evidence drilldown keeps the exact source provenance',async()=>{
 const session=fixtureSession();session.questions=Array.from({length:13},(_,index)=>({id:`question-${index}`,component_id:'api',category:'component',prompt:`Review component assumption ${index}`,reason:'The scan found a qualified source observation.',decision:'pending',evidence:session.components[0].evidence}));
 const h=uiHarness(session);await settle();const displayed=()=>h.get('components').descendants().filter(item=>item.className==='question-batch');assert.equal(displayed().length,6);
 h.get('components').descendants().find(item=>item.textContent==='Next questions').onclick();assert.equal(displayed().length,6);h.get('components').descendants().find(item=>item.textContent==='Next questions').onclick();assert.equal(displayed().length,1);
 const evidence=h.get('components').descendants().find(item=>item.className==='evidence-link');evidence.onclick();const panel=h.get('components').querySelector('.source-preview');assert.match(h.content(panel),new RegExp(session.commit));assert.match(h.content(panel),/source contents are not exposed/);assert.equal(h.requests.filter(item=>item.body).length,0);
});
test('confirmed inactive context remains usable through an explicit activation action',async()=>{
 const session=fixtureSession();session.state='confirmed';session.components[0].decision='confirmed';session.providers[0].decision='deferred';session.integrations[0].decision='exclude';
 const h=uiHarness(session);await settle();assert.equal(h.get('confirm').hidden,true);assert.equal(h.get('activate').hidden,false);assert.equal(h.requests.filter(item=>item.body).length,0);
 await h.get('activate').onclick();await settle();const activation=h.requests.find(item=>item.path.endsWith('/activate'));assert.equal(activation.body.expected_revision,4);assert.equal(activation.body.proposal_fingerprint,'proposal-4');assert.equal(h.get('activate').hidden,true);assert.match(h.get('confirmation-status').textContent,/defaults active/);
});
test('a boundary creation conflict keeps preview retryable without an unavailable session refresh',async()=>{
 const h=uiHarness();await settle();h.get('new').onclick();h.get('reference').value='origin/main';h.context.failure=409;h.get('boundary-form').onsubmit({preventDefault(){}});await settle();
 assert.match(h.get('error').textContent,/boundary could not be saved/);assert.doesNotMatch(h.get('error').textContent,/Refresh and review/);assert.equal(h.get('preview').disabled,false);assert.equal(h.get('session').hidden,true);
});
test('empty candidate navigation remains explicit and cannot confirm a usable architecture',async()=>{
 const session=fixtureSession();session.components=[];session.providers=[];session.integrations=[];const h=uiHarness(session);await settle();
 assert.equal(h.get('component-nav').children.length,0);assert.equal(h.forms().length,0);assert.match(h.content(h.get('components')),/No component boundary was detected/);h.get('known-gaps').checked=true;h.get('known-gaps').onchange();assert.equal(h.get('confirm').disabled,true);assert.match(h.get('confirmation-status').textContent,/no usable approved boundary/);
});
const {driftAssessment}=require('../src/onboarding.js');
function fixtureDrift(session,status='unchanged') {return {version:'csb-onboarding-drift-v1',session_id:session.id,session_revision:session.revision,source_commit:session.commit,target_commit:'b'.repeat(40),reuse_allowed:status==='unchanged',status,affected_components:[],gaps:[],defaults_changed:false,scope_expanded:false,reconfirmation_required:status!=='unchanged'};}
function confirmedFixture(active=false){const session=fixtureSession();session.state='confirmed';session.review_defaults_active=active;session.components[0].decision='confirmed';session.providers[0].decision='deferred';session.integrations[0].decision='exclude';return session;}
test('body-only unchanged context check stays read-only and does not assert unchanged behavior',async()=>{
 const h=uiHarness(confirmedFixture());await settle();h.context.drift=fixtureDrift(h.context.session);h.get('drift-form').onsubmit({preventDefault(){}});await settle();
 const request=h.requests.find(item=>item.path.endsWith('/drift'));assert.equal(request.body.reference,'origin/main');assert.equal(request.body.expected_revision,4);assert.equal(request.body.repository,undefined);assert.equal(h.context.session.revision,4);assert.equal(h.context.session.review_defaults_active,false);assert.equal(h.requests.filter(item=>item.path.endsWith('/confirm')||item.path.endsWith('/activate')).length,0);
 assert.match(h.get('drift-status').textContent,/can be reused/);assert.match(h.content(h.get('drift-result')),/PR behavior still requires separate/);assert.match(h.content(h.get('drift-result')),new RegExp('b'.repeat(40)));
});
test('active changed or incomplete context requires a new boundary without changing saved defaults',async()=>{
 const h=uiHarness(confirmedFixture(true));await settle();h.context.drift=fixtureDrift(h.context.session,'changed');h.context.drift.affected_components=[{component_id:'api',reasons:['Database context signatures changed; endpoints remain unverified.'],evidence:[{repository_index:0,path:'src/main.rs',blob:'d'.repeat(40),start_line:1,end_line:1,producer_version:'csb-onboarding-drift-v1',commit:'b'.repeat(40),side:'target'}]}];h.get('drift-reference').value='proposal';h.get('drift-form').onsubmit({preventDefault(){}});await settle();
 const request=h.requests.find(item=>item.path.endsWith('/drift'));assert.equal(request.body.repository.owner,'example');assert.equal(request.body.reference,'proposal');assert.match(h.get('drift-status').textContent,/require confirmation/);assert.match(h.content(h.get('drift-result')),/Choose New boundary/);assert.match(h.content(h.get('drift-result')),/Blob signature provenance/);assert.match(h.content(h.get('drift-result')),/target/);assert.equal(h.context.session.review_defaults_active,true);assert.equal(h.context.session.revision,4);
 h.context.drift=fixtureDrift(h.context.session,'incomplete');h.context.drift.gaps=['The target scan exceeded its bounded coverage.'];h.get('drift-form').onsubmit({preventDefault(){}});await settle();assert.match(h.get('drift-status').textContent,/incomplete/);assert.match(h.content(h.get('drift-result')),/bounded coverage/);assert.equal(h.context.session.review_defaults_active,true);
});
test('context drift rejects wrong-session results or claims of changed defaults or expanded scope',()=>{
 const session=confirmedFixture();for(const replacement of [{session_id:'other'},{session_revision:8},{source_commit:'other'},{defaults_changed:true},{scope_expanded:true}])assert.throws(()=>driftAssessment({...fixtureDrift(session),...replacement},session),/does not match/);
 const inconsistent={...fixtureDrift(session),reuse_allowed:false};assert.match(driftAssessment(inconsistent,session).instruction,/New boundary/);
});
