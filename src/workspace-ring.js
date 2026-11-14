/* Decorative Standard-only torus; no review data, storage, network, or inline assets. */
(function(root){
'use strict';
function mount(doc=root.document){
 if(!doc?.body)return null;
 let host=doc.getElementById('workspace-ring');
 if(!host){host=doc.createElement('div');host.id='workspace-ring';doc.body.prepend(host);}
 host.setAttribute('aria-hidden','true');host.setAttribute('inert','');
 const stage=doc.createElement('div');stage.className='workspace-ring-stage';
 const surface=doc.createElement('div');surface.className='workspace-ring-surface';
 const ring=doc.createElement('pre');ring.className='workspace-ring-text';
 const text=doc.createTextNode('');ring.append(text);
 const inscription=doc.createElement('div');inscription.className='workspace-ring-inscription';
 const verse=doc.createElement('div');verse.className='workspace-ring-verse';
 const verseTitle=doc.createElement('strong');verseTitle.textContent='One Ring to rule them all';
 const verseLines=doc.createElement('p');verseLines.append(doc.createTextNode('One Ring to find them • One Ring to bring them all'),doc.createElement('br'),doc.createTextNode('And in the darkness bind them'));
 verse.append(verseTitle,verseLines);
 surface.append(ring,inscription);stage.append(surface,verse);host.replaceChildren(stage);
 // Rounded rectangular torus: broad band, thin radial wall, and open center.
 const W=128,H=50,SIZE=W*H,R=1.64,wall=.20,halfBand=.48,corner=.065;
 const TAU=Math.PI*2,angles=280,cross=[];
 for(let j=0;j<=36;j++){const y=-halfBand+corner+(2*halfBand-2*corner)*j/36;cross.push([wall,y,1,0],[-wall,y,-1,0]);}
 for(let j=0;j<=10;j++){const r=-wall+corner+(2*wall-2*corner)*j/10;cross.push([r,halfBand,0,1],[r,-halfBand,0,-1]);}
 for(let quadrant=0;quadrant<4;quadrant++){const cx=(quadrant===0||quadrant===3?1:-1)*(wall-corner),cy=(quadrant<2?1:-1)*(halfBand-corner);for(let j=0;j<=8;j++){const a=(quadrant+j/8)*Math.PI/2,c=Math.cos(a),s=Math.sin(a);cross.push([cx+corner*c,cy+corner*s,c,s]);}}
 const points=new Float32Array(angles*cross.length*6),letters='ONERINGTORULETHEMALL',luma=' .,:;~!=+*#%@';
 const depth=new Float32Array(SIZE),pixels=new Uint16Array(SIZE),output=new Array(SIZE+H-1);
 let at=0;
 for(let i=0;i<angles;i++){const theta=i*TAU/angles,ct=Math.cos(theta),st=Math.sin(theta);for(const [offset,y,nr,ny] of cross){const rad=R+offset;points[at++]=rad*ct;points[at++]=y;points[at++]=rad*st;points[at++]=ct*nr;points[at++]=ny;points[at++]=st*nr;}}
 const inscriptionText=letters.repeat(2),glyphs=new Float32Array(inscriptionText.length*3);
 for(let i=0;i<inscriptionText.length;i++){const theta=-Math.PI/2+.6-1.3+Math.floor(i/letters.length)*Math.PI+(i%letters.length)*2.6/(letters.length-1);glyphs[i*3]=(R+wall+.018)*Math.cos(theta);glyphs[i*3+2]=(R+wall+.018)*Math.sin(theta);}
 const glyphNodes=[...inscriptionText].map(char=>{const span=doc.createElement('span');span.textContent=char;span.hidden=true;inscription.append(span);return span;});
 let spin=.6,clock=0,previous=0,lastPaint=0,raf=0,paused=false,painted=false,disposed=false;
 const reduced=root.matchMedia('(prefers-reduced-motion: reduce)'),forced=root.matchMedia('(forced-colors: active)');
 const button=doc.getElementById('workspace-ring-pause');
 function paint(){
  depth.fill(0);pixels.fill(32);
  const a=.60+.28*Math.sin(clock*.6),b=-.33+.24*Math.sin(clock*.3),ca=Math.cos(a),sa=Math.sin(a),cb=Math.cos(b),sb=Math.sin(b),cs=Math.cos(spin),ss=Math.sin(spin),scale=137;
  for(let i=0;i<points.length;i+=6){const x=points[i],y=points[i+1],z=points[i+2],nx=points[i+3],ny=points[i+4],nz=points[i+5];const rx=x*cs+z*ss,rz=z*cs-x*ss,ry=y*ca-rz*sa,zz=y*sa+rz*ca,xx=rx*cb-ry*sb,yy=rx*sb+ry*cb;const nrx=nx*cs+nz*ss,nrz=nz*cs-nx*ss,nry=ny*ca-nrz*sa,nzz=ny*sa+nrz*ca,nxx=nrx*cb-nry*sb,nyy=nrx*sb+nry*cb;if(nzz>.22)continue;const inv=1/(zz+6),sx=Math.round(W/2+scale*xx*inv),sy=Math.round(H/2+scale*.48*yy*inv);if(sx<0||sx>=W||sy<0||sy>=H)continue;const p=sx+sy*W;if(inv<=depth[p])continue;depth[p]=inv;const light=Math.max(0,-.38*nxx-.55*nyy-.74*nzz),highlight=Math.pow(Math.max(0,-nzz),12),shade=Math.min(1,.12+.73*light+.15*highlight);pixels[p]=luma.charCodeAt(Math.min(luma.length-1,Math.floor(shade*(luma.length-1))));}
  // Etching follows the same surface transform and depth test, never showing through the back.
  for(let i=0;i<inscriptionText.length;i++){const glyph=glyphNodes[i],x=glyphs[i*3],z=glyphs[i*3+2],rx=x*cs+z*ss,rz=z*cs-x*ss,ry=-rz*sa,zz=rz*ca;glyph.hidden=true;if(rz>=0)continue;const xx=rx*cb-ry*sb,yy=rx*sb+ry*cb,inv=1/(zz+6),px=W/2+scale*xx*inv,py=H/2+scale*.48*yy*inv,sx=Math.round(px),sy=Math.round(py);if(sx<0||sx>=W||sy<0||sy>=H)continue;if(inv+.003<depth[sx+sy*W])continue;glyph.hidden=false;glyph.style.left=`${(px+.5)/W*100}%`;glyph.style.top=`${(py+.5)/H*100}%`;}
  let index=0;for(let y=0;y<H;y++){for(let x=0;x<W;x++)output[index++]=String.fromCharCode(pixels[x+y*W]);if(y<H-1)output[index++]='\n';}text.nodeValue=output.join('');painted=true;
 }
 function visible(){return !disposed&&doc.body.classList.contains('standard-desk-active')&&!host.classList.contains('preference-hidden')&&!host.hidden&&!forced.matches;}
 function active(){return visible()&&!paused&&!doc.hidden&&!reduced.matches;}
 function tick(now){raf=0;if(!active())return;if(!previous)previous=now;if(now-lastPaint>=1000/30){const dt=Math.min((now-previous)/1000,.1);clock+=dt;spin+=dt*.42;paint();previous=now;lastPaint=now;}raf=root.requestAnimationFrame(tick);}
 function sync(){
  if(raf)root.cancelAnimationFrame(raf);raf=0;previous=0;
  const running=active();
  host.classList.toggle('workspace-ring-running',running);
  if(button){const label=reduced.matches?'Static Ring':paused?'Play Ring':'Pause Ring';if(button.textContent!==label)button.textContent=label;button.disabled=reduced.matches||forced.matches;button.setAttribute('aria-pressed',String(paused||reduced.matches));}
  if(visible()&&!doc.hidden&&!painted)paint();
  if(running)raf=root.requestAnimationFrame(tick);
 }
 function togglePause(){paused=!paused;sync();return paused;}
 // Observe only state attributes: painting glyphs must not restart the frame loop.
 const observer=new root.MutationObserver(sync);
 observer.observe(doc.body,{attributes:true,attributeFilter:['class']});
 observer.observe(host,{attributes:true,attributeFilter:['class','hidden']});
 doc.addEventListener('visibilitychange',sync);reduced.addEventListener('change',sync);forced.addEventListener('change',sync);
 button?.addEventListener('click',togglePause);
 sync();
 return {togglePause,destroy(){disposed=true;if(raf)root.cancelAnimationFrame(raf);observer.disconnect();doc.removeEventListener('visibilitychange',sync);reduced.removeEventListener('change',sync);forced.removeEventListener('change',sync);button?.removeEventListener('click',togglePause);host.remove();}};
}
if(typeof module!=='undefined')module.exports={mount};
if(root.document)root.WorkspaceRing=mount();
})(typeof window==='undefined'?globalThis:window);
