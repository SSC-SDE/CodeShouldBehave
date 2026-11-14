/* Presentation of authorized HLD records; no architectural inference. */
(function(root){
'use strict';
const kinds={system:'System',container:'Container',component:'Component',dependency:'Package',deployment:'Deployment',schema:'Data store',contract:'Public contract'};
const relations={declares_package_dependency:'declares',imports_declared_package:'imports',local_package_dependency:'local reference',declares_public_contract:'public interface',has_deployment_configuration:'deployment configuration'};
const markers={added:'+ ',removed:'− ',modified:'~ ',uncertain:'? '};
const label=value=>String(value??'').replace(/[\u0000-\u001f\u007f"<>`|&\\%]/g,' ');
const short=value=>String(value||'unavailable').slice(0,8);
const observed=(scope,side)=>scope[side]?.length>0;
function state(scope){
  if(['added','removed','modified'].includes(scope.status))return scope.status;
  return scope.changed?'uncertain':'same';
}
function boundaries(payload,{mode='combined',changedOnly=true,status=new Map()}={}){
  const sides=mode==='accepted'?[['accepted',payload.accepted]]:[['baseline',payload.baseline],['proposal',payload.proposal]];
  const elements=new Map(sides.flatMap(([,version])=>version?.elements||[]).map(item=>[item.id,item])),scopes=new Map();
  const changed=key=>['added','removed','modified'].includes(status.get(key));
  const beforeIds=new Set((payload.baseline?.elements||[]).map(element=>element.id)),afterIds=new Set((payload.proposal?.elements||[]).map(element=>element.id));
  const presenceDiffers=key=>beforeIds.has(key)!==afterIds.has(key);
  const reasons=new Map();
  for(const change of payload.delta?.changes||[])if(change.kind!=='uncertain'&&!reasons.has(change.element_id))reasons.set(change.element_id,change.reason);
  for(const [side,version] of sides)for(const relation of version?.relations||[]){
    const source=elements.get(relation.from),target=elements.get(relation.to);if(!source||!target)continue;
    if(target.kind==='contract'&&mode!=='accepted'&&!changed(target.id))continue;
    const key=`${source.id}\0${target.id}`;
    if(!scopes.has(key))scopes.set(key,{key,source,target,baseline:[],proposal:[],accepted:[]});
    scopes.get(key)[side].push(relation);
  }
  const represented=new Set([...scopes.values()].flatMap(scope=>[scope.source.id,scope.target.id]));
  const targets=new Set([...scopes.values()].map(scope=>scope.target.id));
  const explicit=new Set((payload.delta?.changes||[]).map(change=>change.element_id).filter(Boolean));
  for(const element of elements.values()){
    if(represented.has(element.id)&&!explicit.has(element.id))continue;
    if(targets.has(element.id))continue;
    if(element.kind==='contract'&&mode!=='accepted'&&!changed(element.id))continue;
    if(changedOnly&&mode!=='accepted'&&!changed(element.id)&&!presenceDiffers(element.id))continue;
    const key=`${element.id}\0${element.id}`;scopes.set(key,{key,source:element,target:element,baseline:[],proposal:[],accepted:[]});
  }
  const fingerprint=records=>records.map(item=>JSON.stringify([item.kind,item.protocol,item.synchrony,item.resolution])).sort().join('\n');
  for(const scope of scopes.values()){
    scope.status=status.get(scope.target.id);scope.changed=mode!=='accepted'&&(changed(scope.target.id)||fingerprint(scope.baseline)!==fingerprint(scope.proposal)||(scope.source.id===scope.target.id&&presenceDiffers(scope.target.id)));
    scope.reason=reasons.get(scope.target.id);
  }
  return [...scopes.values()].filter(scope=>mode==='accepted'||!changedOnly||scope.changed).sort((a,b)=>Number(b.changed)-Number(a.changed)||Number(a.target.kind==='contract')-Number(b.target.kind==='contract')||String(a.target.name).localeCompare(String(b.target.name)));
}
function description(scope,mode){
  if(mode==='accepted')return 'Reference in the accepted architecture; this is not a claim about runtime execution.';
  if(scope.reason)return scope.reason;
  if(scope.source.id===scope.target.id)return scope.changed?'Element evidence differs between the snapshots. Inspect the exact records before drawing behavioral conclusions.':'Retained element context. No confirmed change is established by these observations.';
  const before=observed(scope,'baseline'),after=observed(scope,'proposal');
  if(!before&&after)return 'Reference observed in the proposal, not observed in the covered baseline. Architectural addition is not established by observation alone.';
  if(before&&!after)return 'Reference observed in the baseline, not observed in the covered proposal. Architectural removal is not established by observation alone.';
  return 'Reference observed in both snapshots. Runtime order, availability and error behavior require implementation review.';
}
function reference(scope,side){
  const records=scope[side]||[];
  if(!records.length)return 'Not observed in scope';
  return [...new Set(records.map(record=>relations[record.kind]||String(record.kind).replaceAll('_',' ')))].join(' + ');
}
function view(scopes,selectedKey,focused){
  const selected=scopes.find(scope=>scope.key===selectedKey)||scopes[0];
  if(!selected)return {selected:null,scopes:[],start:0,total:0};
  const group=scopes.filter(scope=>scope.source.id===selected.source.id);
  const index=group.findIndex(scope=>scope.key===selected.key),start=focused?index:Math.floor(index/5)*5;
  return {selected,scopes:focused?[selected]:group.slice(start,start+5),start,total:group.length};
}
function diagram(payload,scopes,selected,mode,{narrow=false,palette={},selection=null}={}){
  const versions=mode==='combined'?[payload.baseline,payload.proposal]:[payload[mode]];
  const allowed=new Map(versions.flatMap(version=>version?.elements||[]).map(element=>[element.id,element]));
  const nodes=[],indexes=new Map(),lines=['%%{init: {"fontFamily":"Inter, sans-serif","flowchart":{"nodeSpacing":20,"rankSpacing":42,"curve":"linear","padding":14}}}%%',`flowchart ${narrow?'TB':'LR'}`];
  const add=(element,scope)=>{
    element=allowed.get(element.id);if(!element)return null;
    if(indexes.has(element.id))return indexes.get(element.id);
    const index=nodes.length;nodes.push(element);indexes.set(element.id,index);
    const status=mode==='baseline'||mode==='accepted'?'same':element.id===scope.target.id?state(scope):'same';
    const marker=markers[status]||'';
    lines.push(`fdnode${index}["${label(marker+element.name)}<br/>${label(kinds[element.kind]||element.kind)}"]:::${status}`);
    if(element.id===selection)lines.push(`class fdnode${index} selected`);
    return index;
  };
  let connected=0;
  for(const scope of scopes){
    const from=add(scope.source,scope),to=add(scope.target,scope);
    if(from===null||to===null||from===to)continue;
    const records=mode==='combined'?[...scope.baseline,...scope.proposal]:scope[mode]||[];
    if(!records.length)continue;
    const unresolved=records.some(record=>!['observed_structure','resolved_reference'].includes(record.resolution));
    const uncertainDifference=mode==='combined'&&scope.changed&&state(scope)==='uncertain';
    const selectedEdge=scope.key===selected?.key;
    let text=selectedEdge?[...new Set(records.map(record=>relations[record.kind]||String(record.kind).replaceAll('_',' ')))].join(' + '):'';
    if(selectedEdge&&uncertainDifference)text=(!scope.baseline.length?'only observed after · ':!scope.proposal.length?'only observed before · ':'')+text;
    if(selectedEdge&&unresolved)text='? '+text;
    lines.push(`fdnode${from} ${unresolved||uncertainDifference?'-.->':'-->'}${text?`|"${label(text)}"|`:''} fdnode${to}`);connected++;
  }
  if(!nodes.length)lines.push('empty["No element observed in this snapshot"]');
  const colors={same:palette.line,added:palette.green,removed:palette.red,modified:palette.amber,uncertain:palette.muted};
  for(const [name,color] of Object.entries(colors))lines.push(`classDef ${name} fill:${palette.panel},stroke:${color},color:${palette.text}${name==='uncertain'?',stroke-dasharray:4 3':''}`);
  lines.push(`classDef selected stroke:${palette.accent},stroke-width:2.2px`);
  return {source:lines.join('\n'),nodes,connected};
}
const api={kinds,short,state,boundaries,description,reference,view,diagram};
if(typeof module!=='undefined')module.exports=api;
root.WorkspaceFocusModel=api;
})(typeof window==='undefined'?globalThis:window);
