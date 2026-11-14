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
const scriptURL=document.currentScript.src;
const host=document.createElement('div');
host.id='workspace-moria';host.className='workspace-moria preference-hidden';
host.setAttribute('aria-hidden','true');host.setAttribute('inert','');
const grid=document.createElement('pre');host.append(grid);document.body.prepend(host);
const status=document.createElement('p');status.id='workspace-moria-status';status.hidden=true;
status.setAttribute('role','status');status.textContent='Backdrop unavailable. Review is unaffected.';
document.body.append(status);
const forcedColors=root.matchMedia('(forced-colors: active)');
let attempted=false,failed=false;
const enabled=()=>document.body.classList.contains('focus-desk-active')&&!host.classList.contains('preference-hidden')&&!forcedColors.matches;
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
  grid.replaceChildren(fragment);host.dataset.ready='true';
 }catch(_){failed=true;host.dataset.ready='false';}
 refresh();
}
function refresh(){
 const active=enabled();status.hidden=!(active&&failed);
 if(active&&!attempted)void load();
}
const observer=new MutationObserver(refresh);
observer.observe(document.body,{attributes:true,attributeFilter:['class']});
observer.observe(host,{attributes:true,attributeFilter:['class']});
forcedColors.addEventListener('change',refresh);
refresh();
})(typeof window==='undefined'?globalThis:window);
