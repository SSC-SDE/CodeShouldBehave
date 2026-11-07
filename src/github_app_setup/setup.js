/* Browser-only setup state. Credentials stay in HttpOnly cookies, never storage. */
'use strict';
(() => {
  const $ = id => document.getElementById(id);
  let repositories = [], selected = null, inventory = null, revision = 0, preview = null;
  let generation = 0, busy = false, sourceAvailable = false, architecture = null, publicationAvailable = false, publication = null;
  function clearArchitecture(){clearPublication();architecture=null;["architecture-review","architecture-saved"].forEach(id=>hidden(id,true));$("architecture-components").replaceChildren();$("architecture-gaps").replaceChildren();$("architecture-providers").replaceChildren();$("architecture-receipt").textContent="";$("architecture-gaps-ack").checked=$("architecture-defer").checked=$("scan-consent").checked=false;$("accept-architecture").disabled=$("scan-source").disabled=true;}
  const hidden = (id, value) => { $(id).hidden = value; };
  const status = text => { $('status').textContent = text; };
  function invalidate() { clearArchitecture();preview = null; hidden('confirmation', true); hidden('complete', true); $('acknowledge').checked = false; $('confirm').disabled = true; }
  function clear() { generation++; repositories = []; selected = inventory = null; revision = 0; invalidate(); ['picker','scope'].forEach(id => hidden(id,true)); $('repositories').replaceChildren(); $('paths').replaceChildren(); $('summary').replaceChildren(); $('selected-files').replaceChildren(); $('include').value = $('exclude').value = ''; for(const id of ['revision','saved-status','receipt'])$(id).textContent=''; hidden('logout',true); hidden('selected-repo',true); $('selected-name').textContent='';document.body.classList.remove('ready'); }
  function message(error) {
    const messages = {github_authentication_required:'Session expired. Sign in again.', repository_setup_not_authorized:'Repository access changed. Sign in again and check your installation permissions.', scope_changed_review_again:'The preview expired, or the commit or saved scope changed. Reload paths, review and confirm again.', invalid_setup_request:'Choose at least one allowed file. Use existing literal paths, up to 32 per list.', setup_capacity_reached:'Setup limit reached. Wait and retry; no changes were saved.', github_setup_unavailable:'Setup is unavailable. Retry or reload paths to check the saved revision.'};
    return messages[error.code] || 'Request failed. Check your connection and retry.';
  }
  async function api(path, data) {
    const controller=new AbortController(),timeout=setTimeout(()=>controller.abort(),125000);
    try {
      const response = await fetch(`/v1/github-app/${path}`, {method:data===undefined?'GET':'POST', credentials:'same-origin', cache:'no-store', redirect:'error', signal:controller.signal, headers:data===undefined?{}:{'Content-Type':'application/json'}, body:data===undefined?undefined:JSON.stringify(data)});
      const value = response.status === 204 ? {} : await response.json().catch(()=>null);
      if (!response.ok) { const error = new Error('Setup request failed'); error.code = value?.error; error.status = response.status; throw error; }
      if(!value)throw new Error('Invalid setup response');
      return value;
    } finally {clearTimeout(timeout);}
  }
  function fail(error) {
    invalidate();
    if(inventory)hidden('scope',false);
    if ([401,403].includes(error.status)) { clear(); hidden('login',false); hidden('logout',false); }
    if (error.status===409) { inventory=null; $('preview').disabled=true; }
    $('error').textContent=message(error); hidden('error',false); status('Setup needs attention. Checks remain off.');
  }
  async function run(action) {
    if (busy) return;
    busy=true; hidden('error',true); document.querySelectorAll('button,input,textarea,select').forEach(b=>{b.dataset.wasDisabled=String(b.disabled);b.disabled=true;});
    try { await action(); } catch(error) { fail(error); }
    finally { busy=false; document.querySelectorAll('button,input,textarea,select').forEach(b=>{if(b.dataset.wasDisabled!==undefined){b.disabled=b.dataset.wasDisabled==='true';delete b.dataset.wasDisabled;}}); $('confirm').disabled=!preview||!$('acknowledge').checked; $('preview').disabled=!inventory; $('scan-source').disabled=!sourceAvailable||!$('scan-consent').checked;syncArchitecture();syncPublication(); }
  }
  const identity = () => ({installation_id:selected.installation_id, repository_id:selected.repository_id});
  const lines = id => [...new Set($(id).value.split('\n').map(s=>s.trim()).filter(Boolean))];
  function renderRepositories() {
    $('repositories').replaceChildren(); const query=$('repo-search').value.toLowerCase();
    const rows=repositories.filter(r=>`${r.owner}/${r.name}`.toLowerCase().includes(query)); hidden('repo-empty',rows.length>0);
    for(const repo of rows) { const button=document.createElement('button'); button.type='button';if(busy){button.dataset.wasDisabled='false';button.disabled=true;}button.className='component-choice';button.textContent=`${repo.owner}/${repo.name}`;button.addEventListener('click',()=>run(async()=>{selected=repo;inventory=null;generation++;invalidate();hidden('scope',true); await api('repositories',identity()); await load(true,true);}));$('repositories').append(button); }
  }
  function renderPaths() {
    $('paths').replaceChildren(); if(!inventory)return;
    const query=$('path-search').value.toLowerCase(), includes=new Set(lines('include'));
    for(const entry of inventory.entries.filter(e=>e.path.toLowerCase().includes(query))) {
      const tr=document.createElement('tr'), td=document.createElement('td'), checkbox=document.createElement('input');checkbox.type='checkbox';checkbox.checked=includes.has(entry.path);checkbox.disabled=inventory.blocked.has(entry.path);if(busy){checkbox.dataset.wasDisabled=String(checkbox.disabled);checkbox.disabled=true;}checkbox.setAttribute('aria-label',`Include ${entry.path}`);
      checkbox.addEventListener('change',()=>{const values=new Set(lines('include'));if(checkbox.checked)values.add(entry.path);else values.delete(entry.path);$('include').value=[...values].join('\n');invalidate();});td.append(checkbox);tr.append(td);
      for(const text of [entry.path,inventory.blocked.has(entry.path)?`${entry.kind} · excluded`:entry.kind]) {const cell=document.createElement('td');cell.textContent=text;tr.append(cell);} $('paths').append(tr);
    }
  }
  async function load(restore,resume=false) {
    const current=++generation;invalidate();status('Loading path metadata…');
    const value=await api('inventory',identity());if(current!==generation)return;
    inventory={...value.inventory,blocked:new Set(value.automatically_excluded)};revision=value.saved?.revision||0;
    $('scope-title').textContent=`2. Choose paths · ${selected.owner}/${selected.name}`;
    $('revision').textContent=`${inventory.default_branch} · ${inventory.commit}`;
    $('saved-status').textContent=value.saved?`Saved revision ${revision} · ${value.saved.document.commit} · checks off`:'No scope saved yet.';
    if(restore) {$('include').value=value.saved?.document.include.join('\n')||'';$('exclude').value=value.saved?.document.exclude.join('\n')||'';}
    $('selected-name').textContent=`${selected.owner}/${selected.name}`;hidden('selected-repo',false);hidden('picker',true);hidden('scope',false);renderPaths();$('scope-title').focus();status('Choose paths. No file contents have been read.');if(value.saved&&sourceAvailable&&resume){hidden('scope',true);hidden('complete',false);hidden('source-consent',false);$('receipt').textContent=`${selected.owner}/${selected.name} · revision ${revision} · ${value.saved.document.commit}`;const accepted=await api('architecture/status',identity());if(accepted.architecture_confirmed){await showArchitectureSaved(accepted);}}
  }
  async function connect() {
    clear();const capability=await api('capability');sourceAvailable=capability.source_preview_available===true;publicationAvailable=capability.publication_consent_available===true;
    if(!capability.available) {hidden('login',true);hidden('logout',true);$('connection-detail').textContent='Hosted setup is not configured. No sign-in, repository scan or activation is available on this server.';status('Setup unavailable · checks off');return;}
    hidden('login',false);$('connection-detail').textContent=(params.has('auth_error')?'Authorization was not completed. Sign in again. ':'')+'Only installed public repositories you administer are available.';
    try {const value=await api('repositories');repositories=value.repositories;document.body.classList.add('ready');hidden('login',true);hidden('logout',false);hidden('picker',false);renderRepositories();status('Select a repository. Checks remain off.');} catch(error) {if(error.status===401){status('Sign in to choose an installed repository.');return;}throw error;}
    if(hint&&/^\d+$/.test(hint)) {repositories.sort((a,b)=>Number(String(b.installation_id)===hint)-Number(String(a.installation_id)===hint));renderRepositories();}
  }
  const params=new URLSearchParams(location.search), hint=params.get('installation_id');
  if(hint&&/^[1-9]\d{0,17}$/.test(hint))$('login').href+=`?installation_id=${encodeURIComponent(hint)}`;
  if(params.has('auth_error')) {$('connection-detail').textContent='Authorization was not completed. You can sign in again.';}
  // Remove callback feedback/setup hints from the current browser history entry.
  if(location.search)history.replaceState(null,'',location.pathname);
  $('retry').addEventListener('click',()=>run(connect));
  $('repo-search').addEventListener('input',renderRepositories);$('path-search').addEventListener('input',renderPaths);
  for(const id of ['include','exclude'])$(id).addEventListener('input',()=>{invalidate();if(id==='include')renderPaths();});
  $('refresh').addEventListener('click',()=>run(()=>load(false)));
  $('scope-form').addEventListener('submit',event=>{event.preventDefault();run(async()=>{
    preview=await api('scope/preview',{...identity(),commit:inventory.commit,expected_revision:revision,include:lines('include'),exclude:lines('exclude')});
    const d=preview.document;$('summary').replaceChildren();
    for(const [key,value] of [['Repository',`${selected.owner}/${selected.name}`],['Commit',d.commit],['Include',d.include.join(', ')],['Exclude',d.exclude.join(', ')||'None selected'],['Files',`${d.files.length} selected · ${d.rust_candidates} Rust candidates · ${d.unsupported_files} other/unsupported · ${d.automatically_excluded} automatically excluded`],['Activation','Off · no source scan or publication consent']]) {const dt=document.createElement('dt'),dd=document.createElement('dd');dt.textContent=key;dd.textContent=value;$('summary').append(dt,dd);}
    $('selected-files').replaceChildren();for(const path of d.files){const li=document.createElement('li');li.textContent=path;$('selected-files').append(li);}
    $('acknowledge').checked=false;hidden('scope',true);hidden('confirmation',false);$('confirm-title').focus();status('Review the exact scope before saving.');
  });});
  $('acknowledge').addEventListener('change',()=>{$('confirm').disabled=busy||!preview||!$('acknowledge').checked;});
  $('edit').addEventListener('click',()=>{invalidate();hidden('scope',false);$('include').focus();});
  $('confirm').addEventListener('click',()=>run(async()=>{
    if(!preview||!$('acknowledge').checked)return;
    const saved=await api('scope/confirm',{...identity(),preview_id:preview.preview_id,expected_revision:preview.expected_revision,acknowledge_limits:true});
    revision=saved.revision;invalidate();$('receipt').textContent=`${selected.owner}/${selected.name} · revision ${saved.revision} · ${saved.document.commit} · ${saved.document.files.length} files`;hidden('complete',false);hidden('source-consent',!sourceAvailable);$('next-step').textContent=sourceAvailable?'Source preview is a separate, explicit action.':'Isolated preview worker is not configured. No source scan can start on this host.';$('complete-title').focus();status('Scope saved. Analysis, publication and checks remain off.');
  }));
  $('revise').addEventListener('click',()=>run(()=>load(true)));
  const chooseAnother=()=>{invalidate();selected=inventory=null;hidden('scope',true);hidden('selected-repo',true);hidden('picker',false);$('repo-search').focus();};
  $('another').addEventListener('click',chooseAnother);$('change-repository').addEventListener('click',chooseAnother);
  $('logout').addEventListener('click',()=>run(async()=>{clear();await api('logout',{});hidden('logout',true);hidden('login',false);status('Signed out. Checks remain off.');}));

  function syncArchitecture(){$('accept-architecture').disabled=busy||!architecture||!architecture.result.components.length||!$('architecture-gaps-ack').checked||!$('architecture-defer').checked;}
  async function showArchitectureSaved(value){hidden('publication',!publicationAvailable);hidden('complete',true);hidden('architecture-review',true);hidden('architecture-saved',false);$('architecture-receipt').textContent=`${selected.owner}/${selected.name} · scope revision ${value.scope_revision} · ${value.commit||inventory?.commit||""} · publication is a separate permission`;$('architecture-saved-title').focus();status('Architecture saved. Checks remain off.');if(publicationAvailable){const consent=await api('publication/status',identity());$('publication-status').textContent=consent.publication_consent?`Consent revision ${consent.consent_revision} saved. Checks remain off pending operator activation.`:'No current publication consent.';}}
  $('scan-consent').addEventListener('change',()=>{$('scan-source').disabled=busy||!sourceAvailable||!$('scan-consent').checked;});
  for(const id of ['architecture-gaps-ack','architecture-defer'])$(id).addEventListener('change',syncArchitecture);
  $('scan-source').addEventListener('click',()=>run(async()=>{
    if(!$('scan-consent').checked||!sourceAvailable)return;clearArchitecture();
    status('Reading selected source and preparing architecture…');
    architecture=await api('architecture/preview',{...identity(),scope_revision:revision,allow_selected_source_scan:true});
    $('architecture-revision').textContent=`Scope revision ${architecture.scope_revision} · expires ${new Date(architecture.expires_at_s*1000).toLocaleTimeString()}`;
    for(const component of architecture.result.components){
      const card=document.createElement('fieldset');card.className='architecture-component';card.dataset.id=component.id;
      const title=document.createElement('legend');title.textContent=component.role;card.append(title);
      for(const [name,label,value] of [['name','Component name',component.name],['roots','Roots · one per line',component.roots.join('\n')]]){const l=document.createElement('label');l.textContent=label;const field=document.createElement(name==='roots'?'textarea':'input');field.dataset.field=name;field.value=value;field.maxLength=1024;l.append(field);card.append(l);}
      const l=document.createElement('label');l.textContent='Decision';const select=document.createElement('select');select.dataset.field='decision';for(const [value,label] of [['confirmed','Include'],['rejected','Reject'],['deferred','Defer']]){const o=document.createElement('option');o.value=value;o.textContent=label;select.append(o);}l.append(select);card.append(l);
      const facts=document.createElement('p');facts.textContent=component.stack_candidates.join(' · ');card.append(facts);
      const evidence=document.createElement('details'),summary=document.createElement('summary');summary.textContent='Source evidence and limits';evidence.append(summary);
      for(const text of [...component.evidence.map(e=>`${e.path}:${e.start_line}–${e.end_line} · ${e.blob}`),...component.contradictions]){const p=document.createElement('p');p.textContent=text;evidence.append(p);}card.append(evidence);$('architecture-components').append(card);
    }
    for(const gap of architecture.result.gaps){const li=document.createElement('li');li.textContent=gap;$('architecture-gaps').append(li);}
    for(const mapping of architecture.result.providers){const li=document.createElement('li');li.textContent=`${mapping.provider} · ${mapping.source_root} · ${mapping.verification} · connection deferred`;$('architecture-providers').append(li);}
    hidden('complete',true);hidden('architecture-review',false);$('architecture-title').focus();status('Review component boundaries and gaps. Nothing is enabled.');
  }));
  $('accept-architecture').addEventListener('click',()=>run(async()=>{
    if(!architecture||!$('architecture-gaps-ack').checked||!$('architecture-defer').checked)return;
    const components=[...$('architecture-components').children].map(card=>{const original=architecture.result.components.find(c=>c.id===card.dataset.id);const name=card.querySelector('[data-field="name"]').value.trim(),roots=[...new Set(card.querySelector('[data-field="roots"]').value.split('\n').map(x=>x.trim()).filter(Boolean))];let decision=card.querySelector('select').value;if(decision==='confirmed'&&(name!==original.name||JSON.stringify(roots)!==JSON.stringify(original.roots)))decision='corrected';return {id:original.id,name,roots,decision};});
    const value=await api('architecture/confirm',{...identity(),scope_revision:architecture.scope_revision,preview_id:architecture.preview_id,components,accept_known_gaps:true,defer_integrations:true});clearArchitecture();await showArchitectureSaved(value);
  }));
  for(const id of ['discard-architecture','architecture-back'])$(id).addEventListener('click',()=>{clearArchitecture();hidden('complete',false);});

  function clearPublication(){publication=null;hidden('publication',true);hidden('publication-review',true);$('publication-details').replaceChildren();$('publication-status').textContent='';for(const id of ['publication-analysis','publication-source','publication-checks'])$(id).checked=false;$('publication-confirm').disabled=true;}
  function syncPublication(){$('publication-confirm').disabled=busy||!publication||!['publication-analysis','publication-source','publication-checks'].every(id=>$(id).checked);}
  for(const id of ['publication-analysis','publication-source','publication-checks'])$(id).addEventListener('change',syncPublication);
  $('publication-preview').addEventListener('click',()=>run(async()=>{
    publication=await api('publication/preview',identity());const d=publication.document;
    $('publication-details').replaceChildren();
    for(const [label,value] of [['Repository',`${selected.owner}/${selected.name}`],['Scope',`Revision ${d.scope_revision} · ${d.scope_commit}`],['Include',d.policy.include.join(', ')],['Exclude',d.policy.exclude.join(', ')||'None'],['Public destination',d.operator.report_origin],['Engine SHA-256',d.operator.engine_digest],['Visibility',d.source_visibility],['Retention',`${d.retention_days} days`],['Limits',`${d.limits.unique_blobs} unique blobs · ${d.limits.job_rest_reservations} reserved REST calls per comparison · ${d.limits.installation_daily_rest_reservations} per installation/day`],['Valid until',new Date(d.expires_at*1000).toLocaleTimeString()],['Activation','Off; separate operator approval required']]){const dt=document.createElement('dt'),dd=document.createElement('dd');dt.textContent=label;dd.textContent=value;$('publication-details').append(dt,dd);}
    for(const id of ['publication-analysis','publication-source','publication-checks'])$(id).checked=false;
    hidden('publication-review',false);$('publication-analysis').focus();$('publication-status').textContent='Review all three permissions. Nothing is enabled.';
  }));
  $('publication-confirm').addEventListener('click',()=>run(async()=>{
    if(!publication||!['publication-analysis','publication-source','publication-checks'].every(id=>$(id).checked))return;
    const value=await api('publication/confirm',{...identity(),expected_revision:publication.document.expected_revision,expires_at:publication.document.expires_at,document_digest:publication.document_digest,allow_automatic_analysis:true,allow_public_source:true,allow_checks:true});
    publication=null;hidden('publication-review',true);$('publication-status').textContent=`Consent revision ${value.consent_revision} saved. Checks remain off pending operator activation.`;
  }));
  $('publication-revoke').addEventListener('click',()=>run(async()=>{
    await api('publication/revoke',identity());publication=null;hidden('publication-review',true);$('publication-status').textContent='Consent revoked. Jobs cancelled and existing reports withdrawn. Checks remain off.';
  }));
  run(connect);
})();
