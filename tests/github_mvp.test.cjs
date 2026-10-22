const test = require('node:test');
const assert = require('node:assert/strict');
const {mkdtempSync, mkdirSync, writeFileSync, readFileSync, chmodSync, existsSync, rmSync} = require('node:fs');
const {join} = require('node:path');
const {tmpdir} = require('node:os');
const {execFileSync, spawnSync} = require('node:child_process');

function executable(path, source) { writeFileSync(path, source); chmodSync(path, 0o755); }

test('GitHub MVP uses gh auth, exact refs and loopback-safe report generation', () => {
  const root=mkdtempSync(join(tmpdir(),'csb-github-mvp-'));
  const bin=join(root,'bin'),repo=join(root,'repo'),output=join(root,'output'),log=join(root,'calls.log');
  mkdirSync(bin);mkdirSync(repo);mkdirSync(output);
  executable(join(bin,'gh'),`#!/bin/sh
printf 'gh %s\\n' "$*" >> "$CSB_TEST_LOG"
case "$*" in
  'auth status --hostname github.com') exit 0 ;;
  *'repo view --json nameWithOwner --jq .nameWithOwner') printf 'SSC-SDE/codeshouldbehave\\n' ;;
  *'--jq .baseRefName') printf 'main\\n' ;;
  *'--jq .baseRefOid') printf '%040d\\n' 0 | tr '0' a ;;
  *'--jq .headRefOid') printf '%040d\\n' 0 | tr '0' b ;;
  *'--jq .url') printf 'https://github.com/SSC-SDE/codeshouldbehave/pull/31\\n' ;;
  *) exit 9 ;;
esac
`);
  executable(join(bin,'git'),`#!/bin/sh
printf 'git %s\\n' "$*" >> "$CSB_TEST_LOG"
case "$*" in
  *'rev-parse --is-inside-work-tree') printf 'true\\n' ;;
  'check-ref-format --branch main') exit 0 ;;
  *'rev-parse --verify refs/codeshouldbehave/'*'-base-'*'^{commit}') printf '%040d\\n' 0 | tr '0' a ;;
  *'rev-parse --verify refs/codeshouldbehave/'*'-head-'*'^{commit}') printf '%040d\\n' 0 | tr '0' b ;;
  *' fetch '*) exit 0 ;;
  *' update-ref -d '*) exit 0 ;;
  *) exit 9 ;;
esac
`);
  const flowdiff=join(bin,'flowdiff');
  executable(flowdiff,`#!/bin/sh
printf 'flowdiff %s\\n' "$*" >> "$CSB_TEST_LOG"
while [ "$#" -gt 0 ]; do
  if [ "$1" = --output ]; then shift; printf '<!doctype html><title>review</title>' > "$1"; fi
  if [ "$1" = --artifact ]; then shift; printf '{"artifact_version":"csb-review-artifact-v1"}' > "$1"; fi
  shift
done
exit 0
`);
  const script=join(__dirname,'..','scripts','github_mvp.sh');
  const stdout=execFileSync(script,[repo,'31',output,'--no-serve'],{encoding:'utf8',env:{...process.env,PATH:`${bin}:${process.env.PATH}`,CSB_FLOWDIFF_BIN:flowdiff,CSB_TEST_LOG:log}});
  const calls=readFileSync(log,'utf8');
  assert.match(calls,/gh auth status --hostname github\.com/);
  assert.match(calls,/git .*fetch --no-tags origin a{40}:refs\/codeshouldbehave\/pr-31-base-/);
  assert.match(calls,/git .*fetch --no-tags origin pull\/31\/head:refs\/codeshouldbehave\/pr-31-head-/);
  assert.match(calls,/flowdiff review .*--base a{40} --head b{40} --merge-base/);
  assert.match(calls,/git .*update-ref -d refs\/codeshouldbehave\/pr-31-base-/);
  assert.match(calls,/git .*update-ref -d refs\/codeshouldbehave\/pr-31-head-/);
  assert.ok(existsSync(join(output,'pr-31.html')));
  assert.ok(existsSync(join(output,'pr-31.json')));
  assert.match(calls,/--github-repository SSC-SDE\/codeshouldbehave/);
  assert.match(calls,/--artifact/);
  assert.match(stdout,/https:\/\/github\.com\/SSC-SDE\/codeshouldbehave\/pull\/31/);
  assert.doesNotMatch(stdout,/token|authorization|bearer/i);
  rmSync(root,{recursive:true,force:true});
});

test('GitHub MVP fails before network work when authentication is unavailable', () => {
  const root=mkdtempSync(join(tmpdir(),'csb-github-auth-')),bin=join(root,'bin'),repo=join(root,'repo'),output=join(root,'output');
  mkdirSync(bin);mkdirSync(repo);mkdirSync(output);
  executable(join(bin,'gh'),'#!/bin/sh\nexit 1\n');
  executable(join(bin,'git'),`#!/bin/sh
case "$*" in *'rev-parse --is-inside-work-tree') printf 'true\\n';; *) exit 9;; esac
`);
  const result=spawnSync(join(__dirname,'..','scripts','github_mvp.sh'),[repo,'31',output,'--no-serve'],{encoding:'utf8',env:{...process.env,PATH:`${bin}:${process.env.PATH}`}});
  assert.notEqual(result.status,0);
  assert.equal(existsSync(join(output,'pr-31.html')),false);
  rmSync(root,{recursive:true,force:true});
});

test('browser onboarding keeps credentials server-side and mutations session-bound', () => {
  const html=readFileSync(join(__dirname,'..','src','github_mvp.html'),'utf8');
  const js=readFileSync(join(__dirname,'..','src','github_mvp.js'),'utf8');
  const server=readFileSync(join(__dirname,'..','scripts','github_mvp_server.py'),'utf8');
  assert.match(html,/meta name="csb-session" content="__CSB_SESSION__"/);
  assert.match(html,/script src="\/assets\/github-mvp\.js"/);
  assert.doesNotMatch(html,/<script(?! src=)/);
  assert.match(js,/'X-CSB-Session':session/);
  assert.match(js,/window\.location\.assign\(value\.review_url\)/);
  assert.match(js,/activeButton\.textContent='Retry review'/);
  assert.doesNotMatch(js,/token|bearer|localStorage/i);
  const withoutThemeStorage=js.replace(/sessionStorage\.(?:getItem|removeItem)\('csb\.application-theme'\)/g,'');
  assert.doesNotMatch(withoutThemeStorage,/sessionStorage/i);
  assert.match(js,/\['graphite','blueprint','terminal','contrast'\]\.includes\(theme\)/);
  assert.match(server,/ThreadingHTTPServer\(\("127\.0\.0\.1", args\.port\)/);
  assert.match(server,/compare_digest\(self\.headers\.get\("X-CSB-Session", ""\), state\.session\)/);
  assert.match(server,/re\.fullmatch\(r"pr-\[1-9\]\[0-9\]\*\\\.html", name\)/);
  assert.match(server,/Content-Security-Policy/);
  assert.match(server,/TemporaryDirectory\(prefix=f"\.pending-pr-\{pr\}-", dir=self\.output\)/);
  assert.match(server,/self\.publish_report\(pr,typed,candidate,validated,authentication_generation\)/);
  assert.match(js,/value\.review_generation===lastGeneration&&value\.review_pr===lastPr/);
  assert.match(server,/WORKSPACE = \(ROOT \/ "src" \/ "workspace\.html"\)\.read_bytes\(\)/);
  assert.match(server,/"csb-local-session"/);
});
