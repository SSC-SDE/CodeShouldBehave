// Exercise data-selection boundaries without a browser or third-party packages.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const vm = require('node:vm');

function harness(architecture, reviewFiles = []) {
  class Element {
    constructor(value = '') { this.value = value; this.handlers = {}; this.children = []; this.dataset = {}; this.attributes = {}; this.classes = new Set(); this.classList = {toggle: (name, on) => on ? this.classes.add(name) : this.classes.delete(name), contains: name => this.classes.has(name)}; }
    addEventListener(name, callback) { this.handlers[name] = callback; }
    click() { this.handlers.click?.(); }
    getAttribute(key) { return this.attributes[key]; }
    querySelector(selector) { return this.parts?.[selector]; }
    focus() {}
    setAttribute(key, value) { this.attributes[key] = value; }
    contains(element) { return this.children.includes(element); }
    append(...children) { this.children.push(...children); }
    replaceChildren() { this.children = []; }
  }
  const elements = new Map();
  const get = selector => {
    if (!elements.has(selector)) elements.set(selector, new Element());
    return elements.get(selector);
  };
  const files = reviewFiles.map((path, index) => {
    const button = new Element(); button.dataset.file = String(index);
    const label = new Element(); label.textContent = path;
    button.parts = {'.file-path':label, '.reviewed-mark':new Element()};
    return button;
  });
  const tabs = reviewFiles.length ? ['source', 'architecture', 'charts', 'flow', 'context', 'summary'].map(view => {
    const tab = get('#tab-' + view); tab.dataset.view = view; return tab;
  }) : [];
  const panels = reviewFiles.map((_, i) => { const panel = new Element(); panel.dataset.panel = String(i); return panel; });
  if (reviewFiles.length) get('#observation-data').value = JSON.stringify({repository:'test', baseline:'a'.repeat(40), proposal:'b'.repeat(40),files:reviewFiles,warnings:[]});
  for (const key of ['identity','scope','coverage']) get('#check-' + key).value = 'unknown';
  const windowHandlers = {}, location = {hash:''};
  const choices = [new Element(), new Element()];
  choices.forEach((choice, index) => { choice.dataset.context = String(index); choice.checked = true; });
  get('#markdown').value = 'PRIMARY_PR';
  if (architecture) get('#arch-data').value = JSON.stringify(architecture);
  get('#arch-mode').value = 'diff';
  get('#context-md-0').value = 'SELECTED_EVIDENCE';
  get('#context-md-1').value = 'PRIVATE_UNSELECTED_EVIDENCE';
  get('#context-scope-0').value = 'repository\tA\t/repo-a\tcommit-a\tsrc/\t-';
  get('#context-scope-1').value = 'service\tB\t/repo-b\tcommit-b\tcontracts/\t-';
  let download;
  const chart = new Element(), trigger = new Element(), preview = new Element(), hint = new Element();
  hint.textContent = 'Inspect source';
  chart.children = [trigger, preview, hint];
  chart.querySelector = selector => ({'.chart-trigger':trigger, '.chart-preview':preview, '.chart-hint':hint})[selector];
  const sandbox = {
    document: { querySelector: get, querySelectorAll: selector => selector === '[data-file]' ? files : selector === '[data-view]' ? tabs : selector === '[data-panel]' ? panels : selector === '[data-context]' ? choices : selector === '[data-chart-node]' ? [chart] : [], createElement: () => new Element() },
    URLSearchParams, Blob, URL: { createObjectURL(blob) { download = blob; return 'blob:test'; }, revokeObjectURL() {} }, setTimeout() {},
    window: {location, history: {pushState(_, __, hash) { location.hash = hash; }}, addEventListener(name, fn) { windowHandlers[name] = fn; }}, requestAnimationFrame() {},
  };
  vm.runInNewContext(readFileSync(require('node:path').join(__dirname, '../src/ui.js'), 'utf8'), sandbox);
  return { get, files, panels, location, windowHandlers, choices, chart, trigger, preview, text: () => download.text() };
}

test('deselected evidence is absent from both Markdown and saved scope', async () => {
  const h = harness(); h.choices[1].checked = false; h.choices[1].handlers.change();
  h.get('#export').click();
  const text = await h.text();
  assert.ok(text.includes('PRIMARY_PR'));
  assert.ok(text.includes('SELECTED_EVIDENCE'));
  assert.ok(!text.includes('PRIVATE_UNSELECTED_EVIDENCE'));
  h.get('#save-scope').click();
  const scope = await h.text();
  assert.ok(scope.includes('commit-a')); assert.ok(!scope.includes('/repo-b'));
});

test('architecture drill-down shows baseline/proposal and filters removed/added files', () => {
  const node = (path,status,sourceBefore,sourceAfter) => ({path,status,sourceBefore,sourceAfter,definitionsBefore:[],definitionsAfter:[],callsBefore:[],callsAfter:[]});
  const h = harness({nodes:[node('src/api.rs','modified','OLD_ONLY\nshared','NEW_ONLY\nshared'),node('src/old.rs','removed','removed',''),node('src/new.rs','added','','added')],edges:[]});
  h.get('#arch-boxes').children.find(b=>b.dataset.path==='src').click();
  assert.equal(h.get('#arch-boxes').children.length,3);
  h.get('#arch-boxes').children.find(b=>b.dataset.path==='src/api.rs').click();
  const text = element => [element.textContent || '',...element.children.map(text)].join(' ');
  assert.match(text(h.get('#arch-detail')),/OLD_ONLY/);assert.match(text(h.get('#arch-detail')),/NEW_ONLY/);
  h.get('#arch-mode').value='before';h.get('#arch-mode').handlers.change();
  assert.ok(!h.get('#arch-boxes').children.some(b=>b.dataset.path==='src/new.rs'));
  h.get('#arch-mode').value='after';h.get('#arch-mode').handlers.change();
  assert.ok(!h.get('#arch-boxes').children.some(b=>b.dataset.path==='src/old.rs'));
  h.get('#arch-root').click();assert.equal(h.get('#arch-boxes').children.length,1);
});

test('review command builder maps local PR refs and quotes shell-sensitive paths', () => {
  const h=harness({nodes:[],edges:[]});
  for (const [key,value] of Object.entries({'arch-repo':"/code/it's $(private)",'arch-cache':'/cache','arch-base':'abc123','arch-head':'42','arch-base-kind':'ref','arch-head-kind':'pr'})) h.get('#'+key).value=value;
  h.get('#arch-command-build').click();
  const command=h.get('#arch-commands').textContent;
  assert.ok(command.includes("--repo '/code/it'\\''s $(private)'"));
  assert.ok(command.includes("--head 'refs/remotes/origin/pr/42'"));
  assert.ok(!command.includes('--merge-base'));
});

test('flowchart previews support hover, focus, pinning and Escape dismissal', () => {
  const {chart, trigger, preview} = harness();
  assert.equal(preview.hidden, true);
  chart.handlers.mouseenter();
  assert.equal(preview.hidden, false); assert.equal(trigger.attributes['aria-expanded'], 'true');
  chart.handlers.mouseleave(); assert.equal(preview.hidden, true);
  chart.handlers.focusin(); assert.equal(preview.hidden, false);
  chart.handlers.focusout({relatedTarget:null}); assert.equal(preview.hidden, true);
  trigger.click(); chart.handlers.mouseleave(); assert.equal(preview.hidden, false);
  chart.handlers.keydown({key:'Escape', preventDefault() {}});
  assert.equal(preview.hidden, true); assert.equal(trigger.attributes['aria-expanded'], 'false');
  chart.handlers.mouseenter(); assert.equal(preview.hidden, false);
});

test('scope builder rejects traversal and keeps pending sources out of current exports', async () => {
  const h = harness();
  const fields = {kind:'service', name:'New contract', repo:'/repo-new', revision:'v1', include:'../secret', exclude:''};
  for (const [field, value] of Object.entries(fields)) h.get('#scope-' + field).value = value;
  h.get('#add-context').click();
  assert.match(h.get('#scope-feedback').textContent, /not allowed/);
  h.get('#scope-include').value = 'contracts/'; h.get('#add-context').click();
  h.get('#save-scope').click();
  assert.ok((await h.text()).includes('service\tNew contract\t/repo-new\tv1\tcontracts/\t-'));
  h.get('#export').click();
  assert.ok(!(await h.text()).includes('/repo-new'));
});

test('file navigation stays linked across architecture, source and browser history', () => {
  const node = path => ({path,status:'modified',sourceBefore:'old',sourceAfter:'new',definitionsBefore:[],definitionsAfter:[],callsBefore:[],callsAfter:[]});
  const h = harness({nodes:[node('src/a.rs'),node('src/b.rs')],edges:[]}, ['src/a.rs','src/b.rs']);
  assert.equal(h.get('#view-source').hidden, false);
  h.get('#next-file').click();
  assert.equal(h.get('#active-file').textContent, 'src/b.rs');
  assert.equal(h.panels[0].hidden, true);
  h.get('#tab-architecture').click();
  assert.ok(h.get('#arch-detail').children.some(child => child.textContent === 'src/b.rs'));
  h.get('#arch-boxes').children.find(button => button.dataset.path === 'src/a.rs').click();
  assert.equal(h.get('#active-file').textContent, 'src/a.rs');
  h.location.hash = '#view=source&file=1'; h.windowHandlers.popstate();
  assert.equal(h.get('#active-file').textContent, 'src/b.rs');
  assert.equal(h.get('#view-source').hidden, false);
  h.get('#open-summary').click();
  h.files[0].click();
  assert.equal(h.get('#view-source').hidden, false);
  h.get('#search').handlers.input({target:{value:'missing'}});
  assert.equal(h.get('#next-file').disabled,true);
});

test('observations preserve unknowns and never accept an unfinished review', async () => {
  const h = harness(undefined, ['a.md','b.md']);
  assert.equal(h.get('#observation-accept').disabled,true);
  h.get('#observation-reviewer').value = 'reviewer';
  h.get('#observation-notes').value = 'Navigation blocker';
  h.get('#observation-stop').click();
  const stopped = JSON.parse(await h.text());
  assert.equal(stopped.decision,'stop');
  assert.equal(stopped.checks.scope,'unknown');
  assert.equal(stopped.live_publication_freshness,'not_tested_static_report');
  h.get('#mark-reviewed').click(); h.get('#next-file').click(); h.get('#mark-reviewed').click();
  for (const key of ['identity','scope','coverage']) { h.get('#check-' + key).value = 'yes'; h.get('#check-' + key).handlers.change(); }
  assert.equal(h.get('#observation-accept').disabled,false);
  h.get('#observation-accept').click();
  const accepted = JSON.parse(await h.text());
  assert.equal(accepted.baseline,'a'.repeat(40));
  assert.deepEqual(accepted.reviewed_files,['a.md','b.md']);
  h.get('#mark-reviewed').click();
  assert.equal(h.get('#observation-accept').disabled,true);
});

test('summary can mark every changed file reviewed without bypassing checks', () => {
  const h = harness(undefined, ['a.md','b.md','c.md']);
  h.get('#open-summary').click();
  assert.equal(h.get('#summary-progress').textContent, '0 / 3 changed files reviewed');
  h.get('#mark-all-reviewed').click();
  assert.equal(h.get('#summary-progress').textContent, '3 / 3 changed files reviewed');
  assert.equal(h.get('#mark-all-reviewed').disabled, true);
  assert.equal(h.get('#mark-all-reviewed').textContent, 'All files reviewed');
  assert.equal(h.get('#unreviewed-files').children.length, 0);
  assert.equal(h.get('#observation-accept').disabled, true);
});

test('context navigation cannot accidentally mark the previous changed file reviewed', () => {
  const node = (path, status) => ({path,status,sourceBefore:null,sourceAfter:null,definitionsBefore:[],definitionsAfter:[],callsBefore:[],callsAfter:[]});
  const h = harness({nodes:[node('src/a.rs','modified'),node('src/context.rs','unchanged')],edges:[]}, ['src/a.rs']);
  h.get('#tab-architecture').click();
  h.get('#arch-boxes').children.find(button => button.dataset.path === 'src/context.rs').click();
  assert.equal(h.get('#mark-reviewed').disabled,true);
  assert.match(h.get('#active-file').textContent,/context file/);
  h.get('#tab-source').click();
  assert.equal(h.get('#source-unavailable').hidden,false);
  h.get('#next-file').click();
  assert.equal(h.get('#active-file').textContent,'src/a.rs');
  assert.equal(h.get('#source-unavailable').hidden,true);
});

test('standalone reviews use the approved dark terminal workspace theme', () => {
  const path = require('node:path');
  const report = readFileSync(path.join(__dirname, '../src/report.rs'), 'utf8');
  const theme = readFileSync(path.join(__dirname, '../src/report-dark.css'), 'utf8');
  assert.match(report, /include_str!\("report-dark\.css"\)/);
  assert.match(report, /CodeShouldBehave<span class='beta'>LOCAL/);
  assert.match(theme, /color-scheme:dark/);
  assert.match(theme, /font:15px\/1\.5 ui-monospace/);
  assert.match(theme, /\.file-button\.selected\{background:#17243a/);
  assert.match(theme, /\.diff-add\{background:#102a22\}/);
});
