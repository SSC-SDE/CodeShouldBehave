/* Focus-only layout. Existing controls, source requests and evidence remain authoritative. */
(function(root){
'use strict';
function mount(hooks){
  const model=root.WorkspaceFocusModel,byId=id=>document.getElementById(id);
  const el=(tag,cls,text)=>{const item=document.createElement(tag);if(cls)item.className=cls;if(text!==undefined)item.textContent=text;return item;};
  const button=(text,action,cls='quiet')=>{const item=el('button',cls,text);item.type='button';item.onclick=action;return item;};
  const old=document.querySelector('.hld-workspace'),diagrams=byId('hld-diagrams'),inspector=byId('hld-inspector');
  const desk=el('section','focus-desk');desk.hidden=true;desk.setAttribute('aria-label','High-level review desk');old.before(desk);
  const nav=el('details','focus-desk-nav'),navSummary=el('summary'),count=el('span');
  navSummary.append(document.createTextNode('Boundaries '),count);
  const navBody=el('nav','focus-desk-nav-body');navBody.setAttribute('aria-label','HLD boundaries');
  const search=el('input');search.type='search';search.placeholder='Find boundary';search.setAttribute('aria-label','Find HLD boundary');
  const list=el('div','focus-desk-changes'),extras=el('details','focus-desk-elements');extras.append(el('summary','','All elements'));
  navBody.append(search,list,extras);nav.append(navSummary,navBody);
  nav.onkeydown=event=>{if(event.key==='Escape'){nav.open=false;navSummary.focus();}};
  const canvas=el('section','focus-desk-canvas'),toolbar=el('header','focus-desk-toolbar'),tabs=el('nav','focus-desk-tabs');tabs.setAttribute('aria-label','HLD canvas scope');
  const overviewTab=button('Overview',()=>{focused=false;hooks.redraw();}),boundaryTab=button('Selected boundary',()=>{focused=true;hooks.redraw();});tabs.append(overviewTab,boundaryTab);
  const options=el('div','focus-desk-options'),comparison=el('label','architecture-controls');comparison.setAttribute('aria-label','HLD comparison controls');
  const mode=el('select');mode.setAttribute('aria-label','HLD comparison view');
  for(const [value,text] of [['combined','Change map'],['compare','Side by side'],['baseline','Baseline'],['proposal','Proposal map'],['accepted','Accepted architecture']]){const option=el('option','',text);option.value=value;mode.append(option);}
  mode.onchange=()=>hooks.mode(mode.value);comparison.append(mode);
  const zoom=el('output','focus-desk-zoom');zoom.setAttribute('aria-live','polite');
  const zoomAction=(text,label)=>button(text,()=>{diagrams.querySelectorAll(`.diagram-tools button[aria-label="${label}"]`).forEach(item=>item.click());syncZoom();});
  const fit=zoomAction('Fit','Fit the entire architecture diagram'),minus=zoomAction('−','Zoom diagram out'),plus=zoomAction('+','Zoom diagram in');
  for(const item of [fit,minus,zoom,plus])item.classList.add('focus-desk-zoom-control');
  minus.setAttribute('aria-label','Zoom HLD out');plus.setAttribute('aria-label','Zoom HLD in');
  const evidence=button('Evidence',()=>hooks.evidence(inspector.hidden||inspector.classList.contains('preference-hidden')));evidence.setAttribute('aria-controls','hld-inspector');
  options.append(comparison,fit,minus,zoom,plus,evidence);toolbar.append(nav,tabs,options);
  const intro=el('header','focus-desk-map-heading'),title=el('h3'),position=el('span','focus-desk-position');intro.append(title,position);
  const metadata=el('details','focus-desk-metadata');metadata.append(el('summary','','Coverage and design details'));
  const footer=el('footer','focus-desk-footer'),previous=button('← Previous',()=>step(-1)),next=button('Next →',()=>step(1));previous.setAttribute('aria-label','Previous HLD boundary');next.setAttribute('aria-label','Next HLD boundary');footer.append(previous,next);
  canvas.append(toolbar,intro,diagrams,footer,metadata);desk.append(canvas);
  const homes=new Map();
  function move(item,parent){if(!item)return;if(!homes.has(item)){const mark=document.createComment('Review desk control home');item.before(mark);homes.set(item,mark);}if(item.parentNode!==parent)parent.append(item);}
  // The diagrams were adopted above; record their original home explicitly.
  const diagramHome=document.createComment('Review desk diagram home');old.querySelector('.hld-map-canvas').append(diagramHome);homes.set(diagrams,diagramHome);
  diagramHome.after(diagrams);
  let active=false,focused=window.matchMedia('(max-width:700px)').matches,current=null,all=[],inspectorKey=null,review=null,graphFocus=null;
  function activate(value){
    if(active===value)return;active=value;desk.hidden=!value;old.hidden=value;inspectorKey=null;
    if(value){
      canvas.insertBefore(diagrams,footer);showInspector(false);nav.open=false;metadata.open=false;
      move(inspector,desk);inspector.classList.add('focus-desk-inspector');diagrams.classList.add('focus-desk-diagrams');
      move(document.querySelector('.hld-map-toolbar'),extras);move(byId('hld-element-list'),extras);
      for(const item of [document.querySelector('.review-commits'),byId('hld-identity'),document.querySelector('.hld-drift'),byId('hld-coverage'),document.querySelector('.hld-provenance')])move(item,metadata);
    }else{
      for(const [item,home] of homes)if(item.parentNode!==home.parentNode)home.after(item);
      inspector.classList.remove('focus-desk-inspector');diagrams.classList.remove('focus-desk-diagrams');inspector.hidden=false;
    }
  }
  function syncZoom(){
    const values=[...diagrams.querySelectorAll('.diagram-zoom')].map(item=>Number.parseInt(item.textContent,10)).filter(Number.isFinite);
    const text=!values.length?'—':Math.min(...values)===Math.max(...values)?`${values[0]}%`:`${Math.min(...values)}–${Math.max(...values)}%`;
    if(zoom.textContent!==text)zoom.textContent=text;for(const item of [fit,minus,plus])item.disabled=!values.length;
  }
  new MutationObserver(syncZoom).observe(diagrams,{subtree:true,childList:true});syncZoom();
  function step(delta){const index=all.findIndex(scope=>scope.key===current?.key),scope=all[index+delta];if(scope)hooks.scope(scope);}
  function syncControls(){for(const item of [nav,tabs,comparison,fit,minus,zoom,plus])item.hidden=all.length===0;}
  function drawNav(){
    const scroll=list.scrollTop,focusedKey=document.activeElement?._focusScopeKey;list.replaceChildren();
    const query=search.value.trim().toLowerCase(),visible=all.filter(scope=>`${scope.source.name} ${scope.target.name} ${(scope.target.roots||[]).join(' ')}`.toLowerCase().includes(query));
    const categories=[...new Set(visible.map(scope=>model.kinds[scope.target.kind]||scope.target.kind))],multipleSources=new Set(all.map(record=>record.source.id)).size>1;
    for(const category of categories){
      const group=el('section','focus-desk-group');group.append(el('h4','',category));
      for(const scope of visible.filter(item=>(model.kinds[item.target.kind]||item.target.kind)===category)){
        const item=button('',()=>{nav.open=false;navSummary.focus();hooks.scope(scope);},'focus-desk-boundary');item._focusScopeKey=scope.key;item.setAttribute('aria-current',String(scope.key===current?.key));
        item.append(el('span','',scope.target.name),el('span','focus-desk-marker',({added:'+',removed:'−',modified:'~',uncertain:'?'}[model.state(scope)]||'')));
        if(multipleSources)item.title=`${scope.source.name} → ${scope.target.name}`;
        group.append(item);
      }
      list.append(group);
    }
    if(!visible.length)list.append(el('p','status','No boundaries match this search.'));
    list.scrollTop=scroll;
    if(focusedKey){const item=[...list.querySelectorAll('button')].find(entry=>entry._focusScopeKey===focusedKey);item?.focus({preventScroll:true});}
  }
  search.oninput=drawNav;
  function update({scopes,selectedKey,mode:comparisonMode,reviewId,hasAccepted}){
    if(review!==reviewId){review=reviewId;search.value='';inspectorKey=null;focused=window.matchMedia('(max-width:700px)').matches;showInspector(false);nav.open=false;metadata.open=false;}
    all=scopes;const view=model.view(scopes,selectedKey,focused);current=view.selected;count.textContent=`(${scopes.length})`;mode.value=comparisonMode;mode.querySelector('[value=accepted]').disabled=!hasAccepted;
    syncControls();
    overviewTab.classList.toggle('active',!focused);boundaryTab.classList.toggle('active',focused);
    overviewTab.setAttribute('aria-pressed',String(!focused));boundaryTab.setAttribute('aria-pressed',String(focused));boundaryTab.disabled=!current;
    title.textContent=current?(focused?`${current.source.name} → ${current.target.name}`:current.source.name):'High-level design';
    position.textContent=current?(focused?'':`${view.start+1}–${view.start+view.scopes.length} of ${view.total} boundaries`):'';
    intro.hidden=!current;footer.hidden=scopes.length<2;
    const index=scopes.indexOf(current);previous.disabled=index<=0;next.disabled=index<0||index>=scopes.length-1;
    drawNav();showInspector(!inspector.hidden);return view;
  }
  function showInspector(open){inspector.hidden=!open;evidence.setAttribute('aria-expanded',String(open&&!inspector.classList.contains('preference-hidden')));}
  function empty(message,{retry=false}={}){
    all=[];current=null;inspectorKey=null;review=null;search.value='';list.replaceChildren();count.textContent='(0)';intro.hidden=true;footer.hidden=true;nav.open=false;if(active)showInspector(false);syncControls();
    diagrams.replaceChildren(el('p','status',message));
    if(retry&&!byId('hld-reload').disabled)diagrams.append(button('Reload HLD',()=>byId('hld-reload').click()));
    previous.disabled=true;next.disabled=true;
  }
  function decorateInspector({key,selected,scope,payload,mode:comparisonMode,evidence:spans,onSource}){
    inspectorKey=key;
    const heading=inspector.querySelector('h3');if(!heading)return null;
    const top=el('header','focus-desk-inspector-heading'),close=button('×',()=>{showInspector(false);evidence.focus();});close.setAttribute('aria-label','Close HLD evidence');top.append(el('span','','Evidence'),close);inspector.prepend(top);
    heading.classList.add('focus-desk-selected-name');heading.before(el('span','focus-desk-selected-kind',model.kinds[selected.kind]||selected.kind));
    const status=model.state(scope),statusText=comparisonMode==='accepted'?'Accepted architecture':status==='same'?'Shared context':status==='uncertain'?'? Observation difference · unverified':({added:'+ Added',removed:'− Removed',modified:'~ Modified'}[status]);
    const tag=el('p','focus-desk-selected-status',selected.id===scope.target.id?statusText:'Component context');heading.after(tag);
    const explanation=el('p','focus-desk-description',selected.id===scope.target.id?model.description(scope,comparisonMode):selected.responsibility||'Component responsibility is not established. Inspect the retained source evidence.');tag.after(explanation);
    const snapshots=el('div','focus-desk-snapshots');
    for(const [side,label] of comparisonMode==='accepted'?[['accepted','Accepted']]:[['baseline','Before'],['proposal','After']]){
      const cell=el('section'),present=payload[side]?.elements?.some(item=>item.id===selected.id);
      const text=scope.source.id!==scope.target.id&&selected.id===scope.target.id?model.reference(scope,side):present?'Element observed':'Not observed in scope';
      cell.append(el('span','',label),el('strong','',text));snapshots.append(cell);
    }
    explanation.after(snapshots);
    const sources=inspector.querySelector('.hld-source-evidence'),sourceStatus=byId('hld-source-status'),code=byId('hld-source-text');
    const details=el('details','focus-desk-selection-details');details.append(el('summary','','Design details'));
    for(const item of [...inspector.children])if(![top,heading,tag,explanation,snapshots,sources,sourceStatus,code,byId('hld-zoom-lld')].includes(item)&&!item.classList.contains('focus-desk-selected-kind'))details.append(item);
    inspector.append(details);
    sources.open=true;sources.querySelector('summary').textContent='Source evidence';
    sources.querySelectorAll('button').forEach(item=>item.remove());
    for(const item of spans){
      const entry=button('',()=>{for(const peer of sources.querySelectorAll('button'))peer.setAttribute('aria-current',String(peer===entry));onSource(item);},'focus-desk-source');entry.title=`${item.path}:${item.start_line}-${item.end_line} · ${item.commit} · ${item.relationship_type} · ${item.resolution}`;
      entry.append(el('span','',item.path),el('small','',`Lines ${item.start_line}–${item.end_line} · ${model.short(item.commit)}`));sources.append(entry);
    }
    sources.append(sourceStatus,code);
    const lld=byId('hld-zoom-lld');if(lld){lld.classList.add('focus-desk-lld-action');lld.textContent='Inspect LLD ↗';inspector.append(lld);}
    const first=spans.find(item=>item.commit===payload.proposal?.snapshot&&/\.(rs|[cm]?[jt]sx?|py|go|java|rb|cs|cpp|c|h)$/.test(item.path))||spans.find(item=>item.commit===payload.proposal?.snapshot)||spans[0];
    if(first){const index=spans.indexOf(first);sources.querySelectorAll('button')[index]?.setAttribute('aria-current','true');}
    return first;
  }
  function decorateNodes(host,nodes,selection){
    const svg=host.querySelector('svg');if(svg)svg.setAttribute('role','group');
    host.querySelectorAll('g.node').forEach(item=>{
      const match=item.id.match(/-fdnode(\d+)-/),record=match?nodes[Number(match[1])]:null;if(!record)return;
      item.dataset.designElement=record.id;item.classList.toggle('selected',record.id===selection);item.setAttribute('aria-current',String(record.id===selection));item.setAttribute('role','button');item.setAttribute('tabindex','0');item.setAttribute('aria-label',`Inspect ${record.name}, ${record.kind}`);
      const select=()=>{if(document.activeElement===item)graphFocus=record.id;hooks.element(record.id);};item.onclick=select;item.onkeydown=event=>{if(event.key==='Enter'||event.key===' '){event.preventDefault();select();}};
      if(graphFocus===record.id){item.focus({preventScroll:true});graphFocus=null;}
    });
  }
  return {activate,update,empty,decorateInspector,decorateNodes,showInspector,get active(){return active;},get focused(){return focused;},get selected(){return current;},inspectorCurrent:key=>key===inspectorKey,invalidateInspector(){inspectorKey=null;},emptyInspector(key,message){inspectorKey=key;inspector.replaceChildren(el('p','focus-desk-description',message));},narrow(){if(window.matchMedia('(max-width:700px)').matches)focused=true;}};
}
const api={mount};if(typeof module!=='undefined')module.exports=api;root.WorkspaceFocusDesk=api;
})(typeof window==='undefined'?globalThis:window);
