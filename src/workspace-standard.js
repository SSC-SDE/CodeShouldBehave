/* Standard presentation of authorized DTOs. Native Focus and LLD own their renderers. */
(function(root){
'use strict';
const sections=['changes','hld','lld','source'];
const markers={added:'+',removed:'−',modified:'~',uncertain:'?'};
const short=value=>String(value||'unavailable').slice(0,8);
function changeRows(payload,files=[]){
  const elements=new Map([...(payload?.baseline?.elements||[]),...(payload?.proposal?.elements||[])].map(item=>[item.id,item]));
  const relations=new Map([...(payload?.baseline?.relations||[]),...(payload?.proposal?.relations||[])].map(item=>[item.id,item]));
  const rows=(payload?.delta?.changes||[]).map((change,index)=>{
    const relation=relations.get(change.relation_id),element=elements.get(change.element_id||relation?.to);
    const kind=Object.hasOwn(markers,change.kind)?change.kind:'uncertain';
    return {key:`change-${index}`,kind,title:element?.name||relation?.kind?.replaceAll('_',' ')||'Design observation',reason:change.reason||'No explanatory evidence supplied.',elementId:element?.id||null,relationId:relation?.id||null};
  });
  return rows.length?rows:files.map(file=>({key:`file-${file.path}`,kind:({added:'added',deleted:'removed',modified:'modified',renamed:'modified'})[file.kind]||'uncertain',title:file.path,reason:`${file.kind||'Changed'} file${Number.isFinite(file.additions)?` · +${file.additions}`:''}${Number.isFinite(file.deletions)?` / −${file.deletions}`:''}. Architectural meaning is not established by file counts.`,path:file.path}));
}
function exactComponent(payload,elementId,side='proposal'){
  // Never substitute a different snapshot or a same-named component.
  const version=payload?.[side],element=version?.elements?.find(item=>item.id===elementId);
  const component=element?.kind==='component'?element:version?.elements?.find(item=>item.id===element?.component_id&&item.kind==='component');
  return version&&component?{version,component}:null;
}
function boundaries(payload,status,model){
  const scopes=model.boundaries(payload,{changedOnly:false,status});
  const represented=new Set(scopes.flatMap(scope=>[scope.source.id,scope.target.id]));
  const elements=new Map([...(payload.baseline?.elements||[]),...(payload.proposal?.elements||[])].map(item=>[item.id,item]));
  for(const change of payload.delta?.changes||[]){
    const element=elements.get(change.element_id);if(!element||represented.has(element.id))continue;
    represented.add(element.id);scopes.push({key:`${element.id}\0${element.id}`,source:element,target:element,baseline:[],proposal:[],accepted:[],status:status.get(element.id),changed:true,reason:change.reason});
  }
  return scopes;
}
function sourceMatches(source,evidence,reviewId){
  return Boolean(source&&source.review_id===reviewId&&source.commit===evidence.commit&&source.path===evidence.path&&source.blob_id===(evidence.blob_id||evidence.blob)&&source.start_line===evidence.start_line&&Number.isSafeInteger(source.end_line)&&source.end_line>=source.start_line&&source.end_line<=evidence.end_line&&typeof source.text==='string');
}
class Positions{
  constructor(){this.items=new Map();}
  save(id,value){if(id)this.items.set(id,{...value});}
  get(id){return {...(this.items.get(id)||{section:'changes',boundary:null,changePage:0,sourceReturn:'changes',sourceKind:null,sourceId:null})};}
  clear(){this.items.clear();}
}
function mount(hooks){
  const doc=root.document,byId=id=>doc.getElementById(id),model=root.WorkspaceFocusModel;
  const el=(tag,cls,text)=>{const item=doc.createElement(tag);if(cls)item.className=cls;if(text!==undefined)item.textContent=String(text);return item;};
  const button=(text,action,cls='standard-action')=>{const item=el('button',cls,text);item.type='button';item.onclick=action;return item;};
  const desk=el('section','standard-desk');desk.id='standard-desk';desk.hidden=true;desk.setAttribute('aria-label','Standard review desk');byId('review-content').prepend(desk);
  const heading=el('header','standard-heading'),crumb=el('p','standard-crumb'),title=el('h1'),summary=el('p','standard-summary');heading.append(crumb,title,summary);
  const tabs=el('nav','standard-tabs');tabs.setAttribute('aria-label','Review sections');const tabButtons=new Map();
  for(const section of sections){const item=button(section==='changes'?'Changes':section==='source'?'Source':section.toUpperCase(),()=>navigate(section));tabButtons.set(section,item);tabs.append(item);}
  const evidenceToggle=button('Evidence & scope',()=>{position.evidenceOpen=!position.evidenceOpen;draw();},'standard-evidence-toggle');evidenceToggle.setAttribute('aria-controls','standard-evidence');tabs.append(evidenceToggle);
  const status=el('p','standard-status');status.setAttribute('role','status');
  const content=el('div','standard-content'),evidence=el('section','standard-evidence');evidence.id='standard-evidence';evidence.hidden=true;
  desk.append(heading,tabs,status,evidence,content);
  const searchLabel=el('label','standard-queue-search','Find a PR'),search=el('input');search.type='search';search.placeholder='Repository or PR';search.setAttribute('aria-label','Find a pull request');searchLabel.append(search);byId('pr-list').before(searchLabel);
  const searchStatus=el('p','standard-queue-status');searchStatus.setAttribute('role','status');searchLabel.after(searchStatus);
  let active=false,state=null,pr=null,reviewId=null,position=new Positions().get(null),scopes=[],source=null,sourceError='',sourceBusy=false,sourceGeneration=0,lastKey='',queueKey='',hldPayload=null,filesPayload=null,dataVersion=0;
  const positions=new Positions();
  function save(){positions.save(reviewId,position);}
  function filterQueue(){let count=0,total=0;const query=search.value.trim().toLowerCase();for(const card of byId('pr-list').children){total++;card.hidden=active&&!card.textContent.toLowerCase().includes(query);if(active)card.setAttribute('aria-selected',String(card.dataset.reviewId===reviewId));if(!card.hidden)count++;}searchStatus.textContent=query?`${count} of ${total} loaded PRs${count?'':' · No matches'}`:'';}
  search.oninput=filterQueue;
  function selected(){return scopes.find(scope=>scope.key===position.boundary)||scopes[0]||null;}
  function select(scope,elementId=scope.target.id){position.boundary=scope.key;position.elementId=elementId;hooks.selectElement(elementId);navigate('hld');}
  function clearSource(){++sourceGeneration;source=null;sourceError='';sourceBusy=false;}
  function activate(value){
    if(active===value)return;active=value;desk.hidden=!value;doc.body.classList.toggle('standard-desk-active',value);
    if(!value){save();clearSource();lastKey='';}filterQueue();if(value)draw();
  }
  function navigate(next){
    if(!sections.includes(next)||!state?.details)return;
    const returningToLld=position.section==='source'&&position.sourceReturn==='lld'&&next==='lld';
    if(next==='source'&&position.section!=='source'){position.sourceReturn=position.section;position.sourceKind=position.section==='lld'?'lld':'hld';position.sourceId=null;}
    if(next!==position.section)clearSource();position.section=next;save();
    if(next==='lld'){
      const scope=selected(),id=position.elementId||scope?.target.id;
      const mapping=returningToLld?null:exactComponent(state.hld,id)||exactComponent(state.hld,scope?.source.id)||exactComponent(state.hld,id,'baseline')||exactComponent(state.hld,scope?.source.id,'baseline');
      position.lldUnavailable=!mapping&&!returningToLld;
      if(!position.lldUnavailable)hooks.openLld(mapping);
    }else if(next==='hld'||next==='changes')hooks.openHld();
    draw();if(next==='source'&&position.sourceKind!=='file')void loadSource();
  }
  function openFile(path){clearSource();if(position.section!=='source')position.sourceReturn=position.section;position.section='source';position.sourceKind='file';position.sourceId=path;save();draw();hooks.openFile(path);}
  function inspectImplementation(mapping){position.section='lld';position.lldUnavailable=false;clearSource();save();hooks.openLld(mapping);draw();}
  function evidenceItems(){
    if(position.sourceKind==='lld')return position.lldUnavailable?[]:hooks.lldEvidence();
    const current=selected();return current?hooks.hldEvidence(position.elementId||current.target.id):[];
  }
  async function loadSource(item=null){
    const items=evidenceItems();item=item||items.find(value=>`${value.design_id||''}:${value.id}`===position.sourceId)||items[0];
    clearSource();if(!item){draw();return;}
    position.sourceId=`${item.design_id||''}:${item.id}`;save();const generation=sourceGeneration,id=reviewId,kind=position.sourceKind;
    sourceBusy=true;draw();
    try{const value=await hooks.source(kind,item);if(generation!==sourceGeneration||id!==reviewId||!active||!state?.details)return;if(!value)throw Error('Exact source unavailable in the current authorized context.');if(!sourceMatches(value,item,id))throw Error('Source does not match the selected immutable evidence.');source=value;}
    catch(error){if(generation!==sourceGeneration||id!==reviewId||!active||!state?.details)return;sourceError=error.name==='AbortError'?'Source request superseded. Select evidence to load again.':error.message;}
    finally{if(generation===sourceGeneration&&id===reviewId){sourceBusy=false;draw();}}
  }
  function panel(label){const section=el('section','standard-panel');section.append(el('h2','',label));return section;}
  function map(){
    const box=panel('Where it fits'),scope=selected();
    if(!scope){box.append(el('p','standard-note',state.hldError?`HLD unavailable: ${state.hldError}`:state.loading.has('hld')?'Loading commit-bound architecture…':'No architectural boundaries established in the authorized scope.'));
      box.append(button(state.hldError?'Retry HLD':'Reload HLD',()=>hooks.reloadHld()));return box;}
    const view=model.view(scopes,scope.key,false),graph=el('div','standard-map');graph.setAttribute('aria-label','Bounded structural reference map');
    box.append(el('p','standard-note','Observed structural references, not runtime execution order.'));
    const origin=button(scope.source.name,()=>select(scope,scope.source.id),'standard-map-origin');
    // A source-only element has no relationship: never draw an invented arrow.
    graph.append(origin);const targets=el('div','standard-map-targets');
    for(const row of view.scopes){const target=button('',()=>select(row),'standard-map-target');target.setAttribute('aria-pressed',String(row.key===scope.key));
      if(row.source.id!==row.target.id)target.append(el('span','standard-map-arrow','↓'));
      target.append(el('strong','',row.target.name),el('span','standard-map-kind',row.source.id===row.target.id?'Element context':`${model.reference(row,'baseline')} → ${model.reference(row,'proposal')}`));targets.append(target);}
    graph.append(targets);box.append(graph);
    const controls=el('div','standard-map-controls'),picker=el('select');picker.setAttribute('aria-label','Architecture boundary');
    scopes.forEach((row,index)=>{const option=el('option','',`${row.source.name} → ${row.target.name}`);option.value=String(index);picker.append(option);});picker.value=String(scopes.indexOf(scope));picker.onchange=()=>select(scopes[Number(picker.value)]);
    const index=scopes.indexOf(scope),previous=button('←',()=>select(scopes[index-1])),next=button('→',()=>select(scopes[index+1]));previous.setAttribute('aria-label','Previous boundary');next.setAttribute('aria-label','Next boundary');previous.disabled=index<=0;next.disabled=index>=scopes.length-1;
    controls.append(picker,previous,next);box.append(controls,el('p','standard-note',`${view.start+1}–${view.start+view.scopes.length} of ${view.total} references from this origin · ${scopes.length} boundaries available`));
    return box;
  }
  function changes(){
    const grid=el('div','standard-brief-grid'),left=panel('What changes'),rows=changeRows(state.hld,state.files?.items||[]),start=Math.min(position.changePage*4,Math.max(0,Math.floor((rows.length-1)/4)*4));
    if(!rows.length)left.append(el('p','standard-note',state.loading.has('files')?'Loading authorized changes…':'No changes supplied in the loaded scope. Missing evidence is not proof of no change.'));
    for(const row of rows.slice(start,start+4)){
      const item=button('',()=>{if(row.path){openFile(row.path);return;}const scope=scopes.find(value=>row.relationId&&[...value.baseline,...value.proposal].some(relation=>relation.id===row.relationId))||scopes.find(value=>value.target.id===row.elementId||value.source.id===row.elementId);if(scope)select(scope,row.elementId);},'standard-change');
      item.disabled=!row.path&&!scopes.some(value=>value.target.id===row.elementId||value.source.id===row.elementId);
      item.append(el('span',`standard-change-mark standard-${row.kind}`,markers[row.kind]),el('strong','',row.title),el('span','standard-change-kind',row.kind==='uncertain'?'Unverified observation':row.kind),el('p','',row.reason));left.append(item);
    }
    if(rows.length>4){const pagination=el('div','standard-pagination'),prev=button('Previous',()=>{position.changePage=Math.max(0,position.changePage-1);draw();}),next=button('Next',()=>{position.changePage++;draw();});prev.disabled=start===0;next.disabled=start+4>=rows.length;pagination.append(prev,el('span','',`${start+1}–${Math.min(start+4,rows.length)} of ${rows.length}`),next);left.append(pagination);}
    if(state.files?.next_cursor)left.append(button('Load more changed files',hooks.moreFiles));
    grid.append(left,map());return grid;
  }
  function hld(){
    const grid=el('div','standard-brief-grid'),detail=panel('Selected boundary'),scope=selected();grid.append(map(),detail);
    if(!scope){detail.append(el('p','standard-note','Select a boundary after authorized HLD becomes available.'));return grid;}
    const elementId=position.elementId||scope.target.id,selectedElement=[...(state.hld?.proposal?.elements||[]),...(state.hld?.baseline?.elements||[])].find(item=>item.id===elementId);
    detail.append(el('p','standard-crumb',`${scope.source.name} → ${scope.target.name}`),el('h3','',selectedElement?.name||scope.target.name),el('p','standard-description',model.description(scope,'combined')));
    const sides=el('div','standard-snapshots');
    for(const side of ['baseline','proposal']){
      const version=state.hld?.[side],element=version?.elements?.find(value=>value.id===elementId),section=el('section');section.append(el('h4','',side==='baseline'?'Baseline':'Proposal'),el('p','standard-note',short(version?.snapshot)));
      if(element){section.append(el('p','',element.responsibility||'Responsibility not supplied.'),el('p','standard-note',(element.roots||[]).join(', ')||'No owning path supplied.'));
        const mapping=exactComponent(state.hld,element.id,side)||exactComponent(state.hld,scope.source.id,side);
        if(mapping)section.append(button(`Inspect ${mapping.component.name} · ${side} LLD`,()=>inspectImplementation(mapping)));
        else section.append(el('p','standard-note','No exact component mapping for LLD on this side.'));
      }else section.append(el('p','standard-note','Not observed in this snapshot. Absence outside coverage is not established.'));
      sides.append(section);
    }
    detail.append(sides,button('Inspect exact source',()=>navigate('source')));return grid;
  }
  function sourcePanel(){
    const box=panel('Source at the selected context');box.append(button(`← Back to ${position.sourceReturn==='changes'?'changes':position.sourceReturn.toUpperCase()}`,()=>navigate(position.sourceReturn)));
    const files=state.files?.items||[];
    if(files.length){const label=el('label','standard-source-picker','Changed file diff'),picker=el('select');picker.setAttribute('aria-label','Select changed file source');const empty=el('option','','Choose an authorized file');empty.value='';picker.append(empty);for(const file of files){const option=el('option','',file.path);option.value=file.path;picker.append(option);}picker.value=position.sourceKind==='file'?position.sourceId:'';picker.onchange=()=>{if(picker.value)openFile(picker.value);};label.append(picker);box.append(label);}
    if(position.sourceKind==='file'){box.append(el('p','standard-note',position.sourceId||'Select an authorized changed file.'));return box;}
    const items=evidenceItems();if(!items.length){box.append(el('p','standard-note',position.sourceKind==='lld'?'Select a declaration in LLD first. No retained source evidence is available for the current selection.':'No exact source span supports this boundary. User assertions are not code facts.'));return box;}
    const label=el('label','standard-source-picker','Exact evidence'),picker=el('select');picker.setAttribute('aria-label','Select exact source evidence');
    for(const [index,item] of items.entries()){const option=el('option','',`${item.path}:${item.start_line}–${item.end_line} · ${short(item.commit)}`);option.value=String(index);picker.append(option);}
    picker.value=String(Math.max(0,items.findIndex(value=>`${value.design_id||''}:${value.id}`===position.sourceId)));picker.onchange=()=>void loadSource(items[Number(picker.value)]);label.append(picker);box.append(label);
    const note=sourceBusy?'Loading exact commit-bound source…':sourceError?`Source unavailable: ${sourceError} Another revision is never substituted.`:source?`${source.path}:${source.start_line}–${source.end_line} · commit ${source.commit} · blob ${source.blob_id}${source.truncated?' · bounded excerpt':''}`:'Select evidence to load its exact source.';
    const output=el('p','standard-note',note);output.setAttribute('role','status');box.append(output);
    if(source){const pre=el('pre','standard-source-code'),code=el('code','',source.text);pre.append(code);box.append(pre);}else if(!sourceBusy)box.append(button('Load exact source',()=>void loadSource()));
    return box;
  }
  function drawEvidence(){
    evidence.hidden=!position.evidenceOpen;evidenceToggle.setAttribute('aria-expanded',String(Boolean(position.evidenceOpen)));evidence.replaceChildren();if(!position.evidenceOpen)return;
    evidence.append(el('h2','','Evidence & scope'),el('p','standard-note','Static observations do not establish runtime correctness or merge approval.'));
    const list=el('dl','standard-evidence-grid'),details=state.details||{};
    for(const [label,value] of [['Baseline',details.analysis_baseline||details.selected_baseline],['Proposal',details.proposal],['Review',state.reviewId],['Scope',details.scope_version],['HLD baseline',state.hld?.baseline?.id],['HLD proposal',state.hld?.proposal?.id]]){const row=el('div');row.append(el('dt','',label),el('dd','',value||'Unavailable'));list.append(row);}evidence.append(list);
    const coverage=state.hld?.delta?.coverage||details.public_coverage||state.hld?.proposal?.coverage;
    evidence.append(el('p','standard-note',coverage?.complete===true?'Declared static coverage complete. Runtime behavior remains unverified.':'Coverage incomplete or unverified.'));
    const gaps=[...new Set([...(details.public_coverage?.gaps||[]),...(coverage?.gaps||[]),...(state.hld?.baseline?.coverage?.gaps||[]),...(state.hld?.proposal?.coverage?.gaps||[])])];
    for(const gap of gaps)evidence.append(el('p','standard-note',gap));
    for(const question of state.hld?.delta?.questions||[]){const text=typeof question==='string'?question:question.question||question.reason;if(text)evidence.append(el('p','standard-note',`Recorded review question: ${text}`));}
  }
  function draw(){
    if(!active||!state)return;save();doc.body.dataset.standardSection=position.section;doc.body.dataset.standardSource=position.sourceKind||'';doc.body.dataset.standardLld=position.lldUnavailable?'unmapped':'mapped';
    crumb.textContent=`${pr?.repository?`${pr.repository.owner}/${pr.repository.name}`:pr?.repository_id||'Authorized repository'} · ${pr?.provider_pr_id||'Review'}`;
    title.textContent=pr?.title||`${pr?.provider_pr_id||'Pull request'} · review the change`;
    const files=state.files?.items||[],rows=changeRows(state.hld,files);summary.textContent=state.hld?`${rows.length} ${state.hld.delta?.changes?.length?'recorded design changes':'changed files'} across this exact comparison.`:files.length?`${files.length} loaded changed files. Architectural evidence ${state.loading.has('hld')?'is loading':'is not available'}.`:'Follow the observed changes, then inspect their exact evidence.';
    status.textContent=[state.error,state.details?.stale||state.progress?.stale?'Stale comparison · refresh before relying on these records':'',state.hldError?`HLD unavailable: ${state.hldError}`:''].filter(Boolean).join(' · ');status.hidden=!status.textContent;
    for(const [name,item] of tabButtons){item.setAttribute('aria-current',position.section===name?'page':'false');item.disabled=!state.details;}
    drawEvidence();content.replaceChildren();
    if(!state.details){content.append(el('p','standard-empty',state.error?`Evidence unavailable: ${state.error}`:'Loading the authorized comparison…'));return;}
    if(position.section==='changes')content.append(changes());else if(position.section==='hld')content.append(hld());else if(position.section==='source')content.append(sourcePanel());
    else if(position.section==='lld'&&position.lldUnavailable)content.append(el('p','standard-empty','No exact HLD component maps this boundary to LLD. Choose a mapped component in HLD; an unrelated implementation is never substituted.'));
  }
  function update(value){
    state=value.state;pr=value.pr;
    if(reviewId!==state.reviewId){save();reviewId=state.reviewId;position=positions.get(reviewId);clearSource();lastKey='';}
    if(!state.details&&state.error){positions.clear();position=positions.get(null);clearSource();}
    if(state.hld!==hldPayload){hldPayload=state.hld;scopes=state.hld?boundaries(state.hld,hooks.hldStatus(state.hld),model):[];dataVersion++;}
    if(state.files!==filesPayload){filesPayload=state.files;dataVersion++;}
    if(state.hld&&!scopes.some(scope=>scope.key===position.boundary)){position.boundary=scopes[0]?.key||null;position.elementId=scopes[0]?.target.id||null;}
    activate(value.active);
    const nextQueueKey=`${byId('pr-list').childElementCount}:${active}:${reviewId}`;if(nextQueueKey!==queueKey){queueKey=nextQueueKey;filterQueue();}
    const key=JSON.stringify([state.reviewId,Boolean(state.details),state.error,state.hldError,state.details?.stale,state.progress?.stale,dataVersion,state.lldDetail?.entry_id,[...state.loading],position]);
    if(key!==lastKey){lastKey=key;draw();}
  }
  function restoreSection(section,elementId=null,path=null){if(sections.includes(section)){position.section=section;if(section==='lld')position.lldUnavailable=false;if(elementId){position.elementId=elementId;position.boundary=scopes.find(scope=>scope.target.id===elementId||scope.source.id===elementId)?.key||position.boundary;}if(section==='source'&&path){position.sourceKind='file';position.sourceId=path;}save();draw();}}
  return {update,activate,navigate,restoreSection,openFile,get section(){return position.section;},get active(){return active;},clear(){positions.clear();clearSource();position=positions.get(null);draw();}};
}
const api={mount,changeRows,boundaries,exactComponent,sourceMatches,Positions};if(typeof module!=='undefined')module.exports=api;root.WorkspaceStandard=api;
})(typeof window==='undefined'?globalThis:window);
