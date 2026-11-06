/* Browser-only setup state. Credentials stay in HttpOnly cookies, never storage. */
'use strict';
(() => {
  const $ = id => document.getElementById(id);
  let repositories = [], selected = null, inventory = null, revision = 0, preview = null;
  let generation = 0, busy = false;
  const hidden = (id, value) => { $(id).hidden = value; };
  const status = text => { $('status').textContent = text; };
  function invalidate() { preview = null; hidden('confirmation', true); hidden('complete', true); $('acknowledge').checked = false; $('confirm').disabled = true; }
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
    busy=true; hidden('error',true); document.querySelectorAll('button,input,textarea').forEach(b=>{b.dataset.wasDisabled=String(b.disabled);b.disabled=true;});
    try { await action(); } catch(error) { fail(error); }
    finally { busy=false; document.querySelectorAll('button,input,textarea').forEach(b=>{if(b.dataset.wasDisabled!==undefined){b.disabled=b.dataset.wasDisabled==='true';delete b.dataset.wasDisabled;}}); $('confirm').disabled=!preview||!$('acknowledge').checked; $('preview').disabled=!inventory; }
  }
  const identity = () => ({installation_id:selected.installation_id, repository_id:selected.repository_id});
  const lines = id => [...new Set($(id).value.split('\n').map(s=>s.trim()).filter(Boolean))];
  function renderRepositories() {
    $('repositories').replaceChildren(); const query=$('repo-search').value.toLowerCase();
    const rows=repositories.filter(r=>`${r.owner}/${r.name}`.toLowerCase().includes(query)); hidden('repo-empty',rows.length>0);
    for(const repo of rows) { const button=document.createElement('button'); button.type='button';if(busy){button.dataset.wasDisabled='false';button.disabled=true;}button.className='component-choice';button.textContent=`${repo.owner}/${repo.name}`;button.addEventListener('click',()=>run(async()=>{selected=repo;inventory=null;generation++;invalidate();hidden('scope',true); await api('repositories',identity()); await load(true);}));$('repositories').append(button); }
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
  async function load(restore) {
    const current=++generation;invalidate();status('Loading path metadata…');
    const value=await api('inventory',identity());if(current!==generation)return;
    inventory={...value.inventory,blocked:new Set(value.automatically_excluded)};revision=value.saved?.revision||0;
    $('scope-title').textContent=`2. Choose paths · ${selected.owner}/${selected.name}`;
    $('revision').textContent=`${inventory.default_branch} · ${inventory.commit}`;
    $('saved-status').textContent=value.saved?`Saved revision ${revision} · ${value.saved.document.commit} · checks off`:'No scope saved yet.';
    if(restore) {$('include').value=value.saved?.document.include.join('\n')||'';$('exclude').value=value.saved?.document.exclude.join('\n')||'';}
    $('selected-name').textContent=`${selected.owner}/${selected.name}`;hidden('selected-repo',false);hidden('picker',true);hidden('scope',false);renderPaths();$('scope-title').focus();status('Choose paths. No file contents have been read.');
  }
  async function connect() {
    clear();const capability=await api('capability');
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
    revision=saved.revision;invalidate();$('receipt').textContent=`${selected.owner}/${selected.name} · revision ${saved.revision} · ${saved.document.commit} · ${saved.document.files.length} files`;hidden('complete',false);$('complete-title').focus();status('Scope saved. Analysis, publication and checks remain off.');
  }));
  $('revise').addEventListener('click',()=>run(()=>load(true)));
  const chooseAnother=()=>{invalidate();selected=inventory=null;hidden('scope',true);hidden('selected-repo',true);hidden('picker',false);$('repo-search').focus();};
  $('another').addEventListener('click',chooseAnother);$('change-repository').addEventListener('click',chooseAnother);
  $('logout').addEventListener('click',()=>run(async()=>{clear();await api('logout',{});hidden('logout',true);hidden('login',false);status('Signed out. Checks remain off.');}));
  run(connect);
})();
