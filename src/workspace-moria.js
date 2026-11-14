/* Decorative colored text only; review evidence and application theme are untouched. */
(function(root){
'use strict';
function assetURL(scriptURL,pageURL){
 const page=new URL(pageURL),url=new URL('workspace-moria.json',scriptURL);
 if(url.origin!==page.origin)throw Error('Backdrop assets must be same-origin');
 return url;
}
function validateArtwork(art){
 if(art?.columns!==288||art.lines!==120||art.colors!==128||!Array.isArray(art.rows)||art.rows.length!==120)throw Error('Invalid backdrop dimensions');
 for(const row of art.rows){
  if(!Array.isArray(row)||!row.length)throw Error('Invalid backdrop row');
  let columns=0;
  for(const run of row){
   if(!Array.isArray(run)||run.length!==2||!Number.isInteger(run[0])||run[0]<0||run[0]>=128||typeof run[1]!=='string'||!run[1].length||/[\u0000-\u001f\u007f-\u009f]/.test(run[1]))throw Error('Invalid backdrop text');
   columns+=run[1].length;
  }
  if(columns!==288)throw Error('Invalid backdrop row width');
 }
 return art;
}
const api={assetURL,validateArtwork};
if(typeof module!=='undefined')module.exports=api;
if(typeof document==='undefined')return;
root.WorkspaceMoria=api;
if(document.getElementById('workspace-moria'))return;
const scriptURL=document.currentScript.src;
const host=document.createElement('div');
host.id='workspace-moria';host.className='workspace-moria preference-hidden';
host.setAttribute('aria-hidden','true');host.setAttribute('inert','');
const scene=document.createElement('div');scene.className='moria-scene';
const grid=document.createElement('pre');scene.append(grid);
for(const name of ['fire','staff','bridge','embers']){
 const layer=document.createElement('div');layer.className=`moria-${name}`;
 if(name==='embers')for(let index=0;index<6;index++){
  const ember=document.createElement('span');ember.className='moria-ember';ember.textContent='·';layer.append(ember);
 }
 scene.append(layer);
}
host.append(scene);document.body.prepend(host);
const status=document.createElement('p');status.id='workspace-moria-status';status.hidden=true;
status.setAttribute('role','status');status.textContent='Backdrop unavailable. Review is unaffected.';
document.body.append(status);
const control=document.createElement('label');control.className='moria-control';control.hidden=true;
control.textContent='Backdrop ';
const select=document.createElement('select');select.id='workspace-moria-mode';select.setAttribute('aria-label','Backdrop');
for(const [value,label] of [['live','Live'],['still','Still'],['off','Off']]){
 const option=document.createElement('option');option.value=value;option.textContent=label;select.append(option);
}
control.append(select);document.querySelector('.topbar')?.append(control);
const forcedColors=root.matchMedia('(forced-colors: active)');
const reducedMotion=root.matchMedia('(prefers-reduced-motion: reduce)');
// Visibility remains owned by workspace preferences; motion is a browser-local choice.
const storageKey='csb.moria-motion.v1',choices=['live','still','off'];
let choice='live';
try{const saved=root.localStorage.getItem(storageKey);if(choices.includes(saved))choice=saved;}catch(_){}
let attempted=false,failed=false,ready=false;
const focused=()=>document.body.classList.contains('focus-desk-active')||document.body.classList.contains('focus-lld-active');
const permitted=()=>focused()&&!host.classList.contains('preference-hidden')&&!forcedColors.matches;
async function load(){
 attempted=true;
 try{
  const response=await fetch(assetURL(scriptURL,root.location.href),{credentials:'same-origin',mode:'same-origin',redirect:'error'});
  if(!response.ok)throw Error('Backdrop unavailable');
  const art=validateArtwork(await response.json()),fragment=document.createDocumentFragment();
  art.rows.forEach((row,index)=>{
   for(const [tone,text] of row){const span=document.createElement('span');span.className=`tone-${tone}`;span.textContent=text;fragment.append(span);}
   if(index<art.rows.length-1)fragment.append(document.createTextNode('\n'));
  });
  grid.replaceChildren(fragment);ready=true;host.dataset.ready='true';
 }catch(_){failed=true;host.dataset.ready='false';}
 refresh();
}
function refresh(){
 const allowed=permitted(),active=allowed&&choice!=='off';
 const motion=reducedMotion.matches&&choice==='live'?'still':choice;
 control.hidden=!allowed;select.value=motion;select.children[0].disabled=reducedMotion.matches;
 select.title=reducedMotion.matches?'Live motion is disabled by your reduced-motion preference.':'Backdrop motion · saved in this browser';
 host.dataset.motion=motion;host.dataset.visible=String(active&&ready);
 host.dataset.running=String(active&&ready&&motion==='live'&&!document.hidden);
 status.hidden=!(active&&failed);
 if(active&&!attempted&&!document.hidden)void load();
}
select.addEventListener('change',()=>{
 if(!choices.includes(select.value))return;
 choice=select.value;
 try{root.localStorage.setItem(storageKey,choice);}catch(_){}
 refresh();
});
// Observe only preference/mode changes, never animation state or rendered descendants.
const observer=new MutationObserver(refresh);
observer.observe(document.body,{attributes:true,attributeFilter:['class']});
observer.observe(host,{attributes:true,attributeFilter:['class']});
forcedColors.addEventListener('change',refresh);
reducedMotion.addEventListener('change',refresh);
document.addEventListener('visibilitychange',refresh);
refresh();
})(typeof window==='undefined'?globalThis:window);
