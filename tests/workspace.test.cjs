const {test} = require('node:test');
const assert = require('node:assert/strict');
const {readFileSync, mkdtempSync, writeFileSync, rmSync} = require('node:fs');
const {join} = require('node:path');
const {tmpdir} = require('node:os');
const {execFileSync, spawnSync} = require('node:child_process');
const {ReviewWorkspace, graphItems, filterGraphItems, clusterEdges, mergeGraphs, diffLines, architectureMeaning, architectureAreas, architectureMermaid, observationAge, filterChangedFiles, diagramScale, parseScopePaths, summarizeReviewState, classifyFinding, groupFindings, encodeWorkspaceRoute, decodeWorkspaceRoute, betaFeedbackRecord, importBetaFeedback, BetaFeedbackDraft} = require('../src/workspace.js');

test('file search includes rename origins without altering the loaded evidence', () => {
  const files = [{path:'src/new.rs',old_path:'src/old.rs'}, {path:'docs/usage.md'}];
  assert.deepEqual(filterChangedFiles(files, ' OLD.RS '), [files[0]]);
  assert.deepEqual(filterChangedFiles(files, ''), files);
  assert.deepEqual(filterChangedFiles(files, 'missing'), []);
  assert.equal(files.length, 2);
});

test('unknown update time stays unknown and diagram fit preserves aspect scale', () => {
  assert.equal(observationAge(0), 'Update time unavailable');
  assert.equal(observationAge(NaN), 'Update time unavailable');
  assert.equal(observationAge(1000, 3601000), 'Updated 1h ago');
  assert.equal(diagramScale(1000, 532), .5);
  assert.equal(diagramScale(400, 800), 1);
  assert.equal(diagramScale(1000, 532, 1), 1);
  assert.equal(diagramScale(1000, 532, 3), 2);
  assert.equal(diagramScale(0, 0), 1);
});

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return {promise, resolve, reject};
}

test('late responses cannot replace a newer selected review', async () => {
  const requests = [];
  const request = (path, {signal}) => {
    const work = deferred(); requests.push({path, signal, work}); return work.promise;
  };
  const states = [];
  const workspace = new ReviewWorkspace({request, onState: state => states.push(state)});
  const first = workspace.selectReview('review-old');
  assert.equal(requests.length, 7);
  const second = workspace.selectReview('review-new');
  assert.equal(requests.length, 14);
  assert.ok(requests.slice(0, 7).every(item => item.signal.aborted));
  requests.slice(7).forEach(item => item.work.resolve(
    item.path.endsWith('/progress') ? {review_id:'review-new',stale:false}
      : item.path.includes('/graph?') ? {kind:'children',paths:[]}
        : item.path.endsWith('/annotations') ? {review_id:'review-new',items:[]}
          : item.path.includes('/files?') || item.path.includes('/findings?') ? {review_id:'review-new',items:[],next_cursor:null}
            : {review_id:'review-new'}
  ));
  await second;
  requests.slice(0, 7).forEach(item => item.work.resolve(item.path.includes('/graph?')?{kind:'children',paths:[]}:{review_id:'review-old'}));
  await first;
  const state = states.at(-1);
  assert.equal(state.reviewId, 'review-new');
  assert.equal(state.details.review_id, 'review-new');
});

test('changed-file pagination stays bound to the active review', async () => {
  const paths = [];
  const workspace = new ReviewWorkspace({request: async path => {
    paths.push(path);
    if (path.includes('/files?') && path.includes('cursor=')) return {review_id: 'review-1', items: [{path: 'b.rs', kind: 'added'}], next_cursor: null};
    if (path.includes('/files?')) return {review_id: 'review-1', items: [{path: 'a.rs', kind: 'modified'}], next_cursor: 'next-page'};
    if (path.endsWith('/progress')) return {review_id: 'review-1'};
    if (path.includes('/graph?')) return {kind: 'children', paths: []};
    return {review_id: 'review-1'};
  }});
  await workspace.selectReview('review-1');
  await workspace.loadMoreFiles();
  assert.deepEqual(workspace.snapshot().files.items.map(file => file.path), ['a.rs', 'b.rs']);
  assert.ok(paths.some(path => path.includes('/files?limit=100&cursor=next-page')));
});

test('source is fetched lazily and must match the active review', async () => {
  const paths = [];
  const workspace = new ReviewWorkspace({
    request: async path => {
      paths.push(path);
      if (path.includes('/source?')) return {review_id: 'review-1', commit: 'a'.repeat(40), text: 'line'};
      if (path.endsWith('/progress')) return {review_id: 'review-1'};
      if (path.includes('/graph?')) return {kind: 'children', paths: []};
      return {review_id: 'review-1'};
    },
  });
  await workspace.selectReview('review-1');
  assert.ok(!paths.some(path => path.includes('/source?')));
  await workspace.loadSource({path: 'src/lib.rs', blob: 'blob-1', side: 'proposal', startLine: 3, endLine: 8});
  assert.ok(paths.some(path => path.includes('/source?') && path.includes('blob=blob-1')));
  assert.equal(workspace.snapshot().source.text, 'line');
});

test('source comparison loads exact baseline and proposal spans together', async () => {
  const workspace = new ReviewWorkspace({request: async path => {
    if (path.includes('/source?')) {
      const query=new URLSearchParams(path.split('?')[1]);const side=query.get('side');
      return {review_id:'review-1',commit:(side==='baseline'?'a':'b').repeat(40),path:'src/lib.rs',start_line:3,end_line:4,text:side==='baseline'?'old\nsame':'new\nsame'};
    }
    if (path.endsWith('/progress')) return {review_id:'review-1'};
    if (path.includes('/graph?')) return {kind:'children',paths:[]};
    return {review_id:'review-1',items:[],next_cursor:null};
  }});
  await workspace.selectReview('review-1');
  await workspace.loadSourceComparison({path:'src/lib.rs',baseline:{blob_id:'old',start_line:3,end_line:4},proposal:{blob_id:'new',start_line:3,end_line:4}});
  assert.equal(workspace.snapshot().sourceComparison.baseline.text,'old\nsame');
  assert.equal(workspace.snapshot().sourceComparison.proposal.text,'new\nsame');
});

test('component expansion and symbol selection request bounded neighborhoods', async () => {
  const paths = [];
  const workspace = new ReviewWorkspace({request: async path => {
    paths.push(path);
    if (path.endsWith('/progress')) return {};
    if (path.includes('/graph?')) return {kind: 'children', paths: []};
    return {review_id: 'review-1'};
  }});
  await workspace.selectReview('review-1');
  await workspace.expandComponent({path: 'src/api'});
  await workspace.selectSymbol({symbol: 'src/api.rs::handle', path: 'src/api.rs', direction: 'incoming'});
  assert.ok(paths.some(path => path.includes('parent=src%2Fapi') && path.includes('limit=100')));
  assert.ok(paths.some(path => path.includes('direction=incoming') && path.includes('symbol=src%2Fapi.rs%3A%3Ahandle')));
});

test('file selection requests symbols without fetching source', async () => {
  const paths = [];
  const workspace = new ReviewWorkspace({request: async path => {
    paths.push(path);
    if (path.endsWith('/progress')) return {};
    if (path.includes('/graph?')) return {kind: 'symbols', symbols: []};
    return {review_id: 'review-1'};
  }});
  await workspace.selectReview('review-1');
  await workspace.selectPath({path: 'src/lib.rs', side: 'baseline'});
  assert.ok(paths.some(path => path.includes('side=baseline') && path.includes('path=src%2Flib.rs')));
  assert.ok(!paths.some(path => path.includes('/source?')));
});

test('dense graph edges cluster deterministically while list kinds stay bounded', () => {
  const edges = Array.from({length: 75}, (_, index) => ({
    kind: index < 50 ? 'call' : 'import',
    source_path: index < 50 ? 'src/api.rs' : 'src/model.rs',
    stable_key: `edge-${index}`,
  }));
  const groups = clusterEdges(edges);
  assert.deepEqual(groups.map(([key, items]) => [key, items.length]), [
    ['call · src/api.rs', 50],
    ['import · src/model.rs', 25],
  ]);
  assert.equal(graphItems({kind: 'neighborhood', edges}).length, 75);
  assert.deepEqual(graphItems({kind: 'children', paths: [['src', 'component']]}), [['src', 'component']]);
});

test('graph search and relationship filters keep loaded evidence bounded', () => {
  const graph = {kind:'neighborhood',edges:[
    {kind:'call',from_symbol:'pay',to_symbol:'gateway',source_path:'src/pay.rs'},
    {kind:'import',from_symbol:'events',to_symbol:'queue',source_path:'src/events.rs'},
  ]};
  assert.equal(filterGraphItems(graph, 'gateway').length, 1);
  assert.equal(filterGraphItems(graph, '', 'import')[0].source_path, 'src/events.rs');
  assert.deepEqual(filterGraphItems(graph, 'missing', ''), []);
});

test('architecture diff marks added, removed and unchanged paths', () => {
  const diff = mergeGraphs(
    {kind: 'children', paths: [['src/old.rs', 'file'], ['src/same.rs', 'file']]},
    {kind: 'children', paths: [['src/same.rs', 'file'], ['src/new.rs', 'file']]},
  );
  assert.deepEqual(diff.paths.map(item => [item.path, item.status]), [
    ['src/old.rs', 'removed'], ['src/same.rs', 'unchanged'], ['src/new.rs', 'added'],
  ]);
});

test('code comparison aligns modified rows for split and unified views', () => {
  assert.deepEqual(diffLines('same\nold\ntail', 'same\nnew\ntail'), [
    {kind:'same',before:'same',after:'same'}, {kind:'modified',before:'old',after:'new'}, {kind:'same',before:'tail',after:'tail'},
  ]);
  assert.deepEqual(diffLines('', 'added').at(-1), {kind:'added',before:null,after:'added'});
});

test('architecture Mermaid is bounded, escaped and distinguishes impact states', () => {
  const files=[
    {path:'src/payments.rs',kind:'modified'}, {path:'src/gateway/client.rs',kind:'added'},
    {path:'config/old.toml',kind:'deleted'},
  ];
  assert.deepEqual(architectureAreas(files).map(area=>area.name), ['External service gateway','Payment processing','Runtime routing']);
  const source=architectureMermaid({files,findings:[{kind:'caller_<unsafe>',resolution:'ambiguous',source_path:'src/payments.rs'}],details:{selected_baseline:'a'.repeat(40),proposal:'b'.repeat(40)}},'combined');
  assert.match(source,/Baseline aaaaaaaa/);assert.match(source,/Proposal bbbbbbbb/);assert.match(source,/Payment processing/);assert.match(source,/Owns payment validation/);assert.match(source,/Review: caller unsafe/);assert.match(source,/exists before/);assert.match(source,/changed here/);assert.match(source,/classDef added/);assert.match(source,/classDef removed/);assert.ok(!source.includes('<unsafe>'));
});

test('architecture meaning converts paths into reviewer-facing component roles',()=>{
  assert.equal(architectureMeaning('src/gateway/client.rs').title,'External service gateway');
  assert.equal(architectureMeaning('config/routes.toml').title,'Runtime routing');
  assert.equal(architectureMeaning('src/payments.rs').title,'Payment processing');
  assert.equal(architectureMeaning('src/unknown.rs').title,'Application core');
  assert.equal(architectureMeaning('docs/PR_ARCHITECTURE_INTEGRATION.md').title,'Documentation');
  assert.equal(architectureMeaning('docs/MVP_REVIEWER_JOURNEY.md').title,'Documentation');
});

test('Mermaid labels the analyzed merge base rather than the selected target tip',()=>{
  for (const mode of ['combined','baseline','proposal']) {
    const source=architectureMermaid({files:[{path:'src/lib.rs',kind:'modified'}],details:{selected_baseline:'a'.repeat(40),analysis_baseline:'c'.repeat(40),proposal:'b'.repeat(40)}},mode);
    if(mode!=='proposal')assert.match(source,/Baseline cccccccc/);
    assert.doesNotMatch(source,/aaaaaaaa/);
  }
});

test('snapshot diagrams retain mixed areas and stable navigation identities',()=>{
  const input={files:[{path:'src/gateway/client.rs',kind:'added'},{path:'src/payments.rs',kind:'modified'},{path:'src/billing/new.rs',kind:'added'},{path:'src/invoice/old.rs',kind:'deleted'}]};
  const before=architectureMermaid(input,'baseline'),after=architectureMermaid(input,'proposal');
  assert.match(before,/area1\[/);assert.match(before,/payments.rs/);assert.match(before,/old.rs/);assert.doesNotMatch(before,/new.rs|client.rs/);
  assert.match(after,/area0\[/);assert.match(after,/area1\[/);assert.match(after,/new.rs/);assert.doesNotMatch(after,/old.rs/);
});

test('scope paths are explicit literals and reject traversal or globs', () => {
  assert.deepEqual(parseScopePaths('src/api, contracts/openapi.yaml', true), ['src/api','contracts/openapi.yaml']);
  assert.throws(() => parseScopePaths('../private', true));
  assert.throws(() => parseScopePaths('src/**', true));
  assert.throws(() => parseScopePaths('', true));
});

test('review status keeps analysis, provider and human decisions separate', () => {
  const summary = summarizeReviewState({
    details: {stale: false},
    progress: {job_state: 'completed', stale: false, coverage: [['indexed', 42], ['partial', 2]]},
    graph: {kind: 'children', paths: [['src/payments.rs', 'file']], omitted: 3},
    pr: {state: 'draft'},
  });
  assert.equal(summary.analysis, 'Ready');
  assert.equal(summary.provider, 'PR draft');
  assert.equal(summary.providerDetail, 'Provider checks unavailable');
  assert.equal(summary.human, 'Not started');
  assert.match(summary.observed, /1 authorized affected path/);
  assert.match(summary.limitation, /3 graph items omitted/);
  assert.match(summary.limitation, /Coverage is partial/);
});

test('finding labels separate resolved structure from possible effects', () => {
  assert.deepEqual(classifyFinding({resolution:'resolved'}), {
    category:'Observed structural evidence', tone:'observed',
    limitation:'Resolved in the static graph. This is not proof of runtime behavior.', action:'Inspect evidence',
  });
  const ambiguous = classifyFinding({resolution:'ambiguous'});
  assert.equal(ambiguous.category, 'Possible effect / unresolved');
  assert.match(ambiguous.limitation, /Reviewer verification is required/);
});

test('findings group by reviewer question and retain deduplicated provenance', () => {
  const groups = groupFindings([
    {id:1,kind:'call',resolution:'resolved',source_path:'src/a.rs',source_start_line:2,source_end_line:2},
    {id:2,kind:'call',resolution:'resolved',source_path:'src/a.rs',source_start_line:2,source_end_line:2},
    {id:3,kind:'call',resolution:'ambiguous',source_path:'src/b.rs'},
  ]);
  assert.deepEqual(groups.map(group => [group.title, group.items.length]), [
    ['What changed structurally?', 1], ['What still needs reviewer verification?', 1],
  ]);
  assert.deepEqual(groups[0].items[0].provenance, [1, 2]);
});

test('failed and stale analysis never appear as no architectural change', () => {
  const failed = summarizeReviewState({error: 'analysis timed out', pr: {state: 'open'}});
  assert.equal(failed.analysis, 'Failed');
  assert.equal(failed.analysisDetail, 'analysis timed out');
  assert.match(failed.observed, /Loading authorized structural evidence/);

  const stale = summarizeReviewState({details: {stale: true}, progress: {job_state: 'completed'}});
  assert.equal(stale.analysis, 'Stale');
  assert.match(stale.analysisDetail, /newer review identity/);
});

test('release corpus covers uncertainty and an honest source fallback', () => {
  const corpus = JSON.parse(readFileSync(join(__dirname, '../fixtures/reviewer/task-corpus.json'), 'utf8'));
  assert.equal(corpus.schema, 'codeshouldbehave-phase2-usability-corpus-v1');
  assert.deepEqual(new Set(corpus.cases.map(item => item.state)), new Set(['partial', 'stale', 'incomplete', 'unsupported']));
  assert.ok(corpus.cases.every(item => item.expected.unresolved.length > 0));
  assert.ok(corpus.cases.every(item => /cannot be determined/.test(item.expected.design)));
  assert.match(corpus.cases.find(item => item.id === 'source-only-fallback').expected.design, /source evidence fallback/);
});

test('pilot observation export excludes workspace credentials and source', () => {
  const pilot = readFileSync(join(__dirname, '../examples/phase2_pilot.html'), 'utf8');
  assert.match(pilot, /codeshouldbehave-phase2-reviewer-observation-v1/);
  assert.match(pilot, /contains_credentials_or_source:false/);
  assert.match(pilot, /statistical_improvement:false,merge_approval:false/);
  assert.doesNotMatch(pilot, /contentDocument|contentWindow|session-token/);
  assert.match(pilot, /every\(item=>item\.checked\)/);
});

test('pilot validator accepts a complete different-role record and rejects owner role', () => {
  const directory = mkdtempSync(join(tmpdir(), 'csb-phase2-'));
  const path = join(directory, 'observation.json');
  const record = {
    schema:'codeshouldbehave-phase2-reviewer-observation-v1', recorded_at_utc:'2026-10-04T12:00:00Z',
    role:'service owner', experience:'first time using this workspace', authorized_example:'fixture/PR-1842',
    tasks:{triage:true,identity:true,evidence:true,drilldown:true,revision:true,fallback:true,summary:true},
    elapsed_seconds:180, first_relevant_evidence_seconds:22, backtracks:1,
    wrong_interpretation:'None', qualitative_feedback:'The uncertainty labels were clear.', decision:'pass',
    claims:{statistical_improvement:false,merge_approval:false}, contains_credentials_or_source:false,
  };
  writeFileSync(path, JSON.stringify(record));
  try {
    const output = execFileSync('python3', [join(__dirname, '../scripts/validate_phase2_observation.py'), path], {encoding:'utf8'});
    const summary = JSON.parse(output); assert.equal(summary.status, 'passed'); assert.equal(summary.role, 'service owner');
    record.role = 'repository maintainer'; writeFileSync(path, JSON.stringify(record));
    const rejected = spawnSync('python3', [join(__dirname, '../scripts/validate_phase2_observation.py'), path], {encoding:'utf8'});
    assert.notEqual(rejected.status, 0); assert.match(rejected.stderr, /must differ/);
  } finally { rmSync(directory, {recursive:true, force:true}); }
});

test('partial completion remains analysis evidence and never merge approval', () => {
  const summary = summarizeReviewState({
    details: {stale: false},
    progress: {job_state: 'completed', coverage: [['indexed', 40], ['partial', 2]]},
    graph: {kind: 'children', paths: [['src/payments.rs', 'file']]},
    pr: {state: 'open'},
  });
  assert.equal(summary.analysis, 'Ready');
  assert.equal(summary.provider, 'PR open');
  assert.equal(summary.human, 'Not started');
  assert.match(summary.limitation, /Coverage is partial/);
});

test('workspace routes preserve review identity and typed selection without secrets', () => {
  const hash = encodeWorkspaceRoute({
    reviewId: 'review-29', layout: 'source', side: 'diff',
    selection: {kind: 'symbol', path: 'src/service.rs', symbol: 'service::changed_files'},
  });
  assert.deepEqual(decodeWorkspaceRoute(hash), {
    reviewId: 'review-29', layout: 'source', side: 'diff',
    selection: {kind: 'symbol', path: 'src/service.rs', symbol: 'service::changed_files'},
  });
  assert.ok(!hash.includes('token'));
  assert.equal(decodeWorkspaceRoute('#review=r&layout=unknown&side=proposal'), null);
  assert.equal(decodeWorkspaceRoute('#review=r&layout=source&side=proposal&kind=symbol&path=a'), null);
  assert.throws(() => encodeWorkspaceRoute({reviewId: 'r', selection: {kind: 'path', path: ''}}));
});

test('revoked access clears previously loaded review evidence', async () => {
  let revoked = false;
  const workspace = new ReviewWorkspace({request: async path => {
    if (revoked) { const error = new Error('Request failed (404)'); error.status = 404; throw error; }
    if (path.endsWith('/progress')) return {review_id:'review-1'};
    if (path.includes('/files?')) return {review_id:'review-1',items:[{path:'secret.rs',kind:'modified'}],next_cursor:null};
    if (path.includes('/findings?')) return {review_id:'review-1',items:[{id:1,source_path:'secret.rs'}],next_cursor:null};
    if (path.endsWith('/annotations')) return {review_id:'review-1',items:[{id:'private'}]};
    if (path.includes('/graph?')) return {kind:'children',paths:[['secret.rs','file']]};
    if (path.includes('/source?')) return {review_id:'review-1',text:'private source'};
    return {review_id:'review-1'};
  }});
  await workspace.selectReview('review-1');
  await workspace.loadSource({path:'secret.rs',blob:'blob',side:'proposal',startLine:1,endLine:1});
  assert.equal(workspace.snapshot().source.text, 'private source');
  revoked = true;
  await workspace.selectPath({path:'secret.rs'});
  const state = workspace.snapshot();
  assert.equal(state.details, null); assert.equal(state.files, null);
  assert.equal(state.findings, null);
  assert.equal(state.annotations, null);
  assert.equal(state.graph, null); assert.equal(state.source, null); assert.equal(state.selection, null);
  assert.match(state.error, /404/);
});

test('missing routed path stays empty instead of reusing prior source', async () => {
  const workspace = new ReviewWorkspace({request: async path => {
    if (path.endsWith('/progress')) return {review_id:'review-1'};
    if (path.includes('/files?')) return {review_id:'review-1',items:[],next_cursor:null};
    if (path.includes('/graph?') && path.includes('path=')) return {kind:'symbols',symbols:[]};
    if (path.includes('/graph?')) return {kind:'children',paths:[]};
    return {review_id:'review-1'};
  }});
  await workspace.selectReview('review-1');
  await workspace.selectPath({path:'renamed-or-missing.rs'});
  const state = workspace.snapshot();
  assert.equal(state.selection.path, 'renamed-or-missing.rs');
  assert.deepEqual(state.graph.symbols, []);
  assert.equal(state.source, null);
});

test('annotation writes preserve optimistic version and active review identity', async () => {
  const writes = [];
  const workspace = new ReviewWorkspace({request: async (path, options = {}) => {
    if (options.method === 'PUT') { const body = JSON.parse(options.body); writes.push(body); return {...body,review_id:'review-1',proposal_commit:'b'.repeat(40),version:body.expected_version+1}; }
    if (path.endsWith('/annotations')) return {review_id:'review-1',items:[]};
    if (path.endsWith('/progress')) return {review_id:'review-1'};
    if (path.includes('/files?') || path.includes('/findings?')) return {review_id:'review-1',items:[],next_cursor:null};
    if (path.includes('/graph?')) return {kind:'children',paths:[]};
    return {review_id:'review-1',proposal:'b'.repeat(40)};
  }});
  await workspace.selectReview('review-1');
  const written = await workspace.writeAnnotation({id:'a1',visibility:'private',kind:'question',target_path:'src/lib.rs',target_key:'src/lib.rs:2',body:'Why?',status:'active',expected_version:0});
  assert.equal(written.version, 1); assert.equal(workspace.snapshot().annotations.items[0].review_id, 'review-1');
  assert.equal(writes[0].expected_version, 0);
});

test('client timing history and paginated file retention stay bounded', async () => {
  const workspace=new ReviewWorkspace({request:async path=>{
    if(path.includes('/files?')&&path.includes('cursor=')) return {review_id:'review-1',items:Array.from({length:100},(_,i)=>({path:`next/${i}`,kind:'modified'})),next_cursor:'again'};
    if(path.includes('/files?')) return {review_id:'review-1',items:Array.from({length:500},(_,i)=>({path:`src/${i}`,kind:'modified'})),next_cursor:'next'};
    if(path.endsWith('/progress')) return {review_id:'review-1'};
    if(path.includes('/findings?')||path.endsWith('/annotations')) return {review_id:'review-1',items:[]};
    if(path.includes('/graph?')) return {kind:'children',paths:[]}; return {review_id:'review-1'};
  }});
  await workspace.selectReview('review-1'); await workspace.loadMoreFiles();
  const state=workspace.snapshot(); assert.equal(state.files.items.length,500); assert.equal(state.files.client_omitted,100);
  assert.ok(state.timings.length<=50); assert.ok(state.timings.every(item=>item.duration_ms>=0));
});

test('trusted workspace stylesheet keeps Mermaid readable under strict CSP', () => {
  const css = readFileSync(join(__dirname, '..', 'src', 'workspace.css'), 'utf8');
  assert.match(css, /\.architecture-svg \.node\.added/);
  assert.match(css, /\.architecture-svg \.node text/);
  assert.match(css, /\.architecture-svg \.flowchart-link/);
  assert.match(css, /fill:none!important/);
  assert.match(css, /\.architecture-svg marker path/);
});

test('beta feedback export is bounded, previewable and excludes review data', () => {
  const record = betaFeedbackRecord({category:'confusing',description:'The diagram label is unclear.',route:'/workspace#review=secret&path=private.rs',layout:'architecture',side:'diff',hasReview:true,feedbackId:'feedback-1',createdAt:'2026-10-05T00:00:00Z'});
  assert.equal(record.schema, 'codeshouldbehave-beta-feedback-v2');
  assert.equal(record.ui.route, '/workspace');
  assert.equal(record.ui.review_selected, true);
  assert.equal(record.transport, 'local_download');
  assert.deepEqual(record.privacy, {automatic_metadata_includes_credentials:false,automatic_metadata_includes_session_token:false,automatic_metadata_includes_source:false,automatic_metadata_includes_repository_contents:false,screenshot_included:false,user_content_requires_review:true});
  assert.ok(!JSON.stringify(record).includes('private.rs'));
  assert.throws(() => betaFeedbackRecord({category:'invalid'}));
  assert.throws(() => betaFeedbackRecord({category:'other',description:'x'.repeat(4001)}));
});

test('feedback attachments require separate consent, refreshed preview and cancellation clears them', () => {
  const draft=new BetaFeedbackDraft(),fields={category:'other',feedbackId:'feedback-1',createdAt:'2026-10-06T00:00:00Z'};
  const screenshot={mime_type:'image/png',width:1,height:1,bytes:68,data_url:'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aO00AAAAASUVORK5CYII='};
  draft.setScreenshot(screenshot);
  assert.throws(()=>draft.preview(fields),/confirm/);
  draft.setConsent(true);draft.preview(fields);
  assert.equal(draft.exportRecord().privacy.screenshot_included,true);
  draft.change();assert.throws(()=>draft.exportRecord(),/preview/);
  draft.processing=true;assert.throws(()=>draft.preview(fields),/finish loading/);
  draft.processing=false;draft.setScreenshot(screenshot);assert.throws(()=>draft.preview(fields),/confirm/);
  const generation=draft.imageGeneration;draft.reset();assert.ok(draft.imageGeneration>generation);
  assert.equal(draft.screenshot,null);assert.equal(draft.consent,false);assert.throws(()=>draft.exportRecord());
  assert.throws(()=>betaFeedbackRecord({...fields,screenshot:{...screenshot,mime_type:'image/svg+xml'}}));
  assert.throws(()=>betaFeedbackRecord({...fields,screenshot:{...screenshot,width:12000001}}));
});

test('feedback imports whitelist metadata and preserve validated triage without executing content', () => {
  const record=betaFeedbackRecord({category:'confusing',description:'<script>alert(1)</script>',feedbackId:'feedback-1',createdAt:'2026-10-06T00:00:00Z'});
  const imported=importBetaFeedback(JSON.stringify({...record,token:'SECRET',source:'PRIVATE',ui:{...record.ui,repository:'PRIVATE'}}));
  assert.ok(!JSON.stringify(imported).includes('SECRET'));assert.ok(!JSON.stringify(imported).includes('PRIVATE'));
  assert.equal(imported.description,record.description);
  const resolved=betaFeedbackRecord({category:'other',status:'resolved',severity:'high',resolutionCommit:'a'.repeat(40)});
  assert.equal(resolved.resolution_commit,'a'.repeat(40));
  assert.throws(()=>betaFeedbackRecord({category:'other',status:'resolved'}));
  assert.throws(()=>betaFeedbackRecord({category:'other',feedbackId:'a',status:'duplicate',duplicateOf:'a'}));
  assert.throws(()=>importBetaFeedback(JSON.stringify({...record,ui:{...record.ui,route:'/workspace#secret'}})));
  assert.throws(()=>importBetaFeedback('x'.repeat(4500001)));
  const legacy={...record,schema:'codeshouldbehave-beta-feedback-v1'};delete legacy.screenshot;
  assert.equal(importBetaFeedback(JSON.stringify(legacy)).schema,'codeshouldbehave-beta-feedback-v2');
});

test('workspace beta theme is dark, technical and avoids tiny text', () => {
  const css = readFileSync(join(__dirname, '..', 'src', 'workspace.css'), 'utf8');
  const html = readFileSync(join(__dirname, '..', 'src', 'workspace.html'), 'utf8');
  assert.match(html, /name="color-scheme" content="dark"/);
  assert.match(css, /Phase 3 beta visual system/);
  assert.match(css, /font:15px\/1\.5 ui-monospace/);
  assert.match(css, /\.feedback-dialog\{width:min\(760px/);
  assert.match(css, /\.architecture-svg \.node text[^}]+font-size:14px!important/);
});

test('left rail exposes file deltas and snapshot review action after selection', () => {
  const css = readFileSync(join(__dirname, '..', 'src', 'workspace.css'), 'utf8');
  const html = readFileSync(join(__dirname, '..', 'src', 'workspace.html'), 'utf8');
  assert.match(html, /id="selected-file-actions"[^>]+hidden/);
  assert.match(html, /id="toggle-file-reviewed"[^>]+aria-pressed="false"/);
  assert.match(css, /\.pr-card\[aria-selected=true\]\{background:#17243a/);
  assert.match(css, /\.file-additions\{color:#5ed39a/);
  assert.match(css, /\.file-deletions\{color:#ef8290/);
});

test('review header keeps commits visible and moves secondary evidence into accordions', () => {
  const html = readFileSync(join(__dirname, '..', 'src', 'workspace.html'), 'utf8');
  assert.doesNotMatch(html, /class="review-statuses"/);
  assert.match(html, /class="review-identity"[^>]*>.*id="review-title".*id="revisions"/s);
  assert.match(html, /<details class="change-brief disclosure-panel">/);
  assert.match(html, /<details class="evidence-panel disclosure-panel">/);
});

test('review layout starts with modes and design boundary and ends with continuity', () => {
  const html = readFileSync(join(__dirname, '..', 'src', 'workspace.html'), 'utf8');
  const modes = html.indexOf('class="workspace-modes"');
  const boundary = html.indexOf('class="design-boundary disclosure-panel"');
  const revision = html.indexOf('class="revision-bar"');
  const architectureImpact = html.indexOf('id="architecture-impact"');
  const workspace = html.indexOf('id="workspace-split"');
  const continuity = html.indexOf('class="continuity-panel disclosure-panel"');
  assert.ok(modes > 0 && modes < architectureImpact && architectureImpact < workspace && workspace < revision && revision < boundary);
  assert.ok(workspace < continuity);
  assert.match(html, /class="header-view"[^>]*>.*<select id="side"/s);
  assert.doesNotMatch(html, /id="coverage"/);
  assert.match(html, /class="review-toolbar"[^>]*>.*class="workspace-modes".*class="header-view"/s);
});
