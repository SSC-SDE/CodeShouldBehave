'use strict';
// Optional verification dependencies are supplied by the operator. They are not
// installed by this helper or shipped in the public viewer's runtime bundle.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),{createHash}=require('node:crypto');
const REPO_ROOT=fs.realpathSync(process.env.REPO_ROOT||path.resolve(__dirname,'../..'));
const PUBLIC_SITE=fs.realpathSync(process.env.PUBLIC_SITE||path.join(REPO_ROOT,'target/phase6-public-site'));
const PUBLIC_BASE_URL=process.env.PUBLIC_BASE_URL||'http://127.0.0.1:3220/workspace/';
const base=new URL(PUBLIC_BASE_URL);
assert.equal(base.protocol,'http:','This profile is loopback-only; it does not authorize live-site tests');
assert.ok(['localhost','127.0.0.1','[::1]'].includes(base.hostname));
assert.ok(base.port&&!['3102','3195','3196','3197'].includes(base.port),'Do not use an existing user service');
assert.ok(base.pathname.endsWith('/workspace/')&&!base.search&&!base.hash&&!base.username&&!base.password);
const OUTPUT=path.resolve(process.env.PUBLIC_BROWSER_OUTPUT||path.join(REPO_ROOT,'target/public-browser-qa'));
assert.ok(OUTPUT!==PUBLIC_SITE&&!OUTPUT.startsWith(PUBLIC_SITE+path.sep),'Reports must stay outside the served site');
fs.mkdirSync(OUTPUT,{recursive:true});
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
const relative=value=>typeof value==='string'&&value.length>0&&value.length<1024&&!/[\\:%?#\x00-\x1f\x7f]/.test(value)&&!value.startsWith('/')&&value.split('/').every(part=>part&&part!=='.'&&part!=='..'&&part!=='.git');
function readFile(root,name,max=8*1024*1024){assert.ok(relative(name),'Unsafe verification fixture path');const target=path.join(root,name);assert.ok(!fs.lstatSync(target).isSymbolicLink(),'Symlink fixture rejected');const real=fs.realpathSync(target);assert.ok(real.startsWith(fs.realpathSync(root)+path.sep),'Fixture path escaped its root');const stat=fs.statSync(real);assert.ok(stat.isFile()&&stat.size<=max,'Verification input exceeds its bound');return fs.readFileSync(real);}
const readSite=name=>readFile(PUBLIC_SITE,name,64*1024*1024);
const siteSHA=sha(readSite('site-manifest.json'));
if(process.env.PUBLIC_SITE_SHA256)assert.equal(siteSHA,process.env.PUBLIC_SITE_SHA256,'Expected site receipt changed');
const index=JSON.parse(readSite('index.json'));assert.equal(index.schema_version,'csb-public-index-v1');assert.ok(Array.isArray(index.reports));
const matches=process.env.PUBLIC_REPORT_ID?index.reports.filter(item=>item.public_report_id===process.env.PUBLIC_REPORT_ID):index.reports;
assert.equal(matches.length,1,'Select exactly one published bundle with PUBLIC_REPORT_ID');
const record=matches[0];assert.match(record.public_report_id,/^[0-9a-f]{64}$/);
assert.equal(record.repository.owner,'fixture','This acceptance profile accepts only the documented synthetic fixture');
assert.equal(record.repository.name,'public');
const bundlePrefix='reviews/'+record.public_report_id+'/',dir=path.join(PUBLIC_SITE,bundlePrefix);
const manifestBytes=readSite(bundlePrefix+'manifest.json');assert.equal(sha(manifestBytes),record.manifest_sha256);assert.equal(manifestBytes.length,record.manifest_bytes);
const manifest=JSON.parse(manifestBytes);assert.equal(manifest.public_report_id,record.public_report_id);
function reportFile(name){const bytes=readFile(dir,name),descriptor=manifest.files.find(item=>item.path===name);assert.ok(descriptor,'Missing public chunk descriptor');assert.equal(bytes.length,descriptor.bytes);assert.equal(sha(bytes),descriptor.sha256);return bytes;}
const assetSHA=()=>Object.fromEntries(['workspace.js','public-workspace.js','workspace.css'].map(name=>[name,sha(readSite('assets/'+name))]));
const sourceSHA=()=>Object.fromEntries(['src/workspace.js','src/public-workspace.js','src/workspace.css'].map(name=>[name,sha(readFile(REPO_ROOT,name))]));
const helperSHA=()=>Object.fromEntries(['config.cjs','lifecycle.cjs','performance.cjs','session-oracle.cjs'].map(name=>[name,sha(fs.readFileSync(path.join(__dirname,name)))]));
const metadata=()=>({site_sha256:sha(readSite('site-manifest.json')),bundle_manifest_sha256:record.manifest_sha256,asset_sha256:assetSHA(),source_sha256:sourceSHA(),helpers_sha256:helperSHA()});
function finishMetadata(before){const after=metadata();assert.equal(after.site_sha256,before.site_sha256);assert.deepEqual(after.asset_sha256,before.asset_sha256,'Served assets changed during verification');assert.deepEqual(after.source_sha256,before.source_sha256,'Source assets changed during verification');assert.deepEqual(after.helpers_sha256,before.helpers_sha256,'Verification helpers changed during verification');return after;}
function browserRuntime(){const {chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');const executablePath=process.env.CHROMIUM_EXECUTABLE;return {chromium,launch:{headless:true,chromiumSandbox:true,...(executablePath?{executablePath}:{})}};}
module.exports={fs,path,assert,sha,REPO_ROOT,PUBLIC_SITE,PUBLIC_BASE_URL,OUTPUT,index,record,manifest,dir,readFile,readSite,reportFile,metadata,finishMetadata,browserRuntime};
