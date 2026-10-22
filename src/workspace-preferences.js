/* Presentation only: never grants access or modifies report evidence. */
(function(root){
'use strict';
const clone=value=>JSON.parse(JSON.stringify(value));
function defaults(registry){
 const modes={};for(const mode of ['standard','focus']){
  const visibility={},expanded={},order={left:[],main:[],right:[]};
  for(const item of registry){visibility[item.id]=item[mode];if(item.disclosure)expanded[item.id]=item.expandedDefault?.[mode]||false;order[item.region].push(item.id);}
  modes[mode]={hldAppearance:mode==='focus'?'studio':'classic',visibility,expanded,order,widths:{left:206,right:310},view:mode==='focus'?'architecture':'behavior',level:mode==='focus'?'hld':'structural',comparison:'combined',filters:{changedHld:mode==='focus',changedFunctions:mode==='focus',collapseValues:mode==='focus'}};
 }return {version:1,theme:'graphite',defaultMode:'standard',modes};
}
function validate(doc,registry){
 const factory=defaults(registry);
 for(const name of ['standard','focus'])if(doc?.modes?.[name]&&doc.modes[name].hldAppearance===undefined)doc.modes[name].hldAppearance=factory.modes[name].hldAppearance;
 if(doc&&doc.theme===undefined)doc.theme='graphite';
 if(!['graphite','blueprint','terminal','contrast'].includes(doc?.theme))throw Error('Invalid application theme');
 function shape(value,pattern){if(typeof pattern==='boolean')return typeof value==='boolean';if(Array.isArray(pattern))return Array.isArray(value)&&value.length===pattern.length&&new Set(value).size===value.length&&value.every(x=>pattern.includes(x));if(pattern&&typeof pattern==='object')return value&&typeof value==='object'&&!Array.isArray(value)&&Object.keys(value).length===Object.keys(pattern).length&&Object.keys(pattern).every(k=>shape(value[k],pattern[k]));return typeof value===typeof pattern;}
 if(!shape(doc,factory)||doc.version!==1||!['standard','focus'].includes(doc.defaultMode))throw Error('Invalid preference document');
 for(const m of Object.values(doc.modes)){
  if(!['studio','classic'].includes(m.hldAppearance))throw Error('Invalid HLD appearance');
  if(!['behavior','code','architecture'].includes(m.view)||!m.visibility[m.view]||(m.view==='code'&&!m.visibility.source))throw Error('Select a visible default review view');
  if(!['structural','hld','lld'].includes(m.level)||!m.visibility[m.level])throw Error('Select a visible default architecture level');
  if(!['combined','compare'].includes(m.comparison))throw Error('Invalid comparison');
  if(Object.values(m.widths).some(v=>!Number.isInteger(v)||v<200||v>420))throw Error('Panel widths must be 200–420 px');
 }return doc;
}
function resolve(registry,document,mode,temporary=null,reveal=[]){
 const preset=clone((temporary||document).modes[mode]);
 for(const id of reveal)if(id in preset.visibility)preset.visibility[id]=true;if(reveal.some(x=>['behavior','code'].includes(x)))preset.visibility.viewTabs=true;
 const enable=id=>{const item=registry.find(x=>x.id===id);for(const dep of item?.dependencies||[]){preset.visibility[dep]=true;enable(dep);}};
 for(const id of reveal)enable(id);
 for(let i=0;i<registry.length;i++)for(const item of registry)if(item.dependencies.some(dep=>!preset.visibility[dep]))preset.visibility[item.id]=false;
 return preset;
}
class Store{
 constructor(registry,{request=null,storage=null}={}){this.registry=registry;this.request=request;this.storage=storage;this.document=defaults(registry);this.revision=0;this.temporary=null;this.generation=0;this.loaded=false;this.account=null;}
 clear(){this.generation++;this.document=defaults(this.registry);this.revision=0;this.temporary=null;this.loaded=false;this.account=null;}
 async load(){const generation=++this.generation;let response;
  if(this.storage){const raw=this.storage.getItem('csb.workspace-preferences.v1');response={revision:0,preferences:raw?JSON.parse(raw):defaults(this.registry)};}else response=await this.request('/v1/me/workspace-preferences');
  validate(response.preferences,this.registry);if(!Number.isSafeInteger(response.revision)||response.revision<0)throw Error('Invalid preference revision');
  if(generation!==this.generation)return false;this.document=clone(response.preferences);this.revision=response.revision;this.account=response.account||null;this.loaded=true;return true;
 }
 async save(doc){validate(doc,this.registry);const generation=this.generation;
  if(this.request&&!this.storage&&this.account){const identity=await this.request('/v1/me/workspace-preferences');if(identity.account!==this.account){this.clear();throw Error('Account changed. Reload settings before saving.');}}

  let response;if(this.storage){this.storage.setItem('csb.workspace-preferences.v1',JSON.stringify(doc));response={revision:0,preferences:doc};}else response=await this.request('/v1/me/workspace-preferences',{method:'PUT',headers:{'Content-Type':'application/json'},body:JSON.stringify({revision:this.revision,preferences:doc})});
  validate(response.preferences,this.registry);if(generation!==this.generation)throw Error('Account changed; changes were not applied');
  this.document=clone(response.preferences);this.revision=response.revision;this.temporary=null;return response;
 }
}
const api={defaults,validate,resolve,Store};
if(typeof module!=='undefined')module.exports=api;
if(typeof document==='undefined')return;
root.WorkspacePreferences=api;
const assetBase=new URL('.',document.currentScript.src);
api.ready=fetch(new URL('workspace-registry.json',assetBase),{credentials:'same-origin',cache:'no-cache'}).then(r=>{if(!r.ok)throw Error('Workspace registry unavailable');return r.json();});
api.mount=async function(hooks){
 const registry=await api.ready,store=new Store(registry,{request:hooks.request,storage:hooks.static?{getItem:k=>localStorage.getItem(k),setItem:(k,v)=>localStorage.setItem(k,v)}:null});
 let loadFailure='';let previewSnapshot=null;let mode='standard',draft=null,temporaryEdit=false,editorMode='standard',reveal=[],lastReview=null,applyQueued=false,appliedDisclosures=new WeakMap();
 const el=(tag,text,cls)=>{const e=document.createElement(tag);if(text)e.textContent=text;if(cls)e.className=cls;return e;};
 const button=(text,action)=>{const b=el('button',text);b.type='button';b.onclick=action;return b;};
 const panel=el('section',null,'workspace-settings');panel.hidden=true;panel.setAttribute('aria-label','Workspace settings');
 const title=el('h2','Workspace settings'),status=el('p',hooks.static?'Saved in this browser only':'Personal profile · this service deployment');status.setAttribute('role','status');
 const search=el('input');search.type='search';search.placeholder='Search settings';search.setAttribute('aria-label','Search workspace settings');
 const tabs=el('nav');tabs.setAttribute('aria-label','Preset to edit');for(const value of ['standard','focus'])tabs.append(button(value==='standard'?'Standard':'Focus',()=>{editorMode=value;mode=value;hooks.mode(value);hooks.defaults(draft.modes[value]);hooks.filters();draw();apply();}));
 const body=el('div',null,'settings-items'),footer=el('div',null,'settings-actions');
 const message=text=>{status.textContent=text;};
 function error(e){footer.children[4].hidden=false;message(e.status===409||/conflict/i.test(e.message)?'Settings changed on another device. Reload or explicitly Replace.':'Not saved. '+e.message);conflict.hidden=!(e.status===409||/conflict/i.test(e.message));}
 function close(){draft=null;panel.hidden=true;if(previewSnapshot)hooks.restore(previewSnapshot);hooks.filters();apply();openButton.focus();}
 async function save(asDefaults=false){try{validate(draft,registry);if(temporaryEdit&&!asDefaults){store.temporary=clone(draft);draft=null;panel.hidden=true;hooks.filters();apply();return;}await store.save(draft);loadFailure='';draft=null;panel.hidden=true;hooks.filters();apply();}catch(e){error(e);}}
 const conflict=el('div');conflict.hidden=true;
 conflict.append(button('Reload',async()=>{try{await store.load();draft=clone(store.document);conflict.hidden=true;draw();apply();message('Reloaded saved settings.');}catch(e){error(e);}}),button('Replace saved settings',async()=>{try{const replacement=clone(draft);const account=store.account;await store.load();if(account!==store.account)throw Error('Account changed. Reload settings.');draft=replacement;await save(true);}catch(e){error(e);}}));
 footer.append(button('Save',()=>save()),button('Cancel',close),button('Reset this mode',()=>{draft.modes[editorMode]=defaults(registry).modes[editorMode];draw();apply();message('Factory defaults previewed. Save to keep.');}),button('Save as my defaults',()=>save(true)),button('Retry',async()=>{if(draft)await save(!temporaryEdit);else await load();}));
 panel.append(title,status,tabs,search,body,conflict,footer);document.body.append(panel);
 function open(temporary=false){previewSnapshot=hooks.snapshot();temporaryEdit=temporary;editorMode=mode;draft=clone(store.temporary||store.document);panel.hidden=false;conflict.hidden=true;footer.children[3].hidden=!temporary;footer.children[4].hidden=true;message(temporary?'Customize this PR · clears on reload or leaving this PR':hooks.static?'Saved in this browser only':'Personal profile · synchronized within this service');if(loadFailure){message(loadFailure);footer.children[4].hidden=false;}draw();search.focus();}
 const openButton=button('Profile / Settings',()=>open());openButton.id='workspace-settings-open';document.querySelector('.topbar').append(openButton);
 const temporaryButton=button('Customize this PR',()=>open(true));temporaryButton.id='customize-pr';document.querySelector('.review-heading').append(temporaryButton);
 const essential=el('p',null,'essential-status');essential.id='essential-review-status';essential.setAttribute('role','status');document.querySelector('.review-heading').after(essential);
 function current(){return resolve(registry,draft||store.document,mode,draft?null:store.temporary,reveal);}
 function peers(item){const target=item.orderSelector&&document.querySelector(item.orderSelector);if(!target)return [];return draft.modes[editorMode].order[item.region].filter(id=>{const other=registry.find(x=>x.id===id);return other.orderSelector&&document.querySelector(other.orderSelector)?.parentElement===target.parentElement;});}
 function move(item,delta){const list=draft.modes[editorMode].order[item.region],siblings=peers(item),index=siblings.indexOf(item.id),other=siblings[index+delta];if(!other)return;const a=list.indexOf(item.id),b=list.indexOf(other);[list[a],list[b]]=[list[b],list[a]];draw();apply();body.querySelector(`[data-setting="${item.id}"] button`)?.focus();}
 function select(label,values,currentValue,change){const row=el('label',label),input=el('select');for(const value of values){const option=el('option',value);option.value=value;input.append(option);}input.value=currentValue;input.onchange=()=>{change(input.value);apply();};row.append(input);return row;}
 function enableDependencies(m,id){for(const dep of registry.find(x=>x.id===id)?.dependencies||[]){m.visibility[dep]=true;enableDependencies(m,dep);}}
 function draw(){body.replaceChildren();const m=draft.modes[editorMode];
  body.append(select('HLD appearance',['studio','classic'],m.hldAppearance,v=>{m.hldAppearance=v;apply();}),el('p','Studio: compact cards and clean connectors from the header demo. Classic: original Mermaid styling. HLD only; evidence and LLD are unchanged.'));
  body.append(select('Application theme',['graphite','blueprint','terminal','contrast'],draft.theme,v=>draft.theme=v),el('p','Applies throughout Standard, Focus, queues and settings. Terminal uses ASCII-inspired borders and typography; diagrams retain their interactive behavior.'));
  body.append(el('p','Entry mode: PR check links open Focus; direct dashboard entry opens Standard. Mode can still be switched during review.'),select('Default review view',['behavior','code','architecture'],m.view,v=>{m.view=v;m.visibility[v]=true;draw();hooks.defaults(m);}),select('Default level',['structural','hld','lld'],m.level,v=>{m.level=v;m.visibility[v]=true;draw();hooks.defaults(m);}),select('Comparison',['combined','compare'],m.comparison,v=>{m.comparison=v;hooks.defaults(m);}));
  for(const side of ['left','right']){const label=el('label',`${side} panel width`),range=el('input'),value=el('output',String(m.widths[side]));range.type='range';range.min=200;range.max=420;range.value=m.widths[side];range.oninput=()=>{m.widths[side]=Number(range.value);value.textContent=range.value;apply();};label.append(range,value);body.append(label);}
  const filters=el('fieldset');filters.append(el('legend','Diagram content (presentation only)'));
  for(const [key,label] of Object.entries({changedHld:'Changed HLD relationships only',changedFunctions:'Changed functions only',collapseValues:'Collapse safe linear value steps'})){const row=el('label',label),input=el('input');input.type='checkbox';input.checked=m.filters[key];input.onchange=()=>{m.filters[key]=input.checked;apply();hooks.filters();};row.prepend(input);filters.append(row);}body.append(filters);
  for(const category of new Set(registry.map(x=>x.category))){const members=registry.filter(x=>x.category===category&&(!search.value||`${x.label} ${category}`.toLowerCase().includes(search.value.toLowerCase())));if(!members.length)continue;const group=el('fieldset'),legend=el('legend',category),all=button('Select category',()=>{members.forEach(x=>{m.visibility[x.id]=true;enableDependencies(m,x.id);hooks.revealRegion(x.region);});draw();apply();}),none=button('Hide category',()=>{members.forEach(x=>m.visibility[x.id]=false);draw();apply();});group.append(legend,all,none);
   for(const item of members.sort((a,b)=>m.order[a.region].indexOf(a.id)-m.order[b.region].indexOf(b.id))){const row=el('div',null,'setting-row');row.dataset.setting=item.id;const label=el('label',item.label),input=el('input');input.type='checkbox';input.checked=m.visibility[item.id];input.onchange=()=>{m.visibility[item.id]=input.checked;if(input.checked){enableDependencies(m,item.id);hooks.revealRegion(item.region);}apply();};label.prepend(input);row.append(label,el('small',item.region));
    const targets=item.selector?[...document.querySelectorAll(item.selector)]:[];if(item.selector&&(!targets.length||targets.every(x=>x.closest('[hidden]'))))row.append(el('small','Unavailable in current report; preference retained'));
    if(item.disclosure){const expanded=el('label','Expanded'),check=el('input');check.type='checkbox';check.checked=m.expanded[item.id];check.onchange=()=>{m.expanded[item.id]=check.checked;appliedDisclosures=new WeakMap();apply();};expanded.prepend(check);row.append(expanded);}
    const handle=button('↕',()=>{});handle.draggable=true;handle.setAttribute('aria-label',`Drag ${item.label} within ${item.region}`);handle.ondragstart=e=>e.dataTransfer.setData('text/plain',item.id);row.ondragover=e=>e.preventDefault();row.ondrop=e=>{e.preventDefault();const id=e.dataTransfer.getData('text/plain'),list=m.order[item.region];if(!peers(item).includes(id))return;list.splice(list.indexOf(id),1);list.splice(list.indexOf(item.id),0,id);draw();apply();};const up=button('Move up',()=>move(item,-1)),down=button('Move down',()=>move(item,1)),siblings=peers(item),position=siblings.indexOf(item.id);up.disabled=position<=0;down.disabled=position<0||position===siblings.length-1;handle.disabled=siblings.length<2;handle.draggable=!handle.disabled;if(!item.orderSelector)row.append(el('small','Moves with its parent section'));else row.append(handle,up,down);group.append(row);
   }body.append(group);
  }
 }
 function apply(){if(draft){try{validate(draft,registry);}catch(e){message(e.message);return;}}if(applyQueued)return;applyQueued=true;queueMicrotask(()=>{applyQueued=false;const m=current();document.documentElement.dataset.hldAppearance=m.hldAppearance;document.documentElement.dataset.theme=(draft||store.temporary||store.document).theme||'graphite';try{sessionStorage.setItem('csb.application-theme',document.documentElement.dataset.theme);}catch(_){}
  const active=document.activeElement;const hidden=new Set(),known=new Set();
  for(const item of registry){if(!item.selector)continue;for(const target of document.querySelectorAll(item.selector)){known.add(target);if(!m.visibility[item.id]||item.permissions.some(permission=>!hooks.permission(permission)))hidden.add(target);if(item.disclosure&&target.tagName==='DETAILS'&&appliedDisclosures.get(target)!==`${mode}:${m.expanded[item.id]}`){target.open=m.expanded[item.id];appliedDisclosures.set(target,`${mode}:${m.expanded[item.id]}`);}}}
  for(const target of known)target.classList.toggle('preference-hidden',hidden.has(target));
  const visible=selector=>[...document.querySelectorAll(selector)].some(x=>!x.closest('[hidden],.preference-hidden'));
  const left=visible('#changed-files-panel,#pr-list,#nav-repositories,#nav-history'),right=visible('#inspection-context>section,#inspection-context>details');
  document.body.classList.toggle('preference-no-left',!left);document.body.classList.toggle('preference-no-right',!right);
  document.body.classList.toggle('preference-no-line-numbers',!m.visibility.lineNumbers);document.body.classList.toggle('preference-no-syntax',!m.visibility.syntax);
  document.documentElement.style.setProperty('--preference-left',`${m.widths.left}px`);document.documentElement.style.setProperty('--preference-right',`${m.widths.right}px`);
  // Reorder only siblings; nested controls remain attached to their evidence.
  for(const region of ['left','main','right']){const ranks=new Map(m.order[region].map((id,i)=>[id,i])),groups=new Map();for(const item of registry.filter(x=>x.region===region&&x.orderSelector)){const target=document.querySelector(item.orderSelector);if(!target)continue;const parent=target.parentElement;if(!groups.has(parent))groups.set(parent,[]);groups.get(parent).push([target,ranks.get(item.id)]);}for(const [parent,rows] of groups){const unique=[...new Map(rows.map(([node,rank])=>[node,rank])).entries()];if(unique.length<2)continue;const desired=unique.sort((a,b)=>a[1]-b[1]).map(x=>x[0]),existing=[...parent.children].filter(x=>desired.includes(x));if(existing.some((x,i)=>x!==desired[i])){const markers=existing.map(x=>{const mark=document.createComment('workspace-order');x.before(mark);return mark;});markers.forEach((mark,i)=>mark.replaceWith(desired[i]));}}}
  const statusText=hooks.status();if(essential.textContent!==statusText)essential.textContent=statusText;
  if(active?.closest('.preference-hidden')){const next=[...document.querySelectorAll('.workspace-modes button,.design-levels button')].find(x=>!x.closest('[hidden],.preference-hidden'));(next||openButton).focus();}
  hooks.visibility(m);
 });}
 search.oninput=draw;
 panel.addEventListener('keydown',e=>{if(e.key==='Escape'){e.preventDefault();close();}});
 new MutationObserver(records=>{if(records.some(r=>r.type==='childList'&&(r.addedNodes.length||r.removedNodes.length)))apply();}).observe(document.querySelector('.shell'),{subtree:true,childList:true});
 async function load(){try{await store.load();loadFailure='';message(hooks.static?'Saved in this browser only':'Saved profile loaded');apply();return true;}catch(e){loadFailure='Not saved. '+e.message;message(loadFailure);apply();return false;}}
 window.addEventListener('focus',async()=>{if(hooks.static)return;try{const response=await hooks.request('/v1/me/workspace-preferences');if(response.account!==store.account){store.clear();draft=null;panel.hidden=true;await load();hooks.filters();}else if(!draft){await load();hooks.filters();}}catch(e){if([401,403].includes(e.status)){store.clear();draft=null;panel.hidden=true;apply();}message('Not saved. '+e.message);}});
 const controller={open,current,clearReveal(){reveal=[];},setMode(value){mode=value;apply();},filters(){return current().filters;},changed(){apply();},review(id){if(id!==lastReview){store.temporary=null;draft=null;panel.hidden=true;lastReview=id;reveal=[];}apply();},reveal(ids){reveal=[...new Set([...reveal,...ids])];apply();},async login(){store.clear();draft=null;reveal=[];apply();await load();},logout(){store.clear();draft=null;panel.hidden=true;apply();},defaults(){return (store.temporary||store.document).modes[mode];},defaultMode(){return store.document.defaultMode;}};
 await load();return controller;
};
})(typeof window==='undefined'?globalThis:window);
