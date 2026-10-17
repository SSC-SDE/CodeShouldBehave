'use strict';
function boundaryPaths(value) {return [...new Set(value.split(/\r?\n/).map(item=>item.trim()).filter(Boolean))];}
function inventoryPage(items,query,page,size=100) {
  const filtered=items.filter(item=>item.path.toLowerCase().includes(query.toLowerCase()));
  const count=Math.max(1,Math.ceil(filtered.length/size)), selected=Math.min(Math.max(0,page),count-1);
  return {items:filtered.slice(selected*size,(selected+1)*size),page:selected,count,total:filtered.length};
}
function evidenceCommit(session,evidence) {return evidence.repository_index===0?session.commit:session.contexts?.[evidence.repository_index-1]?.commit||null;}
function printableLabel(value,label) {
  const result=value.trim();if(!result||new TextEncoder().encode(result).length>256||/[\x00-\x1f\x7f]/.test(result))throw new Error(`${label} requires 1-256 printable bytes.`);return result;
}
function mutationContext(session) {return {security_domain:session.repository.security_domain,expected_revision:session.revision};}
function componentMutation(session,component,values) {
  if(!['confirmed','rejected','deferred'].includes(values.decision))throw new Error('Choose include, exclude or defer for this component.');
  const name=printableLabel(values.name,'Component name'),role=printableLabel(values.role,'Component role'),roots=boundaryPaths(values.roots);
  if(!roots.length||roots.some(root=>root!=='.'&&(/^[/\\]|\\|(^|\/)\.\.?($|\/)/.test(root))))throw new Error('Use repository-relative roots within the approved scan boundary.');
  const corrected=name!==component.name||role!==component.role||JSON.stringify(roots)!==JSON.stringify(component.roots);
  return {...mutationContext(session),component:{...component,name,role,roots,decision:values.decision==='confirmed'&&(corrected||component.decision==='corrected')?'corrected':values.decision}};
}
function providerMutation(session,mapping,index,values) {
  if(!['confirmed','rejected','deferred'].includes(values.decision))throw new Error('Choose include, exclude or defer for this hosting label.');
  const provider=printableLabel(values.provider,'Hosting provider'),environment=printableLabel(values.environment,'Environment'),source_root=printableLabel(values.source_root,'Source root');
  if(source_root!=='.'&&(/^[/\\]|\\|(^|\/)\.\.?($|\/)/.test(source_root)))throw new Error('Use a repository-relative hosting root within the approved scan boundary.');
  const corrected=provider!==mapping.provider||environment!==mapping.environment||source_root!==mapping.source_root;
  return {...mutationContext(session),provider_index:index,provider:{...mapping,provider,environment,source_root,decision:values.decision==='confirmed'&&(corrected||mapping.decision==='corrected')?'corrected':values.decision}};
}
function integrationMutation(session,integration,values) {
  if(!['include','exclude','defer'].includes(values.decision))throw new Error('Choose include, exclude or defer for this dependency context.');
  const answer=values.answer?.trim()||'';if(answer)printableLabel(answer,'Reviewer explanation');
  return {...mutationContext(session),integration_id:integration.id,decision:values.decision,...(answer?{answer}:{})};
}
function knownGapsRequired(session) {return !!(session?.confirmation_readiness?.known_gaps_required||session?.accepted_context?.gaps?.length);}
function confirmationReadiness(session,unsaved=0,knownGapsAccepted=false) {
  const readiness=session?.confirmation_readiness;
  if(!readiness)return {ready:false,reason:'The server has not supplied confirmation readiness. Refresh the session.'};
  if(session.state==='confirmed')return {ready:false,reason:'This architecture context is confirmed and immutable.'};
  if(session.state!=='awaiting_confirmation')return {ready:false,reason:'Review the architecture context before confirming.'};
  const pending=readiness.components_pending+readiness.providers_pending+readiness.integrations_pending;
  if(pending)return {ready:false,reason:`${readiness.components_pending} components · ${readiness.providers_pending} hosting labels · ${readiness.integrations_pending} dependencies still require a decision.`};
  if(unsaved)return {ready:false,reason:`Save or discard ${unsaved} draft${unsaved===1?'':'s'} before confirming.`};
  if(knownGapsRequired(session)&&!knownGapsAccepted)return {ready:false,reason:'Review and explicitly accept the listed gaps before confirming.'};
  if(!readiness.ready||!session.accepted_context?.usable)return {ready:false,reason:'The selected context has no usable approved boundary. Include a scoped component or start a new boundary.'};
  return {ready:true,reason:'All included context decisions are saved. Confirm this exact commit and boundary when ready.'};
}
function draftMatches(record,base) {return record?.base===JSON.stringify(base);}
function driftAssessment(result,session) {
  if(!result||result.version!=='csb-onboarding-drift-v1'||result.session_id!==session.id||result.session_revision!==session.revision||result.source_commit!==session.commit||!/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(result.target_commit)||result.defaults_changed!==false||result.scope_expanded!==false||!['unchanged','changed','incomplete'].includes(result.status))throw new Error('The drift result does not match this saved context. Refresh and check again.');
  if(result.status==='unchanged'&&result.reuse_allowed===true&&result.reconfirmation_required===false)return {label:'Confirmed context can be reused.',instruction:'No context boundary change was detected. PR behavior still requires separate source and structural review.'};
  return {label:result.status==='incomplete'?'Context comparison is incomplete.':'Context changes require confirmation.',instruction:'Choose New boundary and scan the target commit. Review and confirm a new context before using it for PR reviews. Existing saved defaults remain unchanged.'};
}
if(typeof module!=='undefined')module.exports={boundaryPaths,inventoryPage,evidenceCommit,componentMutation,providerMutation,integrationMutation,confirmationReadiness,draftMatches,driftAssessment};
if(typeof document!=='undefined') {
  const el=id=>document.getElementById(id),token=document.querySelector('meta[name=csb-local-session]')?.content||'';
  const text=(tag,value)=>{const node=document.createElement(tag);node.textContent=value;return node;};
  let repository=null,session=null,busy=false,page=0,stop=false,stale=false,selectedComponent=null,drafts={},draftStorageAvailable=true,questionPages={},driftResult=null;
  const draftKey=()=>session?`csb-onboarding-draft:${session.id}:${session.input_fingerprint}`:null;
  function persistDrafts() {try{if(draftKey())sessionStorage.setItem(draftKey(),JSON.stringify({drafts,selectedComponent}));}catch{draftStorageAvailable=false;}}
  function restoreDrafts() {drafts={};try{const saved=JSON.parse(sessionStorage.getItem(draftKey())||'null');if(saved&&typeof saved==='object'){drafts=saved.drafts||{};selectedComponent=saved.selectedComponent||null;}}catch{draftStorageAvailable=false;}}
  function clearDraft(key) {delete drafts[key];persistDrafts();}
  function clearPrivateDrafts() {try{if(draftKey())sessionStorage.removeItem(draftKey());}catch{}drafts={};selectedComponent=null;}
  async function api(path,body) {
    const response=await fetch(path,{method:body?'POST':'GET',cache:'no-store',headers:{Authorization:`Bearer ${token}`,'X-CSB-Session':token,'Content-Type':'application/json'},body:body?JSON.stringify(body):undefined});
    if(!response.ok) {
      if([401,403,404].includes(response.status)){clearPrivateDrafts();session=null;driftResult=null;render();}
      if(response.status===409&&session)stale=true;
      const messages={400:'This correction is invalid or outside the approved boundary. Check the fields and source evidence.',401:'GitHub authorization is required. Reconnect from the landing page.',403:'Repository or confirmation access is unavailable. Reconnect with the required repository access.',404:'Repository access or this session is unavailable.',409:session?'The saved session changed. Your drafts are retained. Refresh and review conflicting changes before continuing.':'The boundary could not be saved because its repository context changed. Reload and preview the current reference again.',503:'The onboarding adapter is unavailable. Start the updated local app.'};
      throw new Error(messages[response.status]||`Request failed (${response.status}).`);
    }return response.json();
  }
  async function work(action) {
    if(busy)return;busy=true;el('error').hidden=true;render();
    try{await action();}catch(error){el('error').textContent=error.message;el('error').hidden=false;}finally{busy=false;render();}
  }
  async function mutate(action,body,key) {
    session=await api(`/v1/onboarding/${encodeURIComponent(session.id)}/${action}`,body);
    if(key)clearDraft(key);el('known-gaps').checked=false;el('progress').textContent=`Saved revision ${session.revision}. Context decisions remain separate from final confirmation.`;
  }
  function evidenceList(items) {
    const list=document.createElement('ul');list.className='evidence-list';
    for(const item of items||[]) {
      const row=document.createElement('li'),link=text('button',`${item.path}:${item.start_line}-${item.end_line}`);link.type='button';link.className='evidence-link';
      link.onclick=()=>showEvidence(item);row.append(link,text('span',`Repository ${item.repository_index+1} · commit ${evidenceCommit(session,item)||'unavailable'} · blob ${item.blob.slice(0,12)} · ${item.producer_version}`));row.lastChild.className='provenance';list.append(row);
    }return list;
  }
  function showEvidence(item) {
    const current=el('components').querySelector('.source-preview');if(current)current.remove();
    const panel=document.createElement('div');panel.className='source-preview';panel.tabIndex=-1;panel.setAttribute('role','region');panel.setAttribute('aria-label','Exact source evidence');
    const inventory=session.inventory?.items.find(entry=>entry.repository_index===item.repository_index&&entry.path===item.path);
    panel.append(text('h4',`${item.path}:${item.start_line}-${item.end_line}`),text('p',`Exact commit ${evidenceCommit(session,item)||'unavailable'} · blob ${item.blob} · ${item.producer_version}`),text('p',inventory?`Scoped inventory: ${inventory.kind} · ${inventory.status}. Evidence identifies a committed span; source contents are not exposed by this inventory endpoint.`:'The evidence source is absent from the permitted inventory. It cannot establish an architecture fact.'));
    const dismiss=text('button','Close evidence');dismiss.type='button';dismiss.className='secondary';dismiss.onclick=()=>panel.remove();panel.append(dismiss);el('components').prepend(panel);panel.focus();
  }
  function detail(summary,items) {const node=document.createElement('details');node.append(text('summary',summary),evidenceList(items));return node;}
  function editable() {return session?.state==='awaiting_confirmation'&&!busy&&!stale;}
  function labelInput(form,label,type,value,change) {
    const wrapper=text('label',label),input=document.createElement(type==='textarea'?'textarea':type==='select'?'select':'input');
    if(type!=='textarea'&&type!=='select'){input.type='text';input.maxLength=256;input.autocomplete='off';}
    if(type==='textarea'){input.rows=3;input.spellcheck=false;}
    input.disabled=!editable();input.value=value||'';input.oninput=change;wrapper.append(input);form.append(wrapper);return input;
  }
  function decisionSelect(form,label,value,options,change) {
    const select=labelInput(form,label,'select',value,change);
    for(const [decision,name] of options){const option=text('option',name);option.value=decision;select.append(option);}select.value=value;return select;
  }
  function reviewForm(key,base,initial,fields,action,payload) {
    const form=document.createElement('form');form.className='review-form';
    const record=drafts[key],conflict=!!record&&!draftMatches(record,base),values={...initial,...record?.values};
    let discard;const controls={},update=()=>{const changes=Object.fromEntries(Object.entries(controls).map(([name,input])=>[name,input.value]));drafts[key]={base:record?.base||JSON.stringify(base),values:changes};persistDrafts();if(discard)discard.disabled=false;updateConfirmation();};
    for(const [name,label,type] of fields){controls[name]=type==='decision'?decisionSelect(form,label,values[name],action==='integration-decisions'?[['pending','Choose a decision'],['include','Include in review context'],['exclude','Exclude from review context'],['defer','Defer / keep unknown']]:[['pending','Choose a decision'],['confirmed','Include in architecture context'],['rejected','Exclude from architecture context'],['deferred','Defer / keep unknown']],update):labelInput(form,label,type,values[name],update);}
    if(conflict){const note=text('p','This draft was based on an older saved decision. Compare it with the current evidence and discard it before editing this item again.');note.className='draft-conflict';form.append(note);for(const control of Object.values(controls))control.disabled=true;}
    const actions=document.createElement('div');actions.className='actions';const save=text('button','Save decision');save.type='submit';save.disabled=!editable()||conflict;actions.append(save);
    discard=text('button','Discard draft');discard.type='button';discard.className='secondary';discard.disabled=busy||!record;discard.onclick=()=>{clearDraft(key);render();};actions.append(discard);form.append(actions);
    form.onsubmit=event=>{event.preventDefault();const current=Object.fromEntries(Object.entries(controls).map(([name,input])=>[name,input.value]));work(async()=>mutate(action,payload(current),key));};return form;
  }
  function renderInventory() {
    const inventory=session?.inventory;el('inventory').hidden=!inventory;if(!inventory)return;
    el('inventory-count').textContent=`${inventory.items.length} paths`;
    el('coverage').textContent=`${inventory.items.filter(item=>item.status==='indexed').length} indexed · ${inventory.unsupported} unsupported · ${inventory.deferred} deferred · ${inventory.failed} failed · ${inventory.excluded} outside scope · ${inventory.sensitive_excluded} sensitive paths excluded`;
    el('inventory-limit').textContent=inventory.complete?'Inventory complete. Architecture proposals and HLD/LLD are not generated by this inventory step.':`Saved progress · repository ${inventory.repository_index+1} · tree offset ${inventory.offset}. Resume keeps the same commit and boundary.`;
    el('roots').replaceChildren(...(session.candidate_roots||[]).map(([index,root])=>text('li',`Repository ${index+1} · ${root} · unconfirmed manifest boundary`)));
    const gaps=[...(inventory.gaps||[])];if(inventory.deferred)gaps.push('Deferred files exceed object, path-count or metadata-size limits. Their architecture remains unknown.');if(inventory.unsupported)gaps.push('Unsupported formats, binary objects, symlinks and submodules are not analyzed.');
    el('gaps').replaceChildren(...gaps.map(value=>text('li',value)));
    const result=inventoryPage(inventory.items,el('filter').value,page);page=result.page;
    el('items').replaceChildren(...result.items.map(item=>{const row=document.createElement('tr');for(const value of [item.path,item.kind,item.variable_names?.length?`${item.status} · names: ${item.variable_names.join(', ')}`:item.status])row.append(text('td',value));return row;}));
    el('page-count').textContent=`Page ${page+1} / ${result.count} · ${result.total} matching paths`;el('previous').disabled=page===0;el('next').disabled=page+1>=result.count;
  }
  function renderQuestions(component,card) {
    const questions=(session.questions||[]).filter(question=>question.component_id===component.id);if(!questions.length)return;
    const current=Math.min(questionPages[component.id]||0,Math.max(0,Math.ceil(questions.length/6)-1));questionPages[component.id]=current;
    const batch=document.createElement('details');batch.open=true;batch.append(text('summary',`Confirmation questions · batch ${current+1} / ${Math.ceil(questions.length/6)}`));
    for(const question of questions.slice(current*6,current*6+6)){const row=document.createElement('div');row.className='question-batch';const inherited=question.category!=='component'&&['rejected','deferred'].includes(component.decision);row.append(text('h4',question.prompt),text('p',question.reason),text('p',inherited?'The component is excluded or deferred. This related context does not require an additional decision.':`Saved decision: ${question.decision}. Use the ${question.category==='component'?'component':question.category==='provider'?'hosting':'dependency'} controls below.`),evidenceList(question.evidence));batch.append(row);}
    if(questions.length>6){const actions=document.createElement('div');actions.className='actions';for(const [label,offset,disabled] of [['Previous questions',-1,current===0],['Next questions',1,(current+1)*6>=questions.length]]){const button=text('button',label);button.type='button';button.className='secondary';button.disabled=disabled;button.onclick=()=>{questionPages[component.id]=current+offset;renderCandidates();};actions.append(button);}batch.append(actions);}card.append(batch);
  }
  function renderCandidates() {
    el('candidates').hidden=!session?.inventory?.complete;if(!session?.inventory?.complete)return;
    const components=session.components||[];el('detect').disabled=busy||!['scanning','partial'].includes(session.state);el('detect').hidden=components.length>0||['proposal_ready','awaiting_confirmation','confirmed'].includes(session.state);
    el('inspect').hidden=session.state!=='proposal_ready';el('inspect').disabled=busy||stale;
    el('candidate-status').textContent=session.state==='confirmed'?'Confirmed context. Source observations remain qualified; hosting labels may be unverified.':session.state==='awaiting_confirmation'?'Review each boundary, then save explicit component, hosting and dependency decisions. Names and roots can be corrected within the scanned scope.':session.state==='proposal_ready'?'Source-qualified candidates are ready. Review architecture context to make explicit decisions.':'Scan complete. Inspect bounded manifest, source and static hosting evidence to generate candidates.';
    if(!components.some(component=>component.id===selectedComponent))selectedComponent=components[0]?.id||null;
    el('component-nav').replaceChildren(...components.map(component=>{
      const questions=(session.questions||[]).filter(question=>question.component_id===component.id),pending=questions.filter(question=>question.decision==='pending'&&(question.category==='component'||!['rejected','deferred'].includes(component.decision))).length;
      const button=document.createElement('button');button.type='button';button.className='component-choice';button.setAttribute('aria-current',String(component.id===selectedComponent));button.append(text('strong',component.name),text('span',`${component.role} · ${component.decision}`),text('span',component.roots.join(', ')),text('span',`${pending} / ${questions.length} questions unresolved`));button.onclick=()=>{selectedComponent=component.id;persistDrafts();renderCandidates();};return button;
    }));
    el('components').replaceChildren();const component=components.find(item=>item.id===selectedComponent);if(!component){el('components').append(text('p','No component boundary was detected. The architecture remains unknown; adjust the approved boundary to inspect supported evidence.'));return;}
    const card=document.createElement('article');card.className='component';const heading=document.createElement('div');heading.className='section-head';heading.append(text('h3',component.name),text('span',component.decision));heading.lastChild.className='badge decision-badge';card.append(heading,text('p',`${component.role} · stack evidence: ${component.confidence}`),text('p',`Repository ${component.repository_index+1} · roots: ${component.roots.join(', ')}`),text('p',component.stack_candidates.join(' / ')||'Stack unknown'));
    renderQuestions(component,card);
    if(session.state==='awaiting_confirmation')card.append(reviewForm(`component:${component.id}`,{name:component.name,role:component.role,roots:component.roots,decision:component.decision,evidence:component.evidence,signals:component.signals},{name:component.name,role:component.role,roots:component.roots.join('\n'),decision:component.decision==='corrected'?'confirmed':component.decision},[['name','Component name','text'],['role','Architecture role','text'],['roots','Repository roots, one per line','textarea'],['decision','Component context decision','decision']],'decisions',values=>componentMutation(session,component,values)));
    const hosting=document.createElement('details');hosting.append(text('summary','Hosting context'));const providers=(session.providers||[]).map((mapping,index)=>({mapping,index})).filter(({mapping})=>mapping.component_id===component.id);
    if(!providers.length)hosting.append(text('p','Hosting unknown. No supported provider mapping was found. No provider choice is forced.'));
    for(const {mapping,index} of providers){const row=document.createElement('div');row.className='decision-detail';row.append(text('h4',`${mapping.provider} · ${mapping.environment} · ${mapping.decision}`),text('p',`Root: ${mapping.source_root} · provider verification: ${mapping.verification}. Including a label does not verify hosting or authorize provider access.`));for(const value of mapping.notes||[])row.append(text('p',value));row.append(evidenceList(mapping.evidence));if(session.state==='awaiting_confirmation')row.append(reviewForm(`provider:${index}`,{provider:mapping.provider,environment:mapping.environment,source_root:mapping.source_root,decision:mapping.decision,evidence:mapping.evidence,notes:mapping.notes},{provider:mapping.provider,environment:mapping.environment,source_root:mapping.source_root,decision:mapping.decision==='corrected'?'confirmed':mapping.decision},[['provider','Hosting label','text'],['environment','Environment label','text'],['source_root','Repository source root','text'],['decision','Hosting context decision','decision']],'provider-decisions',values=>providerMutation(session,mapping,index,values)));hosting.append(row);}card.append(hosting);
    const integrations=(session.integrations||[]).filter(item=>item.component_id===component.id);const dependencies=document.createElement('details');dependencies.append(text('summary',`Dependency context · ${integrations.length}`));
    if(!integrations.length)dependencies.append(text('p','No supported dependency context candidates. Source observations remain available below.'));
    for(const integration of integrations){const row=document.createElement('div');row.className='decision-detail';row.append(text('h4',`${integration.name} · ${integration.kind} · ${integration.decision}`),text('p',integration.reason),text('p',integration.test_only?'Test/example scope only. This does not establish a production dependency.':'This is a source-qualified context candidate. Including it does not resolve an edge or retrieve another repository.'),evidenceList(integration.evidence));if(session.state==='awaiting_confirmation')row.append(reviewForm(`integration:${integration.id}`,{decision:integration.decision,answer:integration.answer||'',evidence:integration.evidence,reason:integration.reason},{decision:integration.decision,answer:integration.answer||''},[['decision','Dependency context decision','decision'],['answer','Reviewer explanation (optional, no credentials or secret values)','text']],'integration-decisions',values=>integrationMutation(session,integration,values)));dependencies.append(row);}card.append(dependencies);
    if(component.signals?.length){const observations=document.createElement('details');observations.append(text('summary',`Roles and dependencies · ${component.signals.length} observations`));for(const signal of component.signals){const row=document.createElement('div');row.className='decision-detail';row.append(text('strong',`${signal.name} · ${signal.kind} · ${signal.test_only?'test/example scope':'candidate'}`),text('p',signal.reason));const item=component.evidence[signal.evidence_index];if(item)row.append(evidenceList([item]));else row.append(text('p','Evidence unavailable. This observation cannot establish an architecture fact.'));observations.append(row);}card.append(observations);}
    card.append(detail(`Evidence · ${component.evidence.length} source${component.evidence.length===1?'':'s'}`,component.evidence));for(const value of component.contradictions||[])card.append(text('p',value));el('components').append(card);
  }
  function updateConfirmation() {
    const records=Object.keys(drafts).length;el('draft-status').textContent=stale?'Session changed. Refresh to compare your saved decisions and retained drafts.':records?`${records} unsaved draft${records===1?'':'s'}. ${draftStorageAvailable?'Drafts survive reload in this browser tab.':'Browser draft storage is unavailable; save before leaving.'}`:'Decisions are saved on the server only when you choose Save decision.';
    const result=confirmationReadiness(session,records,el('known-gaps').checked);el('confirmation-status').textContent=session?.state==='confirmed'?`Confirmed revision ${session.revision} · PR defaults ${session.review_defaults_active?'active':'inactive'} · ${session.confirmation?.actor||'authorized reviewer'}`:result.reason;
    el('activate').hidden=session?.state!=='confirmed'||session?.review_defaults_active;el('activate').disabled=busy||stale||session?.state!=='confirmed';el('confirm').disabled=busy||stale||!result.ready;el('confirm').hidden=session?.state==='confirmed';el('known-gaps-label').hidden=!knownGapsRequired(session)||session?.state==='confirmed';el('activate-defaults-label').hidden=session?.state==='confirmed';el('known-gaps').disabled=busy||stale;el('activate-defaults').disabled=busy||stale;el('return-workspace').hidden=session?.state!=='confirmed';
  }
  function renderConfirmation() {
    el('confirmation').hidden=!['awaiting_confirmation','confirmed'].includes(session?.state);if(el('confirmation').hidden)return;
    const context=session.accepted_context;el('context-summary').replaceChildren();if(context){const matrix=document.createElement('div');matrix.className='context-matrix';for(const [name,items,summary] of [['Included components',context.components,item=>item.name],['Dependency context',context.integrations,item=>`${item.name} · ${item.decision}`],['Hosting labels',context.providers,item=>`${item.provider} · ${item.verification}`]]){const column=document.createElement('div');column.append(text('strong',`${name} · ${(items||[]).length}`),text('span',(items||[]).map(summary).join(', ')||'None'));matrix.append(column);}el('context-summary').append(matrix,text('p',`Exact commit: ${context.commit}`),text('p',`Included roots: ${(context.include||[]).join(', ')||'None'}`),text('p',`Excluded roots: ${(context.exclude||[]).join(', ')||'None'}`),text('p',`Related repositories: ${(context.contexts||[]).length} · contracts: ${(context.contracts||[]).length}. A new repository or broader path requires a separately approved scan boundary.`));
      if(context.gaps?.length){const gaps=document.createElement('details');gaps.append(text('summary',`Known context gaps · ${context.gaps.length}`));for(const gap of context.gaps)gaps.append(text('p',gap));el('context-summary').append(gaps);}}
    else el('context-summary').append(text('p','Accepted context preview is unavailable. Refresh the updated service before confirming.'));updateConfirmation();renderDrift();
  }
  function renderDrift() {
    el('drift').hidden=session?.state!=='confirmed';el('drift-check').disabled=busy||stale||session?.state!=='confirmed';el('drift-reference').disabled=busy||stale;el('drift-result').replaceChildren();
    if(!driftResult){el('drift-status').textContent='';return;}
    let assessment;try{assessment=driftAssessment(driftResult,session);}catch{driftResult=null;el('drift-status').textContent='The saved context changed. Check the target reference again.';return;}el('drift-status').textContent=assessment.label;
    el('drift-result').append(text('p',`Saved commit: ${driftResult.source_commit}`),text('p',`Target commit: ${driftResult.target_commit}`),text('p',assessment.instruction),text('p','Defaults changed: no. This read-only comparison does not confirm or activate another context.'));
    for(const affected of driftResult.affected_components||[]){const section=document.createElement('details');const component=session.components.find(item=>item.id===affected.component_id);section.append(text('summary',component?.name||affected.component_id));for(const reason of affected.reasons||[])section.append(text('p',reason));const sources=document.createElement('ul');sources.className='evidence-list';for(const item of affected.evidence||[]){const row=text('li',`${item.side} · ${item.path} · blob ${item.blob} · ${item.producer_version}`);row.append(text('p',`Exact commit ${item.commit}. Blob signature provenance; this context check does not identify specific changed lines or prove a semantic edge.`));sources.append(row);}section.append(sources);el('drift-result').append(section);}
    if(driftResult.gaps?.length){const gaps=document.createElement('details');gaps.append(text('summary',`Comparison gaps · ${driftResult.gaps.length}`));for(const gap of driftResult.gaps)gaps.append(text('p',gap));el('drift-result').append(gaps);}
  }
  function render() {
    el('setup').hidden=!!session;el('session').hidden=!session;el('preview').disabled=busy||!repository;
    if(!session){el('inventory').hidden=true;el('candidates').hidden=true;el('confirmation').hidden=true;el('drift').hidden=true;for(const id of ['items','roots','gaps','components','component-nav','detection-gaps','context-summary','drift-result'])el(id).replaceChildren();for(const id of ['session-repo','commit','approved-include','approved-exclude','session-id','progress','drift-status'])el(id).textContent='';return;}
    el('state').textContent=session.state;el('session-title').textContent=session.inventory?'Saved scan context':'Review before scanning';el('session-repo').textContent=`${session.repository.owner}/${session.repository.name}`;el('commit').textContent=session.commit;el('approved-include').textContent=session.include.join(', ');el('approved-exclude').textContent=session.exclude.join(', ')||'None';el('session-id').textContent=session.id;
    el('scan').disabled=busy||stale||session.inventory?.complete||!['draft','scanning','partial'].includes(session.state);el('scan').textContent=session.inventory?'Continue scan':'Approve and scan this boundary';el('pause').disabled=!busy||!session.inventory||session.inventory.complete;el('refresh').disabled=busy;el('cancel').disabled=busy||!['draft','scanning','partial','proposal_ready','awaiting_confirmation'].includes(session.state);el('new').disabled=busy;
    renderInventory();renderCandidates();renderConfirmation();updateConfirmation();
  }
  el('boundary-form').onsubmit=event=>{event.preventDefault();work(async()=>{
    const include=boundaryPaths(el('include').value),exclude=boundaryPaths(el('exclude').value);if(!include.length)throw new Error('Include at least one path.');
    session=await api('/v1/onboarding',{id:`onboarding-${crypto.randomUUID()}`,reference:el('reference').value.trim(),scope_version:`scope-${crypto.randomUUID()}`,sample_env_names:el('sample-env-names').checked,scope:{primary_repository:repository,primary_include:include,primary_exclude:exclude,contexts:[],contracts:[],export_allowed:false,model_destinations:[]}});drafts={};stale=false;history.replaceState(null,'',`#session=${encodeURIComponent(session.id)}`);el('progress').textContent='Exact commit resolved. No repository content has been scanned. Approve this boundary to begin.';
  });};
  el('scan').onclick=()=>work(async()=>{stop=false;do{session=await api(`/v1/onboarding/${encodeURIComponent(session.id)}/scan`,mutationContext(session));el('progress').textContent=`Saved revision ${session.revision} · ${session.inventory.items.length} inventory paths`;render();}while(!stop&&!session.inventory.complete);});
  el('pause').onclick=()=>{stop=true;el('progress').textContent='Pausing after the current bounded chunk…';};
  el('detect').onclick=()=>work(async()=>{await mutate('detect',mutationContext(session));el('progress').textContent=`Saved ${session.components.length} unconfirmed component${session.components.length===1?'':'s'}. Evidence and limits are listed below.`;el('candidates').scrollIntoView({behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'instant':'smooth',block:'start'});});
  el('inspect').onclick=()=>work(async()=>{await mutate('inspect',mutationContext(session));el('progress').textContent='Review the proposed boundaries. No context has been confirmed or activated.';});
  el('refresh').onclick=()=>work(async()=>{session=await api(`/v1/onboarding/${encodeURIComponent(session.id)}?security_domain=${encodeURIComponent(session.repository.security_domain)}`);stale=false;el('known-gaps').checked=false;el('progress').textContent='Saved decisions reloaded. Retained drafts are compared with the current version.';});
  el('cancel').onclick=()=>work(async()=>{stop=true;session=await api(`/v1/onboarding/${encodeURIComponent(session.id)}/cancel`,mutationContext(session));el('progress').textContent='Cancelled. Saved decisions remain available in session history; cancellation does not confirm them.';});
  el('new').onclick=()=>{persistDrafts();session=null;drafts={};selectedComponent=null;driftResult=null;stale=false;page=0;el('known-gaps').checked=false;el('activate-defaults').checked=false;history.replaceState(null,'',location.pathname);render();};
  el('filter').oninput=()=>{page=0;renderInventory();};el('previous').onclick=()=>{page--;renderInventory();};el('next').onclick=()=>{page++;renderInventory();};el('known-gaps').onchange=updateConfirmation;
  el('confirm').onclick=()=>work(async()=>{const readiness=confirmationReadiness(session,Object.keys(drafts).length,el('known-gaps').checked);if(!readiness.ready)throw new Error(readiness.reason);await mutate('confirm',{...mutationContext(session),proposal_fingerprint:session.proposal_fingerprint,known_gaps_accepted:el('known-gaps').checked,activate_defaults:el('activate-defaults').checked});drafts={};persistDrafts();el('progress').textContent='Architecture and review context confirmed for the displayed commit and scope.';});
  el('activate').onclick=()=>work(async()=>{if(session?.state!=='confirmed'||session.review_defaults_active)throw new Error('Refresh the confirmed context before activating it.');await mutate('activate',{...mutationContext(session),proposal_fingerprint:session.proposal_fingerprint});el('progress').textContent='The displayed confirmed context is now active for PR reviews.';});
  el('drift-form').onsubmit=event=>{event.preventDefault();work(async()=>{if(session?.state!=='confirmed')throw new Error('Confirm the repository context before checking drift.');driftResult=null;el('drift-status').textContent='Checking the requested context…';el('drift-result').replaceChildren();const result=await api(`/v1/onboarding/${encodeURIComponent(session.id)}/drift`,{...mutationContext(session),reference:printableLabel(el('drift-reference').value,'Target commit or ref'),...(session.review_defaults_active?{repository:session.repository}:{})});driftAssessment(result,session);driftResult=result;});};
  work(async()=>{const status=await api('/api/status');if(status.provider_error)throw new Error(status.provider_error);if(!status.authenticated||!status.repository)throw new Error('Connect GitHub from the landing page first.');const [owner,name]=status.repository.split('/');repository={security_domain:'local',provider:'github',owner,name};el('repository').textContent=`${owner}/${name} · GitHub access checked for each request`;const saved=new URLSearchParams(location.hash.slice(1)).get('session');if(saved){session=await api(`/v1/onboarding/${encodeURIComponent(saved)}?security_domain=local`);restoreDrafts();}});
}
