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
