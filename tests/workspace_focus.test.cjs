'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const model=require('../src/workspace-focus-model.js');
const element=(id,kind='dependency',name=id)=>({id,kind,name});
const relation=(to,extra={})=>({from:'api',to,kind:'imports_declared_package',resolution:'resolved_reference',...extra});
function fixture(count=1){
  const elements=[element('api','component'),...Array.from({length:count},(_,i)=>element(`package-${i}`))];
  return {baseline:{elements:structuredClone(elements),relations:[]},proposal:{elements,relations:elements.slice(1).map(item=>relation(item.id))},delta:{changes:[]}};
}
test('observed references remain uncertain rather than claiming architectural additions',()=>{
  const payload=fixture(),[scope]=model.boundaries(payload);
  assert.equal(model.state(scope),'uncertain');
  assert.match(model.description(scope,'combined'),/addition is not established/);
  const graph=model.diagram(payload,[scope],scope,'combined');
  assert.equal(graph.connected,1);assert.match(graph.source,/only observed after/);
  payload.baseline.relations=structuredClone(payload.proposal.relations);
  assert.deepEqual(model.boundaries(payload),[]);
});
test('unchanged uncertain public contracts do not flood changed boundaries',()=>{
  const payload=fixture();
  for(const side of ['baseline','proposal']){
    payload[side].elements.push(element('contract','contract'));
    payload[side].relations.push(relation('contract',{kind:'declares_public_contract'}));
  }
  const status=new Map([['contract','uncertain']]);
  assert.deepEqual(model.boundaries(payload,{status}).map(scope=>scope.target.id),['package-0']);
  status.set('contract','modified');
  payload.delta.changes=[{element_id:'contract',kind:'modified',reason:'Public inputs changed'}];
  const scope=model.boundaries(payload,{status}).find(item=>item.target.id==='contract');
  assert.equal(model.state(scope),'modified');assert.equal(model.description(scope,'combined'),'Public inputs changed');
});
test('baseline and accepted diagrams retain their own names and cannot borrow proposal-only nodes',()=>{
  const payload=fixture();payload.baseline.elements[0].name='Original API';payload.proposal.elements[0].name='New API';
  payload.baseline.elements.pop();payload.accepted={elements:[element('api','component','Approved API')],relations:[]};
  const scopes=model.boundaries(payload),selected=scopes[0];
  const before=model.diagram(payload,scopes,selected,'baseline');
  assert.deepEqual(before.nodes.map(item=>item.name),['Original API']);assert.equal(before.connected,0);
  const acceptedScopes=model.boundaries(payload,{mode:'accepted'});
  assert.deepEqual(model.diagram(payload,acceptedScopes,acceptedScopes[0],'accepted').nodes.map(item=>item.name),['Approved API']);
  assert.equal(model.state(acceptedScopes[0]),'same');
});
test('overview pages bound graph density without making any boundary unreachable',()=>{
  const payload=fixture(13),scopes=model.boundaries(payload),seen=new Set();
  for(const selected of scopes){
    const page=model.view(scopes,selected.key,false);
    assert.equal(page.total,13);assert.ok(page.scopes.length<=5);
    assert.ok(page.scopes.includes(selected));
    page.scopes.forEach(scope=>seen.add(scope.key));
    assert.ok(model.diagram(payload,page.scopes,selected,'proposal').nodes.length<=6);
    assert.deepEqual(model.view(scopes,selected.key,true).scopes,[selected]);
  }
  assert.equal(seen.size,scopes.length);
});
test('repository-controlled labels cannot break out into Mermaid directives or HTML',()=>{
  const payload=fixture();payload.proposal.elements[1].name='x"]\nclick evil "javascript:alert(1)"\n%%{init: {}}%%<script>&|';
  const scopes=model.boundaries(payload),graph=model.diagram(payload,scopes,scopes[0],'proposal');
  assert.doesNotMatch(graph.source,/\nclick |<script>|\n%%\{init/);
  assert.equal((graph.source.match(/%%\{init/g)||[]).length,1);
  assert.ok(payload.proposal.elements[1].name.includes('<script>'));
});
test('missing evidence stays empty and dangling relations do not invent nodes',()=>{
  assert.deepEqual(model.boundaries({}),[]);
  assert.deepEqual(model.view([],null,false),{selected:null,scopes:[],start:0,total:0});
  const payload=fixture();payload.proposal.relations=[relation('missing')];
  assert.deepEqual(model.boundaries(payload),[]);
});

function deskHarness(){
  const {readFileSync}=require('node:fs'),vm=require('node:vm');
  let document;
  class Element {
    constructor(tag){this.tagName=tag;this.children=[];this.attributes={};this.className='';this.value='';this.hidden=false;this.open=false;this.scrollTop=0;this.dataset={};}
    get classList(){return {contains:name=>this.className.split(' ').includes(name),add:name=>this.classList.toggle(name,true),remove:name=>this.classList.toggle(name,false),toggle:(name,on)=>{const names=new Set(this.className.split(' ').filter(Boolean));if(on)names.add(name);else names.delete(name);this.className=[...names].join(' ');}};}
    remove(){if(this.parentNode){this.parentNode.children.splice(this.parentNode.children.indexOf(this),1);this.parentNode=null;}}
    append(...items){for(const item of items){item.remove();item.parentNode=this;this.children.push(item);}}
    before(item){this.parentNode.insertBefore(item,this);}
    after(item){const parent=this.parentNode;item.remove();item.parentNode=parent;parent.children.splice(parent.children.indexOf(this)+1,0,item);}
    insertBefore(item,reference){item.remove();item.parentNode=this;this.children.splice(this.children.indexOf(reference),0,item);}
    replaceChildren(...items){for(const child of [...this.children])child.remove();this.append(...items);}
    setAttribute(name,value){this.attributes[name]=value;}
    getAttribute(name){return this.attributes[name];}
    querySelectorAll(selector){
      const matches=item=>selector.startsWith('.')?item.classList.contains(selector.slice(1)):selector.startsWith('#')?item.id===selector.slice(1):selector==='[value=accepted]'?item.value==='accepted':item.tagName===selector;
      return this.children.flatMap(child=>[...(matches(child)?[child]:[]),...child.querySelectorAll(selector)]);
    }
    querySelector(selector){return this.querySelectorAll(selector)[0]||null;}
    focus(){document.activeElement=this;}
    click(){this.onclick?.();}
  }
  const root=new Element('main'),add=(parent,tag,cls,id)=>{const item=new Element(tag);item.className=cls||'';item.id=id;parent.append(item);return item;};
  const commits=add(root,'p','review-commits'),old=add(root,'section','hld-workspace'),map=add(old,'section','hld-map-canvas');
  const diagrams=add(map,'div','','hld-diagrams'),inspector=add(old,'section','','hld-inspector');
  add(old,'div','hld-map-toolbar');add(old,'div','','hld-element-list');add(root,'div','','hld-identity');add(root,'div','','hld-coverage');add(root,'button','','hld-reload');
  document={activeElement:null,createElement:tag=>new Element(tag),createComment:()=>new Element('comment'),createTextNode:text=>{const item=new Element('text');item.textContent=text;return item;},querySelector:selector=>root.querySelector(selector),getElementById:id=>root.querySelector('#'+id)};
  const window={WorkspaceFocusModel:model,matchMedia:()=>({matches:false})};
  vm.runInNewContext(readFileSync(require('node:path').join(__dirname,'../src/workspace-focus.js'),'utf8'),{window,document,MutationObserver:class{observe(){}}});
  let desk,selection=null;
  desk=window.WorkspaceFocusDesk.mount({redraw(){},scope:scope=>{selection=scope;desk.showInspector(true);},element(){},mode(){},evidence:open=>desk.showInspector(open)});
  const scopes=model.boundaries(fixture(7)),update=(reviewId='review-a',nextScopes=scopes)=>desk.update({scopes:nextScopes,selectedKey:nextScopes[0]?.key,mode:'combined',reviewId,hasAccepted:false});
  return {desk,root,old,map,diagrams,inspector,commits,update,scopes,get selection(){return selection;}};
}

test('Focus HLD uses folded navigation and opens evidence only on action or selection',()=>{
  const h=deskHarness();h.desk.activate(true);h.update();
  const nav=h.root.querySelector('.focus-desk-nav'),metadata=h.root.querySelector('.focus-desk-metadata');
  const evidence=h.root.querySelectorAll('button').find(item=>item.textContent==='Evidence');
  assert.equal(nav.tagName,'details');assert.equal(nav.open,false);assert.equal(metadata.open,false);
  assert.equal(h.inspector.hidden,true);assert.equal(evidence.getAttribute('aria-expanded'),'false');
  evidence.click();assert.equal(h.inspector.hidden,false);
  assert.equal(h.inspector.parentNode,h.root.querySelector('.focus-desk'));
  h.update();assert.equal(h.inspector.hidden,false);
  h.update('review-b');assert.equal(h.inspector.hidden,true);
  nav.open=true;h.root.querySelector('.focus-desk-boundary').click();
  assert.equal(h.selection,h.scopes[0]);assert.equal(nav.open,false);assert.equal(h.inspector.hidden,false);
});

test('leaving Focus restores controls and empty Focus state does not hide Standard evidence',()=>{
  const h=deskHarness();h.desk.activate(true);h.update();
  assert.equal(h.commits.parentNode,h.root.querySelector('.focus-desk-metadata'));
  h.desk.activate(false);
  assert.equal(h.inspector.parentNode,h.old);assert.equal(h.diagrams.parentNode,h.map);
  assert.equal(h.commits.parentNode,h.root);assert.equal(h.inspector.hidden,false);
  h.desk.empty('No authorized HLD evidence.');
  assert.equal(h.inspector.hidden,false);
});

test('empty HLD hides graph controls reversibly and offers retry only for load failure',()=>{
  const h=deskHarness();h.desk.activate(true);h.update('review-a',[]);
  const controls=[h.root.querySelector('.focus-desk-nav'),h.root.querySelector('.focus-desk-tabs'),h.root.querySelector('.architecture-controls'),...h.root.querySelectorAll('.focus-desk-zoom-control')];
  assert.ok(controls.every(item=>item.hidden));
  assert.equal(h.root.querySelectorAll('button').find(item=>item.textContent==='Evidence').hidden,false);
  assert.equal(h.root.querySelector('.focus-desk-metadata').hidden,false);
  h.update();assert.ok(controls.every(item=>!item.hidden));
  h.desk.empty('Loading HLD…');assert.ok(controls.every(item=>item.hidden));assert.equal(h.diagrams.querySelector('button'),null);
  h.desk.empty('HLD unavailable.',{retry:true});assert.equal(h.diagrams.querySelector('button').textContent,'Reload HLD');
  h.root.querySelector('#hld-reload').disabled=true;
  h.desk.empty('HLD unavailable.',{retry:true});assert.equal(h.diagrams.querySelector('button'),null);
  h.update();assert.ok(controls.every(item=>!item.hidden));
});
