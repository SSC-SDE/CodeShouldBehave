'use strict';

// State controller for the service-backed review workspace. It owns request
// generations so a delayed response can never replace a newer review or symbol.
class ReviewWorkspace {
  constructor({request, onState}) {
    this.request = request;
    this.onState = onState || (() => {});
    this.generation = 0;
    this.hldReadGeneration = 0;
    this.hldPreviewGeneration = 0;
    this.lldReadGeneration = 0;
    this.lldDetailGeneration = 0;
    this.lldSourceGeneration = 0;
    this.lldPreviewGeneration = 0;
    this.controllers = new Set();
    this.requestPaths = new Map();
    this.state = {
      reviewId: null,
      details: null,
      progress: null,
      files: null,
      findings: null,
      annotations: null,
      graph: null,
      source: null,
      sourceComparison: null,
      hld: null,
      hldError: null,
      hldPreview: null,
      hldDraft: null,
      lld: null,
      lldSelection: null,
      lldDetail: null,
      lldSource: null,
      lldSourceComparison: null,
      lldError: null,
      lldPreview: null,
      lldDraft: null,
      lldHistory: null,
      lldBaselineEntry: null,
      lldCorrespondenceKind: 'rename',
      selection: null,
      loading: new Set(),
      error: null,
      timings: [],
    };
  }

  snapshot() {
    return {...this.state, loading: new Set(this.state.loading)};
  }

  emit() { this.onState(this.snapshot()); }

  cancelObsolete() {
    for (const controller of this.controllers) controller.abort();
    this.controllers.clear();
    this.requestPaths.clear();
  }

  cancelLldRequests(operations = null) {
    for(const [controller,record] of this.requestPaths){if(!record.path.includes('/lld/'))continue;const operation=record.path.split('/lld/')[1].split('?')[0];if(!operations || operations.includes(operation) || operations.includes(record.group)){controller.abort();this.requestPaths.delete(controller);this.controllers.delete(controller);}}
  }

  clearReview(error = 'Current access does not authorize this review') {
    this.cancelObsolete(); ++this.generation; ++this.hldReadGeneration; ++this.hldPreviewGeneration;
    this.invalidateLld();
    for(const field of ['details','progress','files','findings','annotations','graph','source','sourceComparison','selection','hld','hldPreview','hldDraft'])this.state[field]=null;
    this.state.loading.clear();this.state.error=error;this.state.hldError=error;this.state.lldError=error;this.emit();
  }

  async selectReview(reviewId) {
    if (!reviewId || reviewId.length > 128) throw new Error('Invalid review identity');
    this.cancelObsolete();
    const generation = ++this.generation;
    ++this.hldReadGeneration; ++this.hldPreviewGeneration;
    this.invalidateLld();
    this.state = {
      reviewId,
      details: null,
      progress: null,
      files: null,
      findings: null,
      annotations: null,
      graph: null,
      source: null,
      sourceComparison: null,
      hld: null,
      hldError: null,
      hldPreview: null,
      hldDraft: null,
      lld: null,
      lldSelection: null,
      lldDetail: null,
      lldSource: null,
      lldSourceComparison: null,
      lldError: null,
      lldPreview: null,
      lldDraft: null,
      lldHistory: null,
      lldBaselineEntry: null,
      lldCorrespondenceKind: 'rename',
      selection: null,
      loading: new Set(['review', 'graph', 'files', 'findings', 'annotations']),
      error: null,
      timings: [],
    };
    this.emit();
    const [details, progress, graph, files, findings, annotations] = await Promise.allSettled([
      this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}`, generation),
      this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/progress`, generation),
      this.fetchGraph(reviewId, new URLSearchParams({side:'diff',limit:'100'}), 'diff', generation),
      this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/files?limit=100`, generation),
      this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/findings?limit=100`, generation),
      this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/annotations`, generation),
    ]);
    if (generation !== this.generation) return;
    this.state.loading.clear();
    if (details.status === 'fulfilled') this.state.details = details.value;
    if (progress.status === 'fulfilled') this.state.progress = progress.value;
    if (graph.status === 'fulfilled') this.state.graph = graph.value;
    if (files.status === 'fulfilled') this.state.files = files.value;
    if (findings.status === 'fulfilled') this.state.findings = findings.value;
    if (annotations.status === 'fulfilled') this.state.annotations = annotations.value;
    const failure = [details, progress, graph, files, findings, annotations].find(result => result.status === 'rejected');
    if ([details,progress,graph,files,findings,annotations].some(result=>result.status==='rejected' && [401,403,404].includes(result.reason?.status))) {
      this.state.details=null; this.state.progress=null; this.state.files=null; this.state.findings=null; this.state.annotations=null; this.state.graph=null;
    }
    this.state.error = failure?.reason?.message || null;
    this.emit();
  }

  async writeAnnotation(annotation) {
    const reviewId = this.state.reviewId; if (!reviewId) return;
    const generation = this.generation; this.state.loading.add('annotations'); this.emit();
    try {
      const written = await this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/annotations`, generation, {method:'PUT',headers:{'Content-Type':'application/json'},body:JSON.stringify(annotation)});
      const items = [...(this.state.annotations?.items || [])]; const index = items.findIndex(item => item.id === written.id);
      if (index < 0) items.push(written); else items[index] = written;
      this.state.annotations = {...this.state.annotations,api_version:'v1',review_id:reviewId,items}; this.state.error = null;
      return written;
    } finally { if (generation === this.generation) { this.state.loading.delete('annotations'); this.emit(); } }
  }

  async loadHld() {
    const reviewId = this.state.reviewId;
    if (!reviewId || this.state.loading.has('hld')) return;
    const generation = this.generation;
    const hldGeneration=++this.hldReadGeneration;
    this.state.loading.add('hld'); this.state.hldError = null; this.emit();
    try {
      const value = await this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/hld`, generation);
      if (generation !== this.generation || hldGeneration!==this.hldReadGeneration) return;
      if (value.review_id !== reviewId) throw new Error('HLD response does not match this review');
      this.state.hld = value;
    } catch (error) {
      if (generation === this.generation && hldGeneration===this.hldReadGeneration && error.name !== 'AbortError') { this.state.hld = null; this.state.hldPreview = null; this.state.hldError = error.message; }
    } finally {
      if (generation === this.generation && hldGeneration===this.hldReadGeneration) { this.state.loading.delete('hld'); this.emit(); }
    }
  }

  invalidateHldPreview() { ++this.hldPreviewGeneration; this.state.hldPreview = null; }

  invalidateLld({catalog = true} = {}) {
    this.cancelLldRequests();
    ++this.lldReadGeneration; ++this.lldDetailGeneration; ++this.lldSourceGeneration;
    ++this.lldPreviewGeneration;
    if (!this.state) return;
    if (catalog) { this.state.lld = null; this.state.lldSelection = null; this.state.lldBaselineEntry=null;this.state.lldCorrespondenceKind='rename'; }
    this.state.lldDetail = null; this.state.lldSource = null; this.state.lldError = null;
    this.state.lldSourceComparison = null;
    this.state.lldPreview = null; this.state.lldDraft = null;
    this.state.lldHistory = null;
    for (const key of ['lld','lld-detail','lld-source','lld-history']) this.state.loading?.delete(key);
  }

  async loadLldCatalog({parentHld = null, component = null, cursor = null} = {}) {
    const reviewId = this.state.reviewId;
    if (!reviewId) return;
    const selection = {parentHld,component};
    const prior = this.state.lld;
    if (!cursor) this.invalidateLld();
    const generation = this.generation, token = ++this.lldReadGeneration;
    this.state.lldSelection = selection; this.state.loading.add('lld'); this.state.lldError = null; this.emit();
    const query = new URLSearchParams();
    if (parentHld) query.set('parent_hld',parentHld);
    if (component) query.set('component',component);
    if (cursor) query.set('cursor',cursor);
    try {
      const value = await this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/lld/catalog?${query}`,generation);
      if (generation !== this.generation || token !== this.lldReadGeneration) return;
      validateLldResponse(value,{reviewId,parentHld,component,scopeVersion:this.state.details?.scope_version});
      if (cursor && prior) {
        if (value.baseline?.id !== prior.baseline?.id || value.proposal?.id !== prior.proposal?.id) throw new Error('LLD catalog changed while loading the next page');
        const entries = [...(prior.entries || []),...(value.entries || [])];
        const unique = [...new Map(entries.map(item=>[(item.proposal || item.baseline)?.id,item])).values()];
        this.state.lld = {...value,entries:unique.slice(0,1000),client_omitted:Math.max(0,unique.length-1000)};
      } else this.state.lld = value;
    } catch (error) {
      if (generation === this.generation && token === this.lldReadGeneration && error.name !== 'AbortError') {
        this.state.lld = null; this.state.lldDetail = null; this.state.lldSource = null; this.state.lldSourceComparison=null;this.state.lldError = error.message;
      }
    } finally {
      if (generation === this.generation && token === this.lldReadGeneration) { this.state.loading.delete('lld'); this.emit(); }
    }
  }

  async loadLldDetail(entry, {side = 'diff',baselineEntry = null,correspondenceKind = 'rename'} = {}) {
    const reviewId = this.state.reviewId, selection = this.state.lldSelection;
    if (!reviewId || !selection || !entry) return;
    if(baselineEntry && !['rename','move'].includes(correspondenceKind))throw new Error('Invalid LLD correspondence kind');
    this.cancelLldRequests(['detail','source','history','preview','accept']);
    const generation = this.generation, token = ++this.lldDetailGeneration;
    const retained=this.state.lldDetail?.entry_id===entry && this.state.lldBaselineEntry===baselineEntry?this.state.lldDetail:null;
    const retainedSource=retained?this.state.lldSourceComparison:null;
    this.state.lldBaselineEntry=baselineEntry;
    this.state.lldCorrespondenceKind=correspondenceKind;
    ++this.lldSourceGeneration; this.state.lldDetail = retained; this.state.lldSource = null; this.state.lldError = null;
    this.state.lldSourceComparison = retainedSource;
    this.state.lldPreview = null; this.state.lldDraft = null;
    this.state.lldHistory = null;
    ++this.lldPreviewGeneration;
    this.state.loading.delete('lld-source'); this.state.loading.add('lld-detail'); this.emit();
    const query = new URLSearchParams({entry,side});
    if(baselineEntry){query.set('baseline_entry',baselineEntry);query.set('correspondence_kind',correspondenceKind);}
    if(selection.parentHld)query.set('parent_hld',selection.parentHld);
    if(selection.component)query.set('component',selection.component);
    try {
      const value = await this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/lld/detail?${query}`,generation);
      if(generation !== this.generation || token !== this.lldDetailGeneration)return;
      validateLldResponse(value,{reviewId,...selection,entry,scopeVersion:this.state.lld?.scope_version});
      for(const side of ['baseline','proposal'])if(value[side] && value[side].catalog_id!==this.state.lld?.[side]?.id)throw new Error('LLD detail belongs to a different immutable catalog');
      if(baselineEntry && value.baseline?.entry.id!==baselineEntry)throw new Error('LLD correspondence does not match the selected baseline occurrence');
      this.state.lldDetail = value;
      if(!retainedSource || retained.baseline?.id!==value.baseline?.id || retained.proposal?.id!==value.proposal?.id){this.state.lldSourceComparison=null;this.loadLldSourcePair(value);}
    } catch(error) {
      if(generation === this.generation && token === this.lldDetailGeneration && error.name !== 'AbortError'){this.state.lldDetail=null;this.state.lldSourceComparison=null;this.state.lldError=error.message;}
    } finally {
      if(generation === this.generation && token === this.lldDetailGeneration){this.state.loading.delete('lld-detail');this.emit();}
    }
  }

  async loadLldSource(evidence) {
    const reviewId=this.state.reviewId, selection=this.state.lldSelection;
    if(!reviewId || !selection || !evidence?.id)return;
    this.cancelLldRequests(['lld-hover']);
    const generation=this.generation, detailGeneration=this.lldDetailGeneration, token=++this.lldSourceGeneration;
    this.state.lldSource=null;this.state.loading.add('lld-source');this.emit();
    evidence={...evidence,end_line:Math.min(evidence.end_line,evidence.start_line+199)};
    const query=new URLSearchParams({entry:this.state.lldDetail?.entry_id || '',evidence:evidence.id,start_line:String(evidence.start_line),end_line:String(evidence.end_line)});
    if(selection.parentHld)query.set('parent_hld',selection.parentHld);
    if(selection.component)query.set('component',selection.component);
    if(this.state.lldBaselineEntry){query.set('baseline_entry',this.state.lldBaselineEntry);query.set('correspondence_kind',this.state.lldCorrespondenceKind);}
    try {
      const value=await this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/lld/source?${query}`,generation,{},'lld-hover');
      if(generation!==this.generation || detailGeneration!==this.lldDetailGeneration || token!==this.lldSourceGeneration)return;
      validateLldResponse(value,{reviewId,...selection,scopeVersion:this.state.lld?.scope_version});
      if(value.evidence_id!==evidence.id || value.commit!==evidence.commit || value.path!==evidence.path || value.blob_id!==(evidence.blob_id || evidence.blob) || value.start_line!==evidence.start_line || !Number.isSafeInteger(value.end_line) || value.end_line<value.start_line || value.end_line>evidence.end_line || typeof value.text!=='string')throw new Error('LLD source does not match the selected immutable evidence');
      this.state.lldSource={...value,evidence_id:evidence.id};return this.state.lldSource;
    } catch(error) {
      if(generation===this.generation && token===this.lldSourceGeneration && error.name!=='AbortError')this.state.lldError=error.message;
    } finally {
      if(generation===this.generation && token===this.lldSourceGeneration){this.state.loading.delete('lld-source');this.emit();}
    }
  }

  async loadLldSourcePair(detail) {
    const reviewId=this.state.reviewId,selection=this.state.lldSelection,generation=this.generation,token=this.lldDetailGeneration;
    const load=async(side)=>{
      const evidence=lldEvidence(detail[side])[0];if(!evidence)return null;
      const bounded={...evidence,end_line:Math.min(evidence.end_line,evidence.start_line+199)};
      const query=new URLSearchParams({entry:detail.entry_id,evidence:bounded.id,start_line:String(bounded.start_line),end_line:String(bounded.end_line)});
      if(selection.parentHld)query.set('parent_hld',selection.parentHld);if(selection.component)query.set('component',selection.component);
      if(this.state.lldBaselineEntry){query.set('baseline_entry',this.state.lldBaselineEntry);query.set('correspondence_kind',this.state.lldCorrespondenceKind);}
      const value=await this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/lld/source?${query}`,generation);
      validateLldResponse(value,{reviewId,...selection,scopeVersion:detail.scope_version});
      if(value.evidence_id!==bounded.id||value.commit!==bounded.commit||value.path!==bounded.path||value.blob_id!==(bounded.blob_id||bounded.blob)||value.start_line!==bounded.start_line||!Number.isSafeInteger(value.end_line)||value.end_line<value.start_line||value.end_line>bounded.end_line||typeof value.text!=='string')throw new Error('LLD before/after source does not match its immutable evidence');
      return {...value,truncated:value.truncated || bounded.end_line<evidence.end_line};
    };
    const results=await Promise.allSettled([load('baseline'),load('proposal')]);
    if(generation!==this.generation||token!==this.lldDetailGeneration||!this.state.lldDetail)return;
    this.state.lldSourceComparison={baseline:results[0].status==='fulfilled'?results[0].value:null,proposal:results[1].status==='fulfilled'?results[1].value:null,errors:results.filter(result=>result.status==='rejected').map(result=>result.reason.message)};this.emit();
  }

  async previewLld(body) {
    const reviewId=this.state.reviewId,selection=this.state.lldSelection,detail=this.state.lldDetail;
    if(!reviewId || !detail?.can_accept)throw new Error('LLD confirmation requires current maintainer access');
    const generation=this.generation,token=this.lldDetailGeneration,previewToken=++this.lldPreviewGeneration,serialized=JSON.stringify(body),draft=this.state.lldDraft;
    this.state.lldPreview=null;
    const value=await this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/lld/preview`,generation,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({...body,parent_hld:selection.parentHld,component:selection.component,...(this.state.lldBaselineEntry?{baseline_entry:this.state.lldBaselineEntry,correspondence_kind:this.state.lldCorrespondenceKind}:{}),expected_revision:draft?.expected_revision||0,...(draft?{preview_id:draft.preview_id}:{})})});
    if(generation!==this.generation || token!==this.lldDetailGeneration)throw new DOMException('Obsolete LLD confirmation','AbortError');
    validateLldResponse(value,{reviewId,...selection});
    if(!value.preview_id || !Number.isSafeInteger(value.expected_revision) || typeof value.input_fingerprint!=='string')throw new Error('LLD preview is missing immutable confirmation pins');
    this.state.lldDraft={preview_id:value.preview_id,expected_revision:value.expected_revision};
    if(previewToken!==this.lldPreviewGeneration)throw new Error('The assertion changed while its preview was loading; preview again');
    this.state.lldPreview={response:value,request:serialized};this.emit();return value;
  }

  invalidateLldPreview() {++this.lldPreviewGeneration;this.state.lldPreview=null;}

  async loadLldHistory(entry) {
    const reviewId=this.state.reviewId,selection=this.state.lldSelection;
    if(!reviewId || !selection || !entry)return;
    const generation=this.generation,token=this.lldDetailGeneration,query=new URLSearchParams({entry});
    if(selection.parentHld)query.set('parent_hld',selection.parentHld);if(selection.component)query.set('component',selection.component);
    if(this.state.lldBaselineEntry){query.set('baseline_entry',this.state.lldBaselineEntry);query.set('correspondence_kind',this.state.lldCorrespondenceKind);}
    this.state.lldHistory=null;this.state.loading.add('lld-history');this.emit();
    try{const value=await this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/lld/history?${query}`,generation);if(generation!==this.generation||token!==this.lldDetailGeneration)return;validateLldResponse(value,{reviewId,...selection,entry,scopeVersion:this.state.lld?.scope_version});this.state.lldHistory=value;}
    catch(error){if(generation===this.generation&&token===this.lldDetailGeneration&&error.name!=='AbortError')this.state.lldError=error.message;}
    finally{if(generation===this.generation&&token===this.lldDetailGeneration){this.state.loading.delete('lld-history');this.emit();}}
  }

  async acceptLld(body,{knownGapsAccepted=false,activateDefault=false}={}) {
    const preview=this.state.lldPreview,reviewId=this.state.reviewId,selection=this.state.lldSelection;
    if(!preview || preview.request!==JSON.stringify(body))throw new Error('Preview the current LLD assertion before confirming');
    const generation=this.generation,token=this.lldDetailGeneration;
    const value=await this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/lld/accept`,generation,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({parent_hld:selection.parentHld,component:selection.component,entry:body.entry,...(this.state.lldBaselineEntry?{baseline_entry:this.state.lldBaselineEntry,correspondence_kind:this.state.lldCorrespondenceKind}:{}),preview_id:preview.response.preview_id,expected_revision:preview.response.expected_revision,input_fingerprint:preview.response.input_fingerprint,known_gaps_accepted:Boolean(knownGapsAccepted),activate_default:Boolean(activateDefault)})});
    if(generation!==this.generation || token!==this.lldDetailGeneration)throw new DOMException('Obsolete LLD confirmation','AbortError');
    validateLldResponse(value,{reviewId,...selection});this.state.lldPreview=null;this.state.lldDraft=null;this.emit();return value;
  }

  async previewHld(body) {
    const reviewId = this.state.reviewId;
    if (!reviewId || !this.state.hld?.can_accept) throw new Error('HLD confirmation requires current repository maintainer access');
    const generation = this.generation;
    const previewGeneration=++this.hldPreviewGeneration;
    const serialized=JSON.stringify(body);
    this.state.hldPreview = null;
    const draft=this.state.hldDraft?.design_id===body.design_id?this.state.hldDraft:null;
    try {
      const value = await this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/hld/preview`, generation, {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({...JSON.parse(serialized),expected_revision:draft?.expected_revision || 0,...(draft?{preview_id:draft.preview_id}:{})})});
      if (generation !== this.generation || value.review_id !== reviewId || !value.preview_id || !Number.isSafeInteger(value.expected_revision) || typeof value.input_fingerprint!=='string') throw new Error('HLD preview does not match this review');
      this.state.hldDraft = {design_id:body.design_id,preview_id:value.preview_id,expected_revision:value.expected_revision};
      if(previewGeneration!==this.hldPreviewGeneration)throw new Error('Corrections changed while the preview was loading; preview again');
      this.state.hldPreview = {response:value,request:serialized};
      return value;
    } finally { if(generation===this.generation)this.emit(); }
  }

  async acceptHld(body, {activateDefault = false, knownGapsAccepted = false} = {}) {
    const preview = this.state.hldPreview;
    if (!preview || preview.request !== JSON.stringify(body)) throw new Error('Preview the current HLD corrections before confirming');
    const reviewId = this.state.reviewId, generation = this.generation;
    try {
      const value = await this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/hld/accept`, generation, {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({preview_id:preview.response.preview_id,expected_revision:preview.response.expected_revision,input_fingerprint:preview.response.input_fingerprint,known_gaps_accepted:Boolean(knownGapsAccepted),activate_default:Boolean(activateDefault)})});
      if (generation !== this.generation || value.review_id !== reviewId) throw new Error('HLD confirmation does not match this review');
      this.state.hldPreview = null; this.state.hldDraft = null;
      ++this.hldReadGeneration; this.state.loading.delete('hld');
      await this.loadHld();
      return value;
    } finally { if(generation===this.generation)this.emit(); }
  }

  async loadMoreFiles() {
    const reviewId = this.state.reviewId;
    const cursor = this.state.files?.next_cursor;
    if (!reviewId || !cursor) return;
    const generation = this.generation;
    this.state.loading.add('files'); this.emit();
    try {
      const page = await this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/files?limit=100&cursor=${encodeURIComponent(cursor)}`, generation);
      if (generation !== this.generation || page.review_id !== reviewId) return;
      const combined = [...(this.state.files?.items || []), ...(page.items || [])];
      this.state.files = {...page, items: combined.slice(0,500), client_omitted: Math.max(0,combined.length-500)};
      this.state.error = null;
    } catch (error) {
      if (generation === this.generation && error.name !== 'AbortError') this.state.error = error.message;
    } finally {
      if (generation === this.generation) { this.state.loading.delete('files'); this.emit(); }
    }
  }

  async expandComponent({path = null, cursor = null, side = 'proposal'} = {}) {
    const reviewId = this.state.reviewId;
    if (!reviewId) return;
    const generation = this.generation;
    const params = new URLSearchParams({side, limit: '100'});
    if (path) params.set('parent', path);
    if (cursor) params.set('cursor', cursor);
    this.state.loading.add('graph'); this.emit();
    try {
      const graph = await this.fetchGraph(reviewId, params, side, generation);
      if (generation !== this.generation) return;
      this.state.graph = graph;
      this.state.selection = path ? {kind: 'component', path} : null;
      this.state.error = null;
    } catch (error) {
      if (generation === this.generation && error.name !== 'AbortError') this.state.error = error.message;
    } finally {
      if (generation === this.generation) { this.state.loading.delete('graph'); this.emit(); }
    }
  }

  async selectPath({path, cursor = null, side = 'proposal'} = {}) {
    const reviewId = this.state.reviewId;
    if (!reviewId || !path) return;
    const generation = this.generation;
    const params = new URLSearchParams({side, path, limit: '100'});
    if (cursor) params.set('cursor', cursor);
    this.state.loading.add('graph'); this.emit();
    try {
      const graph = await this.fetchGraph(reviewId, params, side, generation);
      if (generation !== this.generation) return;
      this.state.graph = graph;
      this.state.selection = {kind: 'path', path};
      this.state.error = null;
    } catch (error) {
      if (generation === this.generation && error.name !== 'AbortError') this.state.error = error.message;
    } finally {
      if (generation === this.generation) { this.state.loading.delete('graph'); this.emit(); }
    }
  }

  async selectSymbol({symbol, path, side = 'proposal', direction = 'outgoing'}) {
    const reviewId = this.state.reviewId;
    if (!reviewId || !symbol || !path) return;
    const generation = this.generation;
    const params = new URLSearchParams({side, symbol, direction, limit: '100'});
    this.state.selection = {kind: 'symbol', symbol, path};
    this.state.loading.add('graph'); this.emit();
    try {
      const graph = await this.fetchGraph(reviewId, params, side, generation);
      if (generation !== this.generation) return;
      this.state.graph = graph;
      this.state.error = null;
    } catch (error) {
      if (generation === this.generation && error.name !== 'AbortError') this.state.error = error.message;
    } finally {
      if (generation === this.generation) { this.state.loading.delete('graph'); this.emit(); }
    }
  }

  async loadSource({path, blob, side, startLine, endLine}) {
    const reviewId = this.state.reviewId;
    if (!reviewId) return;
    const generation = this.generation;
    const params = new URLSearchParams({
      side, path, blob,
      start_line: String(startLine),
      end_line: String(endLine),
    });
    this.state.loading.add('source'); this.emit();
    try {
      const source = await this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/source?${params}`, generation);
      if (generation !== this.generation || source.review_id !== reviewId) return;
      this.state.source = source;
      this.state.error = null;
    } catch (error) {
      if (generation === this.generation && error.name !== 'AbortError') this.state.error = error.message;
    } finally {
      if (generation === this.generation) { this.state.loading.delete('source'); this.emit(); }
    }
  }

  async loadSourceComparison({path, baseline = null, proposal = null}) {
    const reviewId = this.state.reviewId;
    if (!reviewId || !path || (!baseline && !proposal)) return;
    const generation = this.generation;
    const load = async (side, descriptor) => {
      if (!descriptor?.blob_id) return null;
      const params = new URLSearchParams({
        side, path, blob: descriptor.blob_id,
        start_line: String(descriptor.start_line), end_line: String(descriptor.end_line),
      });
      return this.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/source?${params}`, generation);
    };
    this.state.loading.add('source'); this.emit();
    try {
      const [before, after] = await Promise.all([load('baseline', baseline), load('proposal', proposal)]);
      if (generation !== this.generation) return;
      this.state.sourceComparison = {review_id:reviewId,path,baseline:before,proposal:after};
      this.state.source = after || before;
      this.state.error = null;
    } catch (error) {
      if (generation === this.generation && error.name !== 'AbortError') this.state.error = error.message;
    } finally {
      if (generation === this.generation) { this.state.loading.delete('source'); this.emit(); }
    }
  }

  async fetch(path, generation, options = {}, group = null) {
    const controller = new AbortController();
    const started = typeof performance !== 'undefined' ? performance.now() : Date.now();
    this.controllers.add(controller);
    this.requestPaths.set(controller,{path,group});
    const lldGeneration=this.lldDetailGeneration,lldReadGeneration=this.lldReadGeneration;
    try {
      const value = await this.request(path, {...options, signal: controller.signal});
      if (generation !== this.generation) throw new DOMException('Obsolete request', 'AbortError');
      return value;
    } catch (error) {
      if (generation === this.generation && (error.invalidateReview || [401, 403].includes(error.status) || (error.status === 404 && !path.includes('/hld') && !path.includes('/lld') && !error.publicFrontier))) {
        this.clearReview(error.message);throw error;
      }
      const currentLldRequest=path.includes('/lld') && (path.includes('/lld/catalog?')?lldReadGeneration===this.lldReadGeneration:lldGeneration===this.lldDetailGeneration);
      if(generation===this.generation && error.status===404 && currentLldRequest && ['lld_unavailable_in_current_context','repository_unavailable'].includes(error.message)){this.clearReview(error.message);throw error;}
      if(generation===this.generation && error.status===404 && currentLldRequest){this.invalidateLld();this.state.lldError=error.message;this.emit();}
      throw error;
    } finally {
      this.controllers.delete(controller);
      this.requestPaths.delete(controller);
      if (generation === this.generation) {
        const ended = typeof performance !== 'undefined' ? performance.now() : Date.now();
        const operation = path.includes('/source?') ? 'source' : path.includes('/graph?') ? 'graph' : path.includes('/annotations') ? 'annotations' : 'review';
        this.state.timings = [...this.state.timings.slice(-49), {operation,duration_ms:Number((ended-started).toFixed(2))}];
      }
    }
  }

  async fetchGraph(reviewId, params, side, generation) {
    const path = `/v1/reviews/${encodeURIComponent(reviewId)}/graph?`;
    if (side !== 'diff') return this.fetch(path + params, generation);
    const baseline = new URLSearchParams(params); baseline.set('side', 'baseline');
    const proposal = new URLSearchParams(params); proposal.set('side', 'proposal');
    const [before, after] = await Promise.all([
      this.fetch(path + baseline, generation),
      this.fetch(path + proposal, generation),
    ]);
    return mergeGraphs(before, after);
  }

  dispose() { this.cancelObsolete(); ++this.generation; }
}

function graphItems(graph) {
  if (!graph) return [];
  return graph.kind === 'children' ? graph.paths : graph.kind === 'symbols' ? graph.symbols : (graph.edges || []);
}

function filterGraphItems(graph, query = '', edgeKind = '') {
  const needle = String(query).trim().toLowerCase();
  return graphItems(graph).filter(item => {
    if (edgeKind && graph?.kind === 'neighborhood' && item.kind !== edgeKind) return false;
    return !needle || JSON.stringify(item).toLowerCase().includes(needle);
  });
}

function clusterEdges(edges) {
  const groups = new Map();
  edges.forEach(edge => {
    const key = `${edge.kind} · ${edge.source_path || 'unknown source'}`;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(edge);
  });
  return Array.from(groups);
}

function mergeGraphs(baseline, proposal) {
  if (baseline.kind !== proposal.kind) throw new Error('Graph revisions are incompatible');
  const fields = baseline.kind === 'children' ? ['paths', item => Array.isArray(item) ? item[0] : item.path]
    : baseline.kind === 'symbols' ? ['symbols', item => item.correspondence_key || item.stable_key]
      : ['edges', item => item.stable_key];
  const [field, identity] = fields;
  const before = new Map((baseline[field] || []).map(item => [identity(item), item]));
  const after = new Map((proposal[field] || []).map(item => [identity(item), item]));
  const items = [];
  for (const [key, item] of before) {
    const next = after.get(key);
    const changed = next && JSON.stringify(item) !== JSON.stringify(next);
    const marked = markGraphItem(next || item, next ? (changed ? 'modified' : 'unchanged') : 'removed', next ? 'proposal' : 'baseline');
    if (baseline.kind === 'symbols') { marked.baseline = item; marked.proposal = next || null; }
    items.push(marked);
  }
  for (const [key, item] of after) if (!before.has(key)) {
    const marked = markGraphItem(item, 'added', 'proposal');
    if (baseline.kind === 'symbols') { marked.baseline = null; marked.proposal = item; }
    items.push(marked);
  }
  return {kind: baseline.kind, [field]: items, next_cursor: baseline.next_cursor || proposal.next_cursor, omitted: Number(baseline.omitted || 0) + Number(proposal.omitted || 0), diff: true};
}

function markGraphItem(item, status, side) {
  if (Array.isArray(item)) return {path: item[0], kind: item[1], status, side};
  return {...item, status, side};
}

function diffLines(beforeText = '', afterText = '') {
  const before = beforeText === '' ? [] : String(beforeText).split('\n').slice(0,500);
  const after = afterText === '' ? [] : String(afterText).split('\n').slice(0,500);
  const table = Array.from({length:before.length + 1}, () => new Uint16Array(after.length + 1));
  for (let left = before.length - 1; left >= 0; left--) for (let right = after.length - 1; right >= 0; right--) {
    table[left][right] = before[left] === after[right] ? table[left + 1][right + 1] + 1 : Math.max(table[left + 1][right], table[left][right + 1]);
  }
  const operations=[]; let left=0,right=0;
  while (left < before.length || right < after.length) {
    if (left < before.length && right < after.length && before[left] === after[right]) { operations.push({kind:'same',before:before[left++],after:after[right++]}); continue; }
    if (right < after.length && (left === before.length || table[left][right + 1] >= table[left + 1][right])) { operations.push({kind:'added',before:null,after:after[right++]}); continue; }
    operations.push({kind:'removed',before:before[left++],after:null});
  }
  const rows=[];
  for (let index=0; index<operations.length; index++) {
    const operation=operations[index], next=operations[index+1];
    if (operation.kind==='removed' && next?.kind==='added') { rows.push({kind:'modified',before:operation.before,after:next.after}); index++; }
    else if (operation.kind==='added' && next?.kind==='removed') { rows.push({kind:'modified',before:next.before,after:operation.after}); index++; }
    else rows.push(operation);
  }
  return rows;
}

function architectureMeaning(path = '') {
  const value=String(path).toLowerCase();
  if (/\.(md|rst|txt)$/.test(value) || /^(docs|documentation)\//.test(value)) return {key:'documentation',title:'Documentation',purpose:'Documents intended design or usage; not a runtime component'};
  const rules=[
    [/gateway|client|integration/,['External service gateway','Calls external services and translates their responses']],
    [/payment|billing|checkout|invoice/,['Payment processing','Owns payment validation and transaction flow']],
    [/event|webhook|message|queue/,['Events and callbacks','Publishes or handles asynchronous events']],
    [/route|router|config/,['Runtime routing','Controls request routes and runtime behavior']],
    [/auth|identity|session|permission/,['Identity and access','Controls authentication and authorization']],
    [/database|storage|repository|migration|schema/,['Data access','Reads, writes or defines persisted data']],
    [/worker|job|schedule|cron/,['Background processing','Runs asynchronous or scheduled work']],
    [/api|handler|controller|endpoint/,['API boundary','Accepts requests and exposes application behavior']],
    [/frontend|web|ui|view|component/,['User interface','Presents reviewer or customer-facing behavior']],
    [/shared|common|lib|util/,['Shared library','Provides behavior reused by multiple components']],
  ];
  const matched=rules.find(([pattern])=>pattern.test(value));
  if(matched)return {key:matched[1][0].toLowerCase().replaceAll(' ','-'),title:matched[1][0],purpose:matched[1][1]};
  const root=String(path).split('/').filter(Boolean)[0] || 'Repository root';
  const title=root==='src'?'Application core':root.replace(/[-_]/g,' ').replace(/\b\w/g,letter=>letter.toUpperCase());
  return {key:`path-${root.toLowerCase()}`,title,purpose:`Contains changed code under ${root}`};
}

function architectureAreas(files = []) {
  const areas = new Map();
  for (const file of files.slice(0,500)) {
    const meaning=architectureMeaning(file.path);
    const current=areas.get(meaning.key) || {key:meaning.key,name:meaning.title,purpose:meaning.purpose,added:0,removed:0,modified:0,renamed:0,lineAdditions:0,lineDeletions:0,hasLineCounts:false,paths:[]};
    const kind=['added','deleted','modified','renamed'].includes(file.kind)?file.kind:'modified';
    current[kind==='deleted'?'removed':kind]++; current.paths.push(file.path);
    if(Number.isInteger(file.additions)&&Number.isInteger(file.deletions)){current.lineAdditions+=file.additions;current.lineDeletions+=file.deletions;current.hasLineCounts=true;}
    areas.set(meaning.key,current);
  }
  return [...areas.values()].sort((a,b)=>a.name.localeCompare(b.name)).slice(0,24);
}

function mermaidLabel(value) { return String(value).replace(/["<>\[\]{}]/g, ' ').replace(/\s+/g, ' ').trim().slice(0,120); }

function analysisBaseline(details = {}) { return details.analysis_baseline || details.revision?.base_commit || details.selected_baseline; }

function observationAge(ms, now = Date.now()) {
  if (!Number.isFinite(ms) || ms <= 0) return 'Update time unavailable';
  const hours = Math.floor(Math.max(0, now - ms) / 3600000);
  return hours < 24 ? `Updated ${hours}h ago` : `Updated ${Math.floor(hours / 24)}d ago`;
}

function filterChangedFiles(files, query) {
  const needle = String(query).trim().toLowerCase();
  return files.filter(file => !needle || `${file.path} ${file.old_path || ''}`.toLowerCase().includes(needle));
}

function diagramScale(width, viewport, mode = 'fit') {
  if (!(width > 0) || !(viewport > 0)) return 1;
  return mode === 'fit' ? Math.min(1, Math.max(0.1, (viewport - 32) / width)) : Math.min(2, Math.max(0.1, Number(mode) || 1));
}

function architectureMermaid({files = [], findings = [], details = {}}, mode = 'combined') {
  const areas=architectureAreas(files), baseline=String(analysisBaseline(details) || '').slice(0,8) || 'baseline', proposal=String(details.proposal || '').slice(0,8) || 'proposal';
  const lines=['flowchart LR'];
  const visible=mode==='baseline'?architectureAreas(files.filter(file=>file.kind!=='added')):mode==='proposal'?architectureAreas(files.filter(file=>file.kind!=='deleted')):areas;
  if (mode==='combined') { lines.push(`base["Baseline ${mermaidLabel(baseline)}"]:::same`,`proposal["Proposal ${mermaidLabel(proposal)}"]:::proposal`); }
  else lines.push(`snapshot["${mode==='baseline'?'Baseline '+mermaidLabel(baseline):'Proposal '+mermaidLabel(proposal)}"]:::${mode==='baseline'?'same':'proposal'}`);
  visible.forEach(area=>{
    const index=areas.findIndex(candidate=>candidate.key===area.key);
    const status=area.added && !area.removed && !area.modified && !area.renamed ? 'added' : area.removed && !area.added && !area.modified && !area.renamed ? 'removed' : 'modified';
    const fileStates=[area.added?`${area.added} added`:null,area.removed?`${area.removed} removed`:null,(area.modified+area.renamed)?`${area.modified+area.renamed} modified`:null].filter(Boolean).join(', ');
    const filenames=area.paths.slice(0,2).map(path=>path.split('/').pop()).join(', ')+(area.paths.length>2?` +${area.paths.length-2} more`: '');
    const evidence=findings.find(finding=>area.paths.includes(finding.source_path));
    const impact=evidence?`${String(evidence.resolution).toLowerCase()==='resolved'?'Observed':'Review'}: ${String(evidence.kind || 'impact unresolved').replaceAll('_',' ')}`:'Review: inspect changed code';
    const parts=[area.name,area.purpose,`Files: ${filenames} (${fileStates})`,area.hasLineCounts?`Lines: +${area.lineAdditions} −${area.lineDeletions}`:null,impact].filter(Boolean);
    const label=parts.map(mermaidLabel).join('<br/>');
    lines.push(`area${index}["${label}"]:::${status}`);
    if (mode==='combined') {
      if (status!=='added') lines.push(`base -->|exists before| area${index}`);
      if (status!=='removed') lines.push(`area${index} -->|changed here| proposal`);
    } else lines.push(`snapshot --> area${index}`);
  });
  lines.push('classDef same fill:#121b29,stroke:#64748b,color:#e7edf7','classDef proposal fill:#17243b,stroke:#7aa7ff,color:#f4f7ff,stroke-width:2px','classDef added fill:#10271f,stroke:#5ed39a,color:#dcfced,stroke-width:2px','classDef removed fill:#2a171d,stroke:#ef8290,color:#ffe7eb,stroke-dasharray:5 4','classDef modified fill:#292214,stroke:#f1bd62,color:#fff3d3,stroke-width:2px','classDef unknown fill:#211a30,stroke:#b69be0,color:#f0e8ff,stroke-dasharray:3 3');
  return lines.join('\n');
}

function hldOutcome(delta) {
  if (delta?.outcome === 'change_detected') return 'HLD change detected';
  if (delta?.outcome === 'no_change_detected_within_scope') return 'No HLD change detected within covered scope';
  return 'Unable to determine completely';
}

function validateLldResponse(value, {reviewId,parentHld = null,component = null,entry = null,scopeVersion = null} = {}) {
  if (!value || value.review_id !== reviewId || (value.parent_hld || null) !== parentHld || (value.component || null) !== component || (entry && value.entry_id !== entry) || typeof value.scope_version !== 'string' || (scopeVersion && value.scope_version !== scopeVersion)) throw new Error('LLD response does not match this review, scope or drill-down');
  return value;
}

function lldOutcome(delta) {
  switch(delta?.outcome) {
    case 'change_detected': return 'LLD change detected';
    case 'no_change_detected_within_scope': return 'No LLD change detected within covered scope';
    case 'incomplete': return 'LLD comparison incomplete';
    default: return 'LLD unavailable';
  }
}

function lldCatalogRows(payload = {}) {
  return (payload?.entries || []).filter(pair=>pair && (pair.baseline || pair.proposal)).slice(0,1000).map(pair=>({...pair,entry:pair.proposal || pair.baseline,id:(pair.proposal || pair.baseline).id}));
}

function lldSafeLabel(value) {
  return String(value ?? '').slice(0,180).replace(/[\r\n\t]+/g,' ').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/["|{}\[\]`\\]/g,' ').replace(/javascript:/gi,'');
}

function lldGraphView(payload = {}, mode = 'combined') {
  const sides = mode === 'baseline' ? ['baseline'] : mode === 'proposal' ? ['proposal'] : ['baseline','proposal'];
  const views=sides.filter(side=>payload[side]).map(side=>({side,detail:payload[side]}));
  return views;
}

function lldMermaid(payload = {}, mode = 'combined') {
  const lines=['flowchart TD'], views=lldGraphView(payload,mode), inventory=[];
  for(const view of views) {
    const graph=view.detail.graph || {}, nodes=(graph.nodes || []).slice(0,200).filter(item=>item.operation!=='unknown_exit'||(graph.edges || []).some(edge=>edge.from===item.id||edge.to===item.id)), ids=new Map(nodes.map((item,index)=>[item.id,`lld${inventory.length+index}`]));
    lines.push(`subgraph ${view.side}["${view.side === 'baseline'?'Baseline':'Proposal'} · ${nodes.length?'Static local control flow':'Declaration structure'}"]`);
    const drawNode=item=>{
      const status=view.side==='baseline'?'before':'after', uncertainty=item.resolution==='unknown'||String(item.resolution).includes('unresolved')||String(item.operation).startsWith('unsupported');
      const label=`${uncertainty?'? ':''}${lldSafeLabel(item.operation)} · ${lldSafeLabel(item.label || item.ast_kind)}`,condition=['condition','pattern','short_circuit','try_branch','try_propagation'].includes(item.operation);
      lines.push(`${ids.get(item.id)}${condition?'{"':'["'}${label}${condition?'"}':'"]'}:::${uncertainty?'unknown':status}`);
    };
    const regions=(graph.regions || []).slice(0,200),grouped=new Set();
    for(const [index,region] of regions.entries()){lines.push(`subgraph region${view.side}${index}["${lldSafeLabel(region.kind)} · separate exit scope"]`);for(const item of nodes.filter(item=>item.region_id===region.id)){drawNode(item);grouped.add(item.id);}lines.push('end');}
    for(const item of nodes){if(!grouped.has(item.id))drawNode(item);inventory.push({side:view.side,node:item,detail:view.detail});}
    if(!nodes.length && view.detail.entry) {
      const declaration=view.detail.entry, root=`lld${inventory.length}`;
      lines.push(`${root}["${lldSafeLabel(declaration.kind)} · ${lldSafeLabel(declaration.name)}"]:::${view.side==='baseline'?'before':'after'}`);
      inventory.push({side:view.side,node:{id:declaration.id,label:declaration.name,resolution:'declaration_structure',evidence_id:declaration.evidence_ids?.[0]},detail:view.detail,entry:declaration});
      for(const child of (view.detail.declarations || []).slice(0,100)){const id=`lld${inventory.length}`;lines.push(`${id}["${lldSafeLabel(child.kind)} · ${lldSafeLabel(child.name)}"]:::${view.side==='baseline'?'before':'after'}`,`${root} -->|contains declaration| ${id}`);inventory.push({side:view.side,node:{id:child.id,label:child.name,resolution:'declaration_structure',evidence_id:child.evidence_ids?.[0]},detail:view.detail,entry:child});}
    } else if(!nodes.length)lines.push(`empty${view.side}["Local flow unavailable for this declaration"]:::unknown`);
    for(const edge of (graph.edges || []).slice(0,400))if(ids.has(edge.from) && ids.has(edge.to))lines.push(`${ids.get(edge.from)} ${edge.resolution==='unknown'?'-.->':'-->'}|"${lldSafeLabel(edge.label || edge.kind)}"| ${ids.get(edge.to)}`);
    lines.push('end');
  }
  if(!views.length)lines.push('empty["No authorized function detail loaded"]:::unknown');
  lines.push('classDef before fill:#2b2029,stroke:#ef8290,color:#edf0f4,stroke-width:1.5px','classDef after fill:#182d27,stroke:#5ed39a,color:#edf0f4,stroke-width:1.5px','classDef unknown fill:#2d281f,stroke:#f1bd62,color:#edf0f4,stroke-dasharray:4 3');
  return {source:lines.join('\n'),nodes:inventory};
}

function lldEvidence(detail, graphNode = null) {
  const ids=graphNode?.evidence_ids || (graphNode?.evidence_id?[graphNode.evidence_id]:detail?.entry?.evidence_ids || detail?.evidence_ids || []);
  const evidence=detail?.evidence || [];
  return ids.length?evidence.filter(item=>ids.includes(item.id)):graphNode?[]:evidence;
}

function hldExportAllowed(payload, mode = 'combined') {
  if(payload?.can_export!==true)return false;
  if(mode==='combined'||mode==='compare')return payload.export_views?.baseline===true&&payload.export_views?.proposal===true;
  return ['baseline','proposal','accepted'].includes(mode)&&payload.export_views?.[mode]===true;
}

function hldComparison(payload = {}, mode = 'combined') {
  const before = mode === 'accepted' ? payload.accepted : mode === 'proposal' ? null : payload.baseline;
  const after = mode === 'baseline' || mode === 'accepted' ? null : payload.proposal;
  const elements = new Map();
  const relations = new Map();
  for (const version of [before, after]) {
    for (const element of version?.elements || []) elements.set(element.id, element);
    for (const relation of version?.relations || []) relations.set(relation.id, relation);
  }
  const status = new Map();
  if (mode === 'combined') for (const change of payload.delta?.changes || []) {
    const key=change.element_id||change.relation_id;
    if(change.kind!=='uncertain'||!status.has(key)||status.get(key)==='uncertain')status.set(key,change.kind);
    const changed=[before,after].flatMap(version=>version?.elements||[]).filter(element=>element.id===change.element_id&&element.kind==='contract');
    for(const contract of changed)if(contract.component_id&&(!status.has(contract.component_id)||(status.get(contract.component_id)==='uncertain'&&change.kind!=='uncertain')))status.set(contract.component_id,change.kind==='uncertain'?'uncertain':'modified');
  }
  const priorities={component:0,dependency:1,deployment:2,schema:3,contract:4};
  const sorted=[...elements.values()].sort((a,b)=>(priorities[a.kind]??5)-(priorities[b.kind]??5)||String(a.id).localeCompare(String(b.id))).slice(0,100);
  const diagramIds=new Set(sorted.filter(element=>element.kind!=='contract').map(element=>element.id));
  const relationPriority=relation=>diagramIds.has(relation.from)&&diagramIds.has(relation.to)?0:1;
  return {before,after,elements:sorted,relations:[...relations.values()].sort((a,b)=>relationPriority(a)-relationPriority(b)||String(a.id).localeCompare(String(b.id))).slice(0,200),status,omitted:Math.max(0,elements.size-100)+Math.max(0,relations.size-200)};
}

function hldDiagramElements(payload, mode = 'combined') { return hldComparison(payload,mode).elements.filter(element=>['component','dependency','deployment','schema'].includes(element.kind)); }

function hldMermaid(payload = {}, mode = 'combined') {
  const comparison = hldComparison(payload, mode);
  const diagramElements=hldDiagramElements(payload,mode);
  const indexes = new Map(diagramElements.map((element,index)=>[element.id,index]));
  const lines = ['flowchart LR'];
  const label = value => mermaidLabel(value).replace(/[&|`\\]/g,' ');
  for (const [index, element] of diagramElements.entries()) {
    const kind = String(element.kind || 'component').replaceAll('_',' ');
    const details = [element.name,kind,element.responsibility || (element.roots || []).join(', ')].filter(Boolean).map(label).join('<br/>');
    const impacted = mode==='combined' && (payload.delta?.possible_impacts||[]).some(impact=>impact.component_id===element.id);
    const rawStatus = comparison.status.get(element.id) || (impacted?'impacted':'same');
    const status = ['added','removed','modified','impacted'].includes(rawStatus) ? rawStatus : rawStatus === 'same' ? 'same' : 'unknown';
    lines.push(`hld${index}["${details}"]:::${status}`);
  }
  for (const relation of comparison.relations) {
    const from = indexes.get(relation.from), to = indexes.get(relation.to);
    if (from === undefined || to === undefined) continue;
    const status=comparison.status.get(relation.id),uncertain = status==='uncertain'||!['observed_structure','resolved_reference'].includes(relation.resolution);
    const marker=status==='added'?'+ ':status==='removed'?'Removed: ':status==='modified'?'Changed: ':'';
    const description = `${marker}${uncertain ? '? ' : ''}${String(relation.kind || 'interaction').replaceAll('_',' ')}${relation.protocol ? ' · '+relation.protocol : ''}`;
    lines.push(`hld${from} ${uncertain||status==='removed' ? '-.->' : '-->'}|${label(description)}| hld${to}`);
  }
  if (!diagramElements.length) lines.push('empty["No authorized HLD components available"]:::unknown');
  lines.push('classDef same fill:#20252c,stroke:#718094,color:#edf0f4','classDef impacted fill:#20252c,stroke:#9bbaff,color:#edf0f4,stroke-width:2px','classDef added fill:#163125,stroke:#5ed39a,color:#edf0f4','classDef removed fill:#351e25,stroke:#ef8290,color:#edf0f4,stroke-dasharray:5 4','classDef modified fill:#2b261c,stroke:#f1bd62,color:#edf0f4','classDef unknown fill:#251e31,stroke:#b69be0,color:#edf0f4,stroke-dasharray:5 4');
  return lines.join('\n');
}

function hldEvidenceCollection(payload, elementId, mode = 'combined') {
  const versions = mode === 'accepted' ? [payload?.accepted] : mode === 'baseline' ? [payload?.baseline] : mode === 'proposal' ? [payload?.proposal] : [payload?.baseline,payload?.proposal];
  const result = new Map();
  for (const version of versions) {
    const element = version?.elements?.find(item=>item.id===elementId);
    if (!element) continue;
    const ids = new Set([...(element.evidence_ids || []),...(element.facts || []).flatMap(fact=>fact.evidence_ids || [])]);
    for (const relation of version.relations || []) if (relation.from===elementId || relation.to===elementId) for (const id of relation.evidence_ids || []) ids.add(id);
    for(const decision of version.decision_records||[])if(decision.component_id===elementId)for(const id of decision.evidence_ids||[])ids.add(id);
    for (const evidence of version.evidence || []) if (ids.has(evidence.id)) result.set(`${version.id}\0${evidence.id}`,{...evidence,design_id:version.id});
  }
  return {items:[...result.values()].slice(0,100),omitted:Math.max(0,result.size-100)};
}

function hldEvidenceFor(payload,elementId,mode='combined'){return hldEvidenceCollection(payload,elementId,mode).items;}

function hldCorrection({id,kind,targetId,value = '',paths = [],relatedIds = [],constraintKind = null,referenceDesignId = null,correspondenceTargets = [],documentEvidenceId = null}, version, referenceVersion = null) {
  if (!version?.elements || !['responsibility','boundary','contract_ownership','data_ownership','major_flow','constraint','correspondence','document_reference'].includes(kind)) throw new Error('Choose a supported correction type');
  if(kind==='correspondence'&&referenceDesignId&&referenceVersion?.id!==referenceDesignId)throw new Error('Correspondence requires the other exact generated design snapshot');
  const source=kind==='correspondence'&&referenceDesignId?referenceVersion:version;
  const target=version.elements.find(element=>element.id===targetId), known=new Set(source.elements.map(element=>element.id));
  if(!target || relatedIds.some(item=>!known.has(item)))throw new Error('Correction elements must belong to the selected generated design');
  const statement=String(value).trim();if(!statement||/[\x00-\x1f\x7f]/.test(statement)||new TextEncoder().encode(statement).length>2048)throw new Error('Enter a correction of at most 2048 bytes without control characters');
  if(['responsibility','boundary','constraint','document_reference'].includes(kind)&&target.kind!=='component')throw new Error('This correction requires a confirmed component target');
  if(kind==='document_reference'&&!(version.document_candidates||[]).some(candidate=>candidate.evidence_id===documentEvidenceId))throw new Error('Select an observed design document from this exact generated snapshot');
  const component=id=>version.elements.some(element=>element.id===id&&element.kind==='component');
  if(kind==='boundary' && (!paths.length || paths.some(path=>!(target.roots||[]).some(root=>root==='.'||path===root||path.startsWith(root+'/')) || !((target.roots||[]).includes(path)||(version.evidence||[]).some(evidence=>(target.evidence_ids||[]).includes(evidence.id)&&(evidence.path===path||evidence.path.startsWith(path+'/')))))))throw new Error('A corrected boundary may only narrow existing evidence-backed component roots');
  if(['contract_ownership','data_ownership','major_flow','correspondence'].includes(kind)&&!relatedIds.length)throw new Error('Select related elements for this correction');
  if(kind==='constraint' && (relatedIds.length!==1 || !['prohibited_dependency','required_contract'].includes(constraintKind)))throw new Error('Choose one related element and a supported architectural constraint');
  if(kind==='constraint' && ((constraintKind==='prohibited_dependency'&&!component(relatedIds[0]))||(constraintKind==='required_contract'&&!version.elements.some(element=>element.id===relatedIds[0]&&element.kind==='contract'))))throw new Error('A dependency constraint requires a component; a contract constraint requires a contract');
  if(kind==='major_flow'&&(relatedIds.length!==2||relatedIds.some(id=>!component(id))||new TextEncoder().encode(statement).length>512))throw new Error('A major flow requires two confirmed component endpoints and at most 512 bytes');
  if(['contract_ownership','data_ownership'].includes(kind)&&(relatedIds.length!==1||!component(relatedIds[0])||target.kind!==(kind==='contract_ownership'?'contract':'schema')))throw new Error('Ownership requires a contract or schema target and one confirmed owning component');
  if(kind==='correspondence' && !['rename','move','split','merge'].includes(statement))throw new Error('Choose a supported element correspondence');
  const targets=correspondenceTargets.length?correspondenceTargets:[targetId];
  if(kind==='correspondence') {
    if(!targets.includes(targetId)||new Set(targets).size!==targets.length||new Set(relatedIds).size!==relatedIds.length||targets.some(item=>!version.elements.some(element=>element.id===item&&element.kind===target.kind))||relatedIds.some(item=>!source.elements.some(element=>element.id===item&&element.kind===target.kind)))throw new Error('Correspondence must link unique existing elements of the same architectural kind');
    if((['rename','move'].includes(statement)&&(relatedIds.length!==1||targets.length!==1))||(statement==='split'&&(relatedIds.length!==1||targets.length<2))||(statement==='merge'&&(relatedIds.length<2||targets.length!==1)))throw new Error('Rename or move requires one source and one target; split and merge require the matching multiple elements');
  }
  return {id,kind,target_id:targetId,value:statement,paths:kind==='boundary'?paths:[],related_ids:kind==='document_reference'?[]:relatedIds,constraint_kind:kind==='constraint'?constraintKind:null,reference_design_id:kind==='correspondence'?referenceDesignId:null,correspondence_targets:kind==='correspondence'?targets:[],document_evidence_id:kind==='document_reference'?documentEvidenceId:null};
}

function parseScopePaths(value, required = false) {
  const paths = String(value).split(',').map(item => item.trim()).filter(Boolean);
  if (required && !paths.length) throw new Error('At least one included path is required');
  if (paths.some(path => path.startsWith('/') || path.includes('\\') || /(^|\/)\.\.($|\/)/.test(path) || /[*?\[\]]/.test(path))) {
    throw new Error('Scope paths must be literal repository-relative paths');
  }
  return paths;
}

function summarizeReviewState({details = null, progress = null, graph = null, error = null, pr = null} = {}) {
  let analysis = 'Loading';
  let analysisDetail = 'Waiting for deterministic evidence';
  if (error) { analysis = 'Failed'; analysisDetail = error; }
  else if (details?.stale || progress?.stale) { analysis = 'Stale'; analysisDetail = 'A newer review identity is available'; }
  else if (progress?.terminal_reason) { analysis = 'Failed'; analysisDetail = progress.terminal_reason; }
  else if (progress?.job_state) {
    analysis = progress.job_state === 'completed' ? 'Ready' : progress.job_state;
    analysisDetail = progress.job_state === 'completed' ? 'Deterministic evidence is available' : 'Analysis has not produced a complete result';
  }
  const provider = pr?.state ? `PR ${pr.state}` : 'Unavailable';
  const providerDetail = 'Provider checks unavailable';
  const human = 'Not started';
  const items = graphItems(graph);
  const kind = graph?.kind || 'evidence';
  const noun = kind === 'children' ? 'affected path' : kind === 'symbols' ? 'symbol' : 'relationship';
  const observed = graph ? `${items.length} authorized ${noun}${items.length === 1 ? '' : 's'} available` : 'Loading authorized structural evidence…';
  const omitted = Number(graph?.omitted || 0);
  const partial = Array.isArray(progress?.coverage) && progress.coverage.some(item => String(item[0]).toLowerCase().includes('partial') || String(item[0]).toLowerCase().includes('omitted'));
  const limitation = omitted || partial ? `${omitted ? `${omitted} graph items omitted. ` : ''}Coverage is partial; inspect limits before drawing a system-wide conclusion.` : 'Possible behavioral effects require reviewer judgment; structural evidence is independently inspectable.';
  return {analysis, analysisDetail, provider, providerDetail, human, observed, limitation};
}

function classifyFinding(finding = {}) {
  const resolution = String(finding.resolution || '').toLowerCase();
  const observed = resolution === 'resolved';
  return {
    category: observed ? 'Observed structural evidence' : 'Possible effect / unresolved',
    tone: observed ? 'observed' : 'possible',
    limitation: observed
      ? 'Resolved in the static graph. This is not proof of runtime behavior.'
      : 'The static graph could not resolve this completely. Reviewer verification is required.',
    action: observed ? 'Inspect evidence' : 'Investigate path',
  };
}

function groupFindings(findings = []) {
  const deduplicated = new Map();
  findings.forEach(finding => {
    const classification = classifyFinding(finding);
    const key = [classification.tone, finding.kind, finding.resolution, finding.source_path, finding.source_start_line, finding.source_end_line].join('\0');
    const current = deduplicated.get(key);
    if (current) current.provenance.push(finding.id);
    else deduplicated.set(key, {...finding, classification, provenance:[finding.id]});
  });
  const groups = new Map([
    ['observed', {title:'What changed structurally?', items:[]}],
    ['possible', {title:'What still needs reviewer verification?', items:[]}],
  ]);
  deduplicated.forEach(finding => groups.get(finding.classification.tone).items.push(finding));
  return Array.from(groups.values()).filter(group => group.items.length);
}

const WORKSPACE_LAYOUTS = new Set(['connected', 'source', 'architecture']);
const WORKSPACE_SIDES = new Set(['baseline', 'proposal', 'diff']);
const WORKSPACE_SELECTIONS = new Set(['component', 'path', 'symbol']);
const LLD_INITIAL_SCALE = 1;

function encodeWorkspaceRoute({reviewId, bundleId = null, layout = 'connected', side = 'diff', selection = null, designLevel = 'structural', designMode = 'combined', designElement = null, parentHld = null, component = null, detail = null, lldElement = null, lldNode = null, lldPinned = false, lldBaselineEntry = null, lldCorrespondence = 'rename'} = {}) {
  if (!reviewId || String(reviewId).length > 128) throw new Error('Invalid review identity');
  if (!WORKSPACE_LAYOUTS.has(layout)) throw new Error('Invalid workspace layout');
  if (!WORKSPACE_SIDES.has(side)) throw new Error('Invalid comparison side');
  const route = new URLSearchParams({review: String(reviewId), layout, side});
  if(bundleId){if(!/^[0-9a-f]{64}$/.test(bundleId))throw new Error('Invalid public bundle identity');route.set('bundle',bundleId);}
  if (!['structural','hld','lld'].includes(designLevel)) throw new Error('Invalid design level');
  if (layout === 'architecture' && designLevel !== 'structural') {
    if (!['combined','compare','baseline','proposal','accepted'].includes(designMode)) throw new Error('Invalid design comparison');
    route.set('design',designLevel); route.set('design_view',designMode);
    if (designElement) { if (String(designElement).length > 256 || String(designElement).includes('\0')) throw new Error('Invalid design element'); route.set('element',String(designElement)); }
    if(designLevel==='lld') {
      for(const [key,value] of [['parent_hld',parentHld],['component',component],['lld_element',lldElement],['lld_node',lldNode],['lld_baseline_entry',lldBaselineEntry]])if(value){if(String(value).length>256||/[\x00-\x1f\x7f]/.test(String(value)))throw new Error('Invalid LLD navigation identity');route.set(key,String(value));}
      if(lldBaselineEntry){if(!['rename','move'].includes(lldCorrespondence))throw new Error('Invalid LLD correspondence');route.set('lld_correspondence',lldCorrespondence);}
      if(lldPinned && lldNode)route.set('lld_pin','1');
      if(detail){if(!['module','type','function','flow'].includes(detail))throw new Error('Invalid LLD detail level');route.set('detail',detail);}
      if(component&&designElement&&component!==designElement)throw new Error('LLD component mapping must match the selected HLD element');
    }
  }
  if (selection) {
    if (!WORKSPACE_SELECTIONS.has(selection.kind)) throw new Error('Invalid selection kind');
    const path = String(selection.path || '');
    if (!path || path.length > 1024 || path.includes('\0')) throw new Error('Invalid selection path');
    route.set('kind', selection.kind); route.set('path', path);
    if (selection.kind === 'symbol') {
      const symbol = String(selection.symbol || '');
      if (!symbol || symbol.length > 1024 || symbol.includes('\0')) throw new Error('Invalid symbol identity');
      route.set('symbol', symbol);
    }
  }
  return '#' + route.toString();
}

function decodeWorkspaceRoute(hash = '') {
  const route = new URLSearchParams(String(hash).replace(/^#/, ''));
  const reviewId = route.get('review');
  if (!reviewId) return null;
  const layout = route.get('layout') || 'connected';
  const side = route.get('side') || 'diff';
  const bundleId=route.get('bundle');if(route.has('bundle')&&(!/^[0-9a-f]{64}$/.test(bundleId)||route.getAll('bundle').length!==1||route.getAll('review').length!==1))return null;
  if (reviewId.length > 128 || !WORKSPACE_LAYOUTS.has(layout) || !WORKSPACE_SIDES.has(side)) return null;
  const kind = route.get('kind');
  const path = route.get('path');
  let selection = null;
  if (kind || path) {
    if (!WORKSPACE_SELECTIONS.has(kind) || !path || path.length > 1024 || path.includes('\0')) return null;
    selection = {kind, path};
    if (kind === 'symbol') {
      const symbol = route.get('symbol');
      if (!symbol || symbol.length > 1024 || symbol.includes('\0')) return null;
      selection.symbol = symbol;
    }
  }
  const designLevel = route.get('design');
  if (designLevel) {
    const designMode = route.get('design_view') || 'combined', designElement = route.get('element');
    if (!['structural','hld','lld'].includes(designLevel) || !['combined','compare','baseline','proposal','accepted'].includes(designMode) || (designElement && (designElement.length > 256 || designElement.includes('\0')))) return null;
    const result={reviewId,layout,side,selection,designLevel,designMode,designElement};if(bundleId)result.bundleId=bundleId;
    if(designLevel==='lld'&&['parent_hld','component','detail','lld_element'].some(key=>route.has(key))) {
      const parentHld=route.get('parent_hld'),component=route.get('component'),detail=route.get('detail'),lldElement=route.get('lld_element'),lldNode=route.get('lld_node'),lldPinned=route.get('lld_pin'),lldBaselineEntry=route.get('lld_baseline_entry'),lldCorrespondence=route.get('lld_correspondence');
      if([parentHld,component,lldElement,lldNode,lldBaselineEntry].some(value=>value&&(value.length>256||/[\x00-\x1f\x7f]/.test(value)))||(detail&&!['module','type','function','flow'].includes(detail))||(component&&designElement&&component!==designElement)||(lldPinned && (lldPinned!=='1'||!lldNode))||(lldCorrespondence && (!lldBaselineEntry||!['rename','move'].includes(lldCorrespondence))))return null;
      Object.assign(result,{parentHld,component,detail,lldElement});
      if(lldNode)Object.assign(result,{lldNode,lldPinned:lldPinned==='1'});
      if(lldBaselineEntry)Object.assign(result,{lldBaselineEntry,lldCorrespondence:lldCorrespondence || 'rename'});
    }
    return result;
  }
  return {...{reviewId, layout, side, selection},...(bundleId?{bundleId}:{})};
}

function resolvePublicWorkspaceRoute(transport,hash='') {
  const route=decodeWorkspaceRoute(hash),values=new URLSearchParams(String(hash).replace(/^#/,''));
  if(!route){if(values.has('review')||values.has('bundle'))throw new Error('Published comparison link is invalid');return null;}
  return transport.resolveRoute(route);
}

const FEEDBACK_CATEGORIES = new Set(['confusing', 'incorrect_result', 'missing_capability', 'performance', 'other']);
const FEEDBACK_STATUSES = new Set(['new','investigating','resolved','duplicate','dismissed']);
const FEEDBACK_SEVERITIES = new Set(['untriaged','low','medium','high']);
const FEEDBACK_IMAGE_LIMIT = 3 * 1024 * 1024;
function betaFeedbackRecord({category, description = '', productArea = 'review_workspace', route = '/workspace', layout = 'connected', side = 'diff', hasReview = false, feedbackId, createdAt, screenshot = null, status = 'new', severity = 'untriaged', duplicateOf = null, resolutionCommit = null} = {}) {
  if (!FEEDBACK_CATEGORIES.has(category)) throw new Error('Invalid feedback category');
  const cleanDescription = String(description).trim();
  if (cleanDescription.length > 4000) throw new Error('Feedback description is too long');
  if (!WORKSPACE_LAYOUTS.has(layout) || !WORKSPACE_SIDES.has(side)) throw new Error('Invalid feedback UI state');
  const cleanRoute = String(route).split(/[?#]/, 1)[0];
  if (cleanRoute !== '/workspace') throw new Error('Invalid feedback route');
  if (productArea !== 'review_workspace') throw new Error('Invalid product area');
  if (!FEEDBACK_STATUSES.has(status) || !FEEDBACK_SEVERITIES.has(severity)) throw new Error('Invalid feedback disposition');
  if (duplicateOf !== null && !/^[a-zA-Z0-9_-]{1,128}$/.test(duplicateOf)) throw new Error('Invalid duplicate report ID');
  if (resolutionCommit !== null && !/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(resolutionCommit)) throw new Error('Resolution needs a full commit SHA');
  if (status === 'duplicate' && (!duplicateOf || duplicateOf === feedbackId)) throw new Error('Choose a different duplicate report ID');
  if (status === 'resolved' && !resolutionCommit) throw new Error('Resolved feedback needs a resolution commit');
  let attachment = null;
  if (screenshot) {
    if (screenshot.mime_type !== 'image/png' || !Number.isInteger(screenshot.width) || !Number.isInteger(screenshot.height) || screenshot.width < 1 || screenshot.height < 1 || screenshot.width * screenshot.height > 12000000 || !Number.isInteger(screenshot.bytes) || screenshot.bytes < 1 || screenshot.bytes > FEEDBACK_IMAGE_LIMIT || !/^data:image\/png;base64,iVBORw0KGgo[A-Za-z0-9+/]*={0,2}$/.test(screenshot.data_url) || screenshot.data_url.length > Math.ceil(FEEDBACK_IMAGE_LIMIT / 3) * 4 + 22) throw new Error('Invalid or oversized screenshot');
    attachment = {mime_type:'image/png',width:screenshot.width,height:screenshot.height,bytes:screenshot.bytes,data_url:screenshot.data_url};
  }
  return {
    schema:'codeshouldbehave-beta-feedback-v2', feedback_id:String(feedbackId || ''), created_at:String(createdAt || ''),
    category, description:cleanDescription,
    ui:{product_area:String(productArea).slice(0,64),route:cleanRoute,layout,comparison:side,review_selected:Boolean(hasReview)},
    privacy:{automatic_metadata_includes_credentials:false,automatic_metadata_includes_session_token:false,automatic_metadata_includes_source:false,automatic_metadata_includes_repository_contents:false,screenshot_included:Boolean(attachment),user_content_requires_review:true},
    screenshot:attachment,
    transport:'local_download', status, severity, duplicate_of:duplicateOf, resolution_commit:resolutionCommit,
  };
}

function importBetaFeedback(text) {
  if (typeof text !== 'string' || text.length > 4500000) throw new Error('Feedback import exceeds the size limit');
  const input = JSON.parse(text);
  if (!input || !['codeshouldbehave-beta-feedback-v1','codeshouldbehave-beta-feedback-v2'].includes(input.schema) || !/^[a-zA-Z0-9_-]{1,128}$/.test(input.feedback_id) || typeof input.created_at !== 'string' || !Number.isFinite(Date.parse(input.created_at)) || input.created_at.length > 40 || !input.ui || input.ui.route !== '/workspace' || input.ui.product_area !== 'review_workspace' || input.transport !== 'local_download') throw new Error('Invalid feedback record');
  // Reconstruct only known fields; untrusted extra fields never survive import.
  return betaFeedbackRecord({category:input.category,description:input.description,productArea:input.ui.product_area,route:input.ui.route,layout:input.ui.layout,side:input.ui.comparison,hasReview:input.ui.review_selected,feedbackId:input.feedback_id,createdAt:input.created_at,screenshot:input.screenshot || null,status:input.status,severity:input.severity,duplicateOf:input.duplicate_of,resolutionCommit:input.resolution_commit});
}

class BetaFeedbackDraft {
  constructor() { this.reset(); }
  reset() { this.record = null; this.screenshot = null; this.consent = false; this.previewed = null; this.processing = false; this.imageGeneration = (this.imageGeneration || 0) + 1; }
  change() { this.previewed = null; }
  setScreenshot(screenshot) { this.screenshot = screenshot; this.consent = false; this.change(); }
  setConsent(value) { this.consent = Boolean(value); this.change(); }
  preview(fields) {
    if (this.processing) throw new Error('Wait for the selected file to finish loading');
    if (this.screenshot && !this.consent) throw new Error('Review the screenshot and confirm its inclusion first');
    this.previewed = betaFeedbackRecord({...fields,screenshot:this.screenshot});
    return this.previewed;
  }
  exportRecord() { if (!this.previewed) throw new Error('Refresh and inspect the preview before export'); return this.previewed; }
}

// Authority loss clears inactive HLD panes as well as the currently selected view.
function clearHldEvidence(root) {
  const get=id=>root.getElementById(id);
  for(const id of ['hld-diagrams','hld-element-list','hld-inspector','hld-coverage','hld-corrections-list','hld-correction-element','hld-related-elements','hld-flow-source','hld-flow-target','hld-document-evidence','hld-correspondence-sources','hld-correspondence-targets'])get(id).replaceChildren();
  for(const id of ['hld-identity','hld-full-identity','hld-drift-status','hld-correspondence-source-identity','hld-correspondence-target-identity'])get(id).textContent='';
  for(const id of ['hld-responsibility','hld-boundary-paths'])get(id).value='';
  for(const id of ['hld-mermaid-source','hld-confirmation-preview']){get(id).hidden=true;get(id).firstElementChild.textContent='';}
  for(const id of ['hld-source','hld-export','hld-confirm','hld-accepted'])get(id).disabled=true;
  for(const id of ['hld-known-gaps','hld-activate-default'])get(id).checked=false;
  get('hld-confirmation-status').textContent='Preview is required before confirmation.';
  get('hld-summary').textContent='No authorized HLD evidence loaded.';
  get('hld-acceptance').hidden=true;
  delete get('hld-correction-element').dataset.designId;
}

function clearLldEvidence(root) {
  const get=id=>root.getElementById(id);
  for(const id of ['lld-diagrams','lld-entry-list','lld-inspector','lld-coverage','lld-changes','lld-history'])get(id).replaceChildren();
  for(const id of ['lld-component-name','lld-parent-identity','lld-full-identity','lld-detail-breadcrumb','lld-catalog-status','lld-confirmation-status'])get(id).textContent='';
  for(const id of ['lld-mermaid-source','lld-confirmation-preview']){get(id).hidden=true;get(id).firstElementChild.textContent='';}
  for(const id of ['lld-mermaid','lld-export','lld-confirm'])get(id).disabled=true;
  for(const id of ['lld-history','lld-acceptance','lld-more','lld-load-history'])get(id).hidden=true;
  for(const id of ['lld-known-gaps','lld-activate-default'])get(id).checked=false;
  for(const id of ['lld-invariant','lld-filter'])get(id).value='';
}

function clearReviewEvidence(root) {
  const get=id=>root.getElementById(id);clearHldEvidence(root);clearLldEvidence(root);
  for(const id of ['graph','architecture-diagrams','baseline-revision','proposal-revision','scope-contexts','edge-filter'])get(id).replaceChildren();
  for(const id of ['graph-selection','graph-overview','selected-file-name','selected-file-delta','revisions','architecture-impact-summary','architecture-status','scope-preview'])get(id).textContent='';
  for(const id of ['graph-search','file-search','scope-primary-include','scope-primary-exclude','review-question','handoff-reviewer'])get(id).value='';
  get('architecture-mermaid-source').hidden=true;get('architecture-mermaid-source').firstElementChild.textContent='';
  get('architecture-source').disabled=true;get('architecture-source').textContent='Mermaid source';get('save-scope').disabled=true;
  get('selected-file-actions').hidden=true;delete get('toggle-file-reviewed').dataset.path;
  get('baseline-source-label').textContent='Baseline';get('proposal-source-label').textContent='Proposal';
  get('lld-summary').textContent='No authorized LLD evidence loaded.';
}

// Presentation reuse only: every analysis/source read still authorizes freshly.
// Keeping unchanged DOM also preserves the reviewer's keyboard focus and folds.
class LldCatalogView {
  constructor({list,status,more,node,onOpen}) {Object.assign(this,{list,status,more,node,onOpen});this.clear();}
  clear() {this.payload=undefined;this.query=undefined;this.selected=null;this.buttons=new Map();this.list.replaceChildren();}
  render(payload,{query='',selection=null,loading=false}={}) {
    this.more.hidden=!payload?.next_cursor;this.more.disabled=loading;
    const needle=query.toLowerCase();
    if(this.payload===payload && this.query===needle){this.select(selection);return;}
    this.payload=payload;this.query=needle;this.selected=null;this.buttons.clear();this.list.replaceChildren();
    const rows=lldCatalogRows(payload),visible=rows.filter(row=>!needle||[row.entry.name,row.entry.path,row.entry.module,row.entry.container,row.entry.logical_key].join(' ').toLowerCase().includes(needle));
    this.status.textContent=payload?`${visible.length} of ${rows.length} loaded declarations${payload.next_cursor?' · more available':''}`:'No catalog loaded.';
    const paths=new Map();for(const row of visible){if(!paths.has(row.entry.path))paths.set(row.entry.path,[]);paths.get(row.entry.path).push(row);}
    const node=this.node;
    for(const [path,entries] of paths){
      const group=node('details','lld-module-group');group.open=Boolean(needle)||entries.some(row=>row.id===selection)||paths.size<4;
      const summary=node('summary');summary.append(node('span','',path),node('span','badge',String(entries.length)));group.append(summary);
      for(const row of entries){
        const button=node('button','lld-entry');button.type='button';button.dataset.entryId=row.id;button.setAttribute('aria-current','false');
        const label=node('span','');label.append(node('strong','',row.entry.name),node('span','mono',`${row.entry.kind} · ${row.entry.container || row.entry.module} · ${row.entry.visibility}`));button.append(label,node('span','badge',row.status || row.entry.detail_state));
        if(row.entry.parent?.state!=='matched')button.append(node('span','lld-mapping-warning',`${row.entry.parent?.state || 'unmapped'} HLD ownership`));
        button.addEventListener('click',()=>this.onOpen(row.id,row.entry));group.append(button);this.buttons.set(row.id,{button,group});
      }
      this.list.append(group);
    }
    if(payload && !rows.length)this.list.append(node('p','status','No declarations were observed in this authorized component. Inventory coverage determines whether this is complete.'));
    this.select(selection);
  }
  select(selection) {
    if(this.selected===selection)return;
    this.buttons.get(this.selected)?.button.setAttribute('aria-current','false');
    const current=this.buttons.get(selection);if(current){current.button.setAttribute('aria-current','true');current.group.open=true;}
    this.selected=selection;
  }
}

class LldSourcePairView {
  constructor({node}) {this.node=node;this.host=null;this.comparison=undefined;this.presence='';}
  clear() {this.host?.replaceChildren();this.host=null;this.comparison=undefined;this.presence='';}
  render(host,comparison,payload) {
    const presence=`${Boolean(payload.baseline)}|${Boolean(payload.proposal)}`;
    if(this.host===host && this.comparison===comparison && this.presence===presence)return;
    this.host=host;this.comparison=comparison;this.presence=presence;host.replaceChildren();if(!comparison)return;
    const node=this.node,rows=diffLines(comparison.baseline?.text || '',comparison.proposal?.text || '');
    for(const [side,code] of [['Baseline',comparison.baseline],['Proposal',comparison.proposal]]){
      const section=node('section');section.append(node('strong','',`${side}${code?.truncated?' · bounded excerpt':''}`));
      if(code){section.append(node('p','mono',`${code.path}:${code.start_line}-${code.end_line} · ${code.commit ? String(code.commit).slice(0,8) : 'unavailable'}`));const pre=node('pre','lld-exact-code');let line=code.start_line;for(const row of rows.filter(row=>side==='Baseline'?row.kind!=='added':row.kind!=='removed')){const kind=row.kind==='modified'?(side==='Baseline'?'removed':'added'):row.kind,text=side==='Baseline'?row.before:row.after;pre.append(node('span',`diff-line ${kind}`,`${line++} ${kind==='added'?'+':kind==='removed'?'-':' '} ${text}\n`));}section.append(pre);}
      else section.append(node('p','status',payload[side.toLowerCase()]?'No retained source is available for this declaration.':side==='Baseline'?'Absent in baseline.':'Absent in proposal.'));
      host.append(section);
    }
    for(const error of comparison.errors)host.append(node('p','status',`Source unavailable: ${error}`));
  }
}

// Decode only known local authority/frontier codes. Provider text never enters UI errors.
async function workspaceHttpError(response) {
  const error = new Error(response.status === 401 ? 'Session expired or invalid' : `Request failed (${response.status})`);
  error.status = response.status;
  const allowed = new Set(['lld_unavailable_in_current_context','repository_unavailable','lld_not_generated_regenerate_review','lld_hld_parent_unavailable','exact_hld_source_unavailable','exact_lld_source_unavailable']);
  if(response.status !== 404 || response.headers?.get('content-type')?.split(';')[0].trim().toLowerCase() !== 'application/json' || !response.body?.getReader)return error;
  const reader=response.body.getReader(),decoder=new TextDecoder();let bytes=0,text='';
  try {
    while(true){const {done,value}=await reader.read();if(done)break;bytes+=value.byteLength;if(bytes>4096){await reader.cancel();return error;}text+=decoder.decode(value,{stream:true});}
    text+=decoder.decode();const payload=JSON.parse(text);
    if(payload && !Array.isArray(payload) && allowed.has(payload.error)){error.message=payload.error;error.code=payload.error;}
  } catch (failure) { if(failure.name==='AbortError')throw failure; /* Invalid or non-JSON bodies keep the fixed safe message. */ }
  finally {reader.releaseLock();}
  return error;
}

if (typeof module !== 'undefined') module.exports = {ReviewWorkspace, workspaceHttpError, clearHldEvidence, clearLldEvidence, clearReviewEvidence, LldCatalogView, LldSourcePairView, graphItems, filterGraphItems, clusterEdges, mergeGraphs, diffLines, architectureMeaning, architectureAreas, architectureMermaid, hldOutcome, hldExportAllowed, hldComparison, hldDiagramElements, hldMermaid, hldEvidenceFor, hldCorrection, validateLldResponse, lldOutcome, lldCatalogRows, lldMermaid, lldEvidence, LLD_INITIAL_SCALE, observationAge, filterChangedFiles, diagramScale, parseScopePaths, summarizeReviewState, classifyFinding, groupFindings, encodeWorkspaceRoute, decodeWorkspaceRoute, resolvePublicWorkspaceRoute, betaFeedbackRecord, importBetaFeedback, BetaFeedbackDraft};
if (typeof window !== 'undefined') Object.assign(window, {ReviewWorkspace, encodeWorkspaceRoute, decodeWorkspaceRoute});

if (typeof document !== 'undefined' && document.getElementById('session-form')) {
  const $ = id => document.getElementById(id);
  const publicMeta=document.querySelector('meta[name="csb-public-static"]');
  const publicStatic=Boolean(publicMeta);
  let publicTransport=null,publicBootstrapError=null;
  if(publicStatic){try{if(!window.CodeShouldBehavePublic)throw new Error('Public viewer transport unavailable');const indexURL=new URL(publicMeta.content,window.location.href);if(indexURL.origin!==window.location.origin)throw new Error('Public index must share this project origin');publicTransport=window.CodeShouldBehavePublic.createPublicStaticTransport({indexURL});}catch(error){publicBootstrapError=error;}}
  let token = document.querySelector('meta[name="csb-local-session"]')?.content || '';
  $('onboarding-link').hidden = publicStatic || !token;
  let inboxCursor = null;
  let graphCursor = null;
  let activePr = null;
  let pinned = false;
  let direction = 'outgoing';
  let renderedGraph = null;
  let graphWindow = 40;
  let expandedCluster = null;
  let graphQuery = '';
  let edgeKindFilter = '';
  let layout = 'connected';
  let codeDiffMode = 'split';
  let architectureMode = 'combined';
  let designLevel = 'structural';
  let hldMode = 'combined';
  let hldSelection = null;
  let renderedHldReview = null;
  let lastHldKey = '';
  let hldSourceGeneration = 0;
  let hldCorrections = [];
  let hldCorrectionDesignId = null;
  let lldParentHld = null, lldComponent = null, lldDetail = null, lldElement = null;
  let lldNode = null, lldPinned = false, lastLldKey = '', lldSourceTimer = null, lldSourceRequest = 0, lldQuery = '';
  let lastLldInspectorKey = '', activeLldEvidence = null, lldInvariantId = null;
  let lldBaselineEntry = null, lldCorrespondence = 'rename', lldCorrespondenceId = null;
  let lldActiveSourcePromise = null, lldActiveSourceKey = null;
  let mermaidLoading = null;
  let architectureGeneration = 0;
  let lastArchitectureKey = '';
  const diagramObservers = [];
  let applyingRoute = false;
  let pendingRoute = decodeWorkspaceRoute(window.location.hash);
  const coverageQueue = [];
  let coverageActive = 0;
  let previewedScope = null;
  const feedbackDraft = new BetaFeedbackDraft();

  const request = async (path, options = {}) => {
    if(publicStatic){if(publicBootstrapError)throw publicBootstrapError;return publicTransport.request(path,options);}
    if (!token) throw new Error('Connect with an authorized session');
    const response = await fetch(path, {
      ...options,
      headers: {...options.headers, Authorization: `Bearer ${token}`, Accept: 'application/json'},
      cache: 'no-store',
    });
    if (!response.ok) {
      throw await workspaceHttpError(response);
    }
    return response.json();
  };

  const workspace = new ReviewWorkspace({request, onState: renderWorkspace});
  const lldCatalogView=new LldCatalogView({list:$('lld-entry-list'),status:$('lld-catalog-status'),more:$('lld-more'),node,onOpen:openLldEntry});
  const lldSourcePairView=new LldSourcePairView({node});
  const coverageObserver = typeof IntersectionObserver === 'undefined' ? null : new IntersectionObserver(entries => {
    entries.filter(entry => entry.isIntersecting).forEach(entry => {
      coverageObserver.unobserve(entry.target);
      coverageQueue.push(entry.target);
    });
    drainCoverageQueue();
  }, {root: $('pr-list'), rootMargin: '80px'});

  function short(value) { return value ? String(value).slice(0, 8) : 'unavailable'; }
  function node(tag, className, text) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (text !== undefined) element.textContent = text;
    return element;
  }

  function buildFeedbackRecord() {
    if (!feedbackDraft.record) feedbackDraft.record = betaFeedbackRecord({category:'confusing',feedbackId:typeof crypto?.randomUUID === 'function' ? crypto.randomUUID() : `local-${Date.now()}`,createdAt:new Date().toISOString()});
    const previous = feedbackDraft.record;
    return {...previous,category:$('feedback-category').value,description:$('feedback-description').value,productArea:previous.ui.product_area,route:previous.ui.route,layout:previous.ui.layout,side:previous.ui.comparison,hasReview:previous.ui.review_selected,feedbackId:previous.feedback_id,createdAt:previous.created_at,status:$('feedback-triage-status').value,severity:$('feedback-triage-severity').value,duplicateOf:$('feedback-duplicate').value||null,resolutionCommit:$('feedback-resolution').value||null};
  }

  function previewFeedback() {
    try {
      const record = feedbackDraft.preview(buildFeedbackRecord());
      const visible = {...record,screenshot:record.screenshot ? {...record.screenshot,data_url:'PNG content shown in the image preview; included in the exported JSON.'} : null};
      $('feedback-preview').firstElementChild.textContent = JSON.stringify(visible,null,2);
      $('feedback-status').textContent = 'Preview ready. Inspect the metadata, description and any image before local export.';
      $('export-feedback').disabled = false;
      $('copy-feedback').disabled = false;
    } catch (error) { $('feedback-status').textContent = error.message; $('export-feedback').disabled = true; $('copy-feedback').disabled = true; }
  }

  function invalidateFeedback() {
    feedbackDraft.change();$('export-feedback').disabled = true;
    $('copy-feedback').disabled = true;$('feedback-preview').firstElementChild.textContent = 'Preview out of date. Refresh before export.';
    $('feedback-status').textContent = 'Report changed. Refresh and inspect the preview before export.';
  }

  function clearFeedback() {
    feedbackDraft.reset();$('feedback-form').reset();$('feedback-image-content').removeAttribute('src');$('feedback-image-preview').hidden = true;$('feedback-preview').firstElementChild.textContent = '';$('export-feedback').disabled = true;$('copy-feedback').disabled = true;
  }

  async function prepareFeedbackImage(file) {
    if (!file || file.size > FEEDBACK_IMAGE_LIMIT || !['image/png','image/jpeg','image/webp'].includes(file.type)) throw new Error('Choose a PNG, JPEG or WebP up to 3 MiB');
    const header=new Uint8Array(await file.slice(0,12).arrayBuffer());
    const png=header[0]===137 && header[1]===80 && header[2]===78 && header[3]===71 && header[4]===13 && header[5]===10 && header[6]===26 && header[7]===10;
    const jpeg=header[0]===255 && header[1]===216 && header[2]===255;
    const webp=String.fromCharCode(...header.slice(0,4))==='RIFF' && String.fromCharCode(...header.slice(8,12))==='WEBP';
    if(!({ 'image/png':png,'image/jpeg':jpeg,'image/webp':webp }[file.type]))throw new Error('Image contents do not match an allowed raster format');
    const bitmap = await createImageBitmap(file);
    try {
      if (bitmap.width * bitmap.height > 12000000) throw new Error('Screenshot exceeds 12 million pixels');
      const canvas = document.createElement('canvas');canvas.width = bitmap.width;canvas.height = bitmap.height;canvas.getContext('2d').drawImage(bitmap,0,0);
      const dataUrl = canvas.toDataURL('image/png');
      const bytes = Math.floor((dataUrl.length - 22) * 3 / 4) - (dataUrl.endsWith('==') ? 2 : dataUrl.endsWith('=') ? 1 : 0);
      if (bytes > FEEDBACK_IMAGE_LIMIT) throw new Error('Re-encoded screenshot exceeds 3 MiB; choose a smaller image');
      return {mime_type:'image/png',width:bitmap.width,height:bitmap.height,bytes,data_url:dataUrl};
    } finally { bitmap.close(); }
  }

  async function loadInbox({append = false} = {}) {
    $('inbox-status').textContent = publicStatic?'Loading published comparisons…':'Loading authorized pull requests…';
    try {
      const query = new URLSearchParams({limit: '100'});
      if (append && inboxCursor) query.set('cursor', inboxCursor);
      const page = await request(`/v1/prs?${query}`);
      if (!append) $('pr-list').replaceChildren();
      page.items.forEach(addPrCard);
      inboxCursor = page.next_cursor;
      $('more-prs').hidden = !inboxCursor;
      $('inbox-status').textContent = page.items.length || append ? '' : publicStatic?'No published comparisons.':'No authorized pull requests.';
      $('connection').textContent = publicStatic?'Published · read-only':'Connected';
      if(!append && !page.next_cursor && workspace.state.reviewId && !page.items.some(item=>item.review_id===workspace.state.reviewId))workspace.clearReview('The selected review is no longer present in the authorized inbox.');
      if (!append && (pendingRoute || publicStatic&&/[#&](?:review|bundle)=/.test(window.location.hash))) {
        if (pendingRoute && /^pr-[1-9][0-9]*$/.test(pendingRoute.reviewId)) {
          const match=page.items.find(pr=>pr.provider_pr_id===`PR-${pendingRoute.reviewId.slice(3)}`);
          if(match) pendingRoute={...pendingRoute,reviewId:match.review_id};
        }
        await restoreRoute(pendingRoute);
      }
    } catch (error) {
      if(error.invalidateReview || [401,403].includes(error.status))workspace.clearReview(error.message);
      $('inbox-status').textContent = error.message;
      $('connection').textContent = 'Disconnected';
    }
  }

  function addPrCard(pr) {
    const card = node('button', 'pr-card');
    card.type = 'button'; card.role = 'option'; card.dataset.reviewId = pr.review_id; card.review = pr;
    card.setAttribute('aria-selected', 'false');
    const name = node('div', 'pr-name');
    const repository = pr.repository ? `${pr.repository.owner}/${pr.repository.name}` : String(pr.repository_id);
    const identity = node('span', 'pr-identity');
    identity.append(node('strong', 'pr-number', pr.provider_pr_id), node('span', 'pr-repository', repository));
    name.append(identity, node('span', 'badge', pr.state));
    const meta = node('div', 'pr-meta');
    const coverage = node('span', 'coverage-value', 'coverage pending');
    coverage.dataset.state = 'loading'; coverage.dataset.reviewId = pr.review_id;
    meta.append(node('span', 'pr-age', observationAge(pr.observed_at_ms)), coverage);
    const commits = node('div', 'pr-commits', `${short(pr.base_commit)} → ${short(pr.head_commit)}`);
    card.append(name, meta, commits);
    card.addEventListener('click', () => openPr(pr, card));
    $('pr-list').append(card);
    if (coverageObserver && pr.review_id) coverageObserver.observe(coverage);
    else if (pr.review_id) { coverageQueue.push(coverage); drainCoverageQueue(); }
  }

  function drainCoverageQueue() {
    while (coverageActive < 4 && coverageQueue.length) {
      const target = coverageQueue.shift(); coverageActive++;
      request(`/v1/reviews/${encodeURIComponent(target.dataset.reviewId)}/progress`)
        .then(progress => { const values = progress.coverage || []; target.title = values.map(item => `${item[0]} ${item[1]}`).join(' · '); const present = values.filter(item => item[1] > 0); target.textContent = present.length ? present.map(item => `${item[1]} ${item[0]}`).join(' · ') : (progress.job_state || 'no files indexed'); target.dataset.state = 'ready'; })
        .catch(() => { target.textContent = 'coverage unavailable'; target.dataset.state = 'error'; })
        .finally(() => { coverageActive--; drainCoverageQueue(); });
    }
  }

  async function openPr(pr, card, {remember = true, route = null} = {}) {
    activePr = pr; pinned = false;lldCatalogView.clear();lldSourcePairView.clear();
    setDesignLevel('structural',{remember:false,load:false});
    hldSelection = null; lastHldKey = ''; renderedHldReview = null; ++hldSourceGeneration;
    lldParentHld=null;lldComponent=null;lldDetail=null;lldElement=null;lldNode=null;lldPinned=false;lldBaselineEntry=null;lldCorrespondenceId=null;lastLldKey='';lastLldInspectorKey='';lldQuery='';$('lld-filter').value='';
    hldCorrections=[];hldCorrectionDesignId=null;$('hld-corrections-list').replaceChildren();$('hld-known-gaps').checked=false;$('hld-activate-default').checked=false;
    $('file-search').value = '';
    document.querySelectorAll('.pr-card').forEach(item => item.setAttribute('aria-selected', String(item === card)));
    $('empty').hidden = true; $('review-content').hidden = false;
    $('review-title').textContent = `${pr.repository ? `${pr.repository.owner}/${pr.repository.name}` : pr.repository_id} · ${pr.provider_pr_id}`;
    $('breadcrumbs').textContent = 'Inbox / architecture';
    $('revisions').textContent = `${short(pr.base_commit)} baseline → ${short(pr.head_commit)} proposal`;
    clearSourceInspector();
    $('scope-primary-include').value = '.'; $('scope-primary-exclude').value = '';
    $('scope-contexts').replaceChildren(); invalidateScopePreview();
    setLayout('connected', {remember: false});
    await Promise.all([workspace.selectReview(pr.review_id), loadRevisions(pr)]);
    if (route) await applyRoute(route);
    else if (remember) rememberRoute();
  }

  function rememberRoute({replace = false} = {}) {
    if (applyingRoute || !workspace.state.reviewId) return;
    const identity=publicStatic?publicTransport.routeFor(workspace.state.reviewId):{reviewId:workspace.state.reviewId};
    const hash = encodeWorkspaceRoute({...identity, layout, side:$('side').value, selection:workspace.state.selection,designLevel,designMode:hldMode,designElement:hldSelection,parentHld:lldParentHld,component:lldComponent,detail:lldDetail,lldElement,lldNode,lldPinned,lldBaselineEntry,lldCorrespondence});
    if (window.location.hash === hash) return;
    window.history[replace ? 'replaceState' : 'pushState'](null, '', hash);
  }

  async function applyRoute(route) {
    if (!route || route.reviewId !== workspace.state.reviewId) return;
    applyingRoute = true;
    try {
      $('side').value = route.side;
      setLayout(route.layout, {remember:false});
      hldMode = route.designMode || 'combined'; hldSelection = route.designElement || route.component || null;
      lldParentHld=route.parentHld||null;lldComponent=route.component||null;lldDetail=route.detail||null;lldElement=route.lldElement||null;
      lldNode=route.lldNode||null;lldPinned=Boolean(route.lldPinned);
      lldBaselineEntry=route.lldBaselineEntry||null;lldCorrespondence=route.lldCorrespondence || 'rename';
      setDesignLevel(route.designLevel || 'structural',{remember:false,load:route.designLevel!=='lld',inferParent:false});
      if(route.designLevel==='lld'){await loadSelectedLldCatalog();if(lldElement)await workspace.loadLldDetail(lldElement,{baselineEntry:lldBaselineEntry,correspondenceKind:lldCorrespondence});}
      if (route.selection?.kind === 'component') await workspace.expandComponent({path:route.selection.path, side:route.side});
      if (route.selection?.kind === 'path') await workspace.selectPath({path:route.selection.path, side:route.side});
      if (route.selection?.kind === 'symbol') await workspace.selectSymbol({symbol:route.selection.symbol, path:route.selection.path, side:route.side, direction});
    } finally { applyingRoute = false; }
  }

  async function restoreRoute(route = decodeWorkspaceRoute(window.location.hash)) {
    if(publicStatic){try{route=resolvePublicWorkspaceRoute(publicTransport,window.location.hash);}catch(error){workspace.clearReview(error.message);$('inbox-status').textContent=error.message;return;}}
    pendingRoute = route;
    if (!route) return;
    const card = Array.from(document.querySelectorAll('.pr-card')).find(item => item.dataset.reviewId === route.reviewId);
    if (!card) { $('inbox-status').textContent = 'The linked review is unavailable in the authorized inbox.'; return; }
    if (workspace.state.reviewId !== route.reviewId || (publicStatic&&!workspace.state.details&&workspace.state.error)) await openPr(card.review, card, {remember:false, route});
    else await applyRoute(route);
    pendingRoute = null;
  }

  function addScopeContext() {
    const row = node('div', 'scope-context');
    const fields = [
      ['name','Context name'],['provider','Provider'],['owner','Owner'],['repository','Repository'],
      ['commit','Exact commit'],['include','Includes'],['exclude','Excludes'],['security_domain','Security domain'],
    ];
    fields.forEach(([field, label]) => {
      const wrapper = node('label', '', label); const input = node('input'); input.dataset.field = field;
      if (field === 'provider') input.value = activePr?.repository?.provider || '';
      if (field === 'security_domain') input.value = activePr?.repository?.security_domain || '';
      wrapper.append(input); row.append(wrapper);
    });
    const remove = node('button','quiet remove-context','Remove'); remove.type = 'button';
    remove.addEventListener('click', () => { row.remove(); invalidateScopePreview(); }); row.append(remove);
    row.addEventListener('input', invalidateScopePreview); $('scope-contexts').append(row); invalidateScopePreview();
  }

  function proposedScope() {
    if (!activePr?.repository) throw new Error('Select a pull request first');
    const contexts = Array.from(document.querySelectorAll('.scope-context')).map(row => {
      const value = field => row.querySelector(`[data-field="${field}"]`).value.trim();
      if (!value('name') || !value('repository') || !value('commit')) throw new Error('Every context needs a name, repository, and exact commit');
      return {name:value('name'),repository:{security_domain:value('security_domain'),provider:value('provider'),owner:value('owner'),name:value('repository')},commit:value('commit'),include:parseScopePaths(value('include'),true),exclude:parseScopePaths(value('exclude'))};
    });
    return {primary_repository:activePr.repository,primary_include:parseScopePaths($('scope-primary-include').value,true),primary_exclude:parseScopePaths($('scope-primary-exclude').value),contexts,contracts:[],export_allowed:true,model_destinations:[]};
  }

  function invalidateScopePreview() { previewedScope = null; $('save-scope').disabled = true; if ($('scope-preview')) $('scope-preview').textContent = 'Preview required before saving.'; }

  async function previewScope() {
    const generation=workspace.generation,reviewId=workspace.state.reviewId;
    try {
      const scope = proposedScope(); const serialized = JSON.stringify(scope);
      const delta = await workspace.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/scope/preview`,generation,{method:'POST',headers:{'Content-Type':'application/json'},body:serialized});
      if(generation!==workspace.generation || reviewId!==workspace.state.reviewId)return;
      previewedScope = serialized; $('save-scope').disabled = !delta.changes_identity;
      const changes = [...delta.added_contexts.map(name => `add ${name}`),...delta.removed_contexts.map(name => `remove ${name}`),...delta.changed_contexts.map(name => `change ${name}`)];
      if (delta.primary_paths_changed) changes.push('change primary paths');
      const files = [...delta.added_files.map(path => `+ ${path}`), ...delta.removed_files.map(path => `− ${path}`)];
      const fileSummary = files.length ? ` Files: ${files.join(' · ')}${delta.file_preview_truncated ? ' · more omitted' : ''}` : '';
      $('scope-preview').textContent = changes.length ? `New review identity: ${changes.join(' · ')}.${fileSummary}` : 'No scope identity change.';
    } catch (error) { if(generation===workspace.generation && error.name!=='AbortError'){invalidateScopePreview(); $('scope-preview').textContent = error.message;} }
  }

  async function saveScope() {
    const generation=workspace.generation,reviewId=workspace.state.reviewId;
    try {
      const scope = proposedScope(); const serialized = JSON.stringify(scope);
      if (serialized !== previewedScope) throw new Error('Scope changed after preview; preview it again');
      const receipt = await workspace.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/scope`,generation,{method:'PUT',headers:{'Content-Type':'application/json'},body:JSON.stringify({provider_pr_id:activePr.provider_pr_id,generation:activePr.generation,priority:activePr.priority || 0,scope})});
      if(generation!==workspace.generation || reviewId!==workspace.state.reviewId)return;
      activePr.review_id = receipt.review_id; pinned = false; clearSourceInspector('Source cleared because the immutable scope changed.');
      await workspace.selectReview(receipt.review_id); rememberRoute(); previewedScope = null; $('save-scope').disabled = true; $('scope-preview').textContent = 'New immutable scope saved; fresh analysis queued.';
    } catch (error) { if(generation===workspace.generation && error.name!=='AbortError')$('scope-preview').textContent = error.message; }
  }

  async function loadRevisions(pr) {
    const generation=workspace.generation,reviewId=workspace.state.reviewId;
    try {
      const query = new URLSearchParams({limit: '100', repository_id: String(pr.repository_id)});
      const page = await workspace.fetch(`/v1/prs/${encodeURIComponent(pr.provider_pr_id)}/revisions?${query}`,generation);
      if (activePr !== pr || generation!==workspace.generation || reviewId!==workspace.state.reviewId) return;
      const commits = new Map();
      page.items.forEach(item => {
        commits.set(item.base_commit, `${short(item.base_commit)} · base generation ${item.generation}`);
        commits.set(item.head_commit, `${short(item.head_commit)} · head generation ${item.generation}`);
      });
      commits.set(pr.base_commit, `${short(pr.base_commit)} · selected base`);
      commits.set(pr.head_commit, `${short(pr.head_commit)} · latest head`);
      for (const select of [$('baseline-revision'), $('proposal-revision')]) {
        select.replaceChildren();
        commits.forEach((label, commit) => { const option = node('option', '', label); option.value = commit; select.append(option); });
      }
      $('baseline-revision').value = pr.base_commit;
      $('proposal-revision').value = pr.head_commit;
    } catch (error) {
      if(activePr===pr && generation===workspace.generation && error.name!=='AbortError')$('revisions').textContent = `Revision history unavailable: ${error.message}`;
    }
  }

  async function compareRevisions() {
    if (!activePr?.repository) return;
    const generation=workspace.generation,reviewId=workspace.state.reviewId;
    const button = $('compare-revisions'); button.disabled = true;
    $('revisions').textContent = 'Creating exact comparison…';
    try {
      const receipt = await workspace.fetch('/v1/reviews',generation, {
        method: 'POST',
        headers: {'Content-Type': 'application/json'},
        body: JSON.stringify({
          repository: activePr.repository,
          provider_pr_id: activePr.provider_pr_id,
          generation: activePr.generation,
          selected_baseline: $('baseline-revision').value,
          proposal: $('proposal-revision').value,
          target_branch: activePr.target_branch,
          state: activePr.state === 'draft' ? 'draft' : 'open',
          priority: activePr.priority || 0,
        }),
      });
      if(generation!==workspace.generation || reviewId!==workspace.state.reviewId)return;
      activePr.review_id = receipt.review_id;
      $('revisions').textContent = `${short($('baseline-revision').value)} baseline → ${short($('proposal-revision').value)} proposal`;
      await workspace.selectReview(receipt.review_id);
      rememberRoute();
    } catch (error) {
      if(generation===workspace.generation && error.name!=='AbortError')$('revisions').textContent = error.message;
    } finally { button.disabled = false; }
  }

  function renderWorkspace(state) {
    if (!state.reviewId) return;
    if(publicStatic&&$('public-export-report'))$('public-export-report').disabled=!(state.details?.capabilities||[]).includes('export');
    $('hld-boundary-state').textContent=state.hld?.accepted?'Accepted as-built baseline':state.hld?(publicStatic?'Published static evidence':'Generated; explicit acceptance required'):state.hldError?'Unavailable':'Not loaded';
    if(publicStatic&&state.details){const scope=state.details.scope_manifest;$('scope-primary-include').value=(scope?.include||[]).join(', ');$('scope-primary-exclude').value=(scope?.exclude||[]).join(', ');$('scope-preview').textContent=`Published scope ${state.details.scope_version}. ${state.details.public_coverage?.complete?'Declared analysis coverage complete; runtime behavior remains unverified.':(state.details.public_coverage?.gaps||[]).join(' · ')} Request a new analysis to change scope or commits.`;}
    $('lld-boundary-state').textContent=state.lld?lldOutcome(state.lld.delta):state.lldError?'Unavailable':'Not loaded';
    if(!state.details && state.error){
      pinned=false;clearHldView();clearLldView();clearSourceInspector('Current access does not authorize this review.');clearReviewEvidence(document);
      hldSelection=null;renderedGraph=null;graphCursor=null;lastArchitectureKey='';graphQuery='';edgeKindFilter='';expandedCluster=null;previewedScope=null;++architectureGeneration;
    }
    const summary = summarizeReviewState({...state, pr: activePr});
    $('change-brief-title').textContent = summary.observed;
    $('change-brief-detail').textContent = summary.limitation;
    $('brief-action').disabled = !state.graph;
    renderChangedFiles(state);
    renderFindings(state);
    renderContinuity(state);
    $('graph-status').textContent = state.loading.has('graph') ? 'Loading bounded neighborhood…' : (state.error || '');
    const measured=state.timings.filter(item=>['graph','source'].includes(item.operation)).at(-1);
    $('interaction-timing').textContent=measured?`${measured.operation} ${measured.duration_ms} ms · local request boundary`:'No graph/source interaction timing yet.';
    $('stale').hidden = !state.details?.stale && !state.progress?.stale;
    if (state.graph && state.graph !== renderedGraph) {
      renderedGraph = state.graph;
      renderGraph(state.graph, state);
    }
    if (state.sourceComparison && (!pinned || state.sourceComparison.review_id === state.reviewId)) renderSourceComparison(state.sourceComparison);
    else if (state.source && (!pinned || state.source.review_id === state.reviewId)) renderSingleSource(state.source);
    if (layout === 'architecture' && (designLevel==='hld' || (state.details && state.files))) renderArchitectureImpact(state);
    if (designLevel==='lld')renderLld(state);
  }

  function renderContinuity(state) {
    const items = state.annotations?.items || [];
    const reviewed = items.filter(item => item.kind === 'reviewed' && item.status === 'active');
    const questions = items.filter(item => item.kind === 'question' && item.status === 'active');
    const bookmarks = items.filter(item => item.kind === 'bookmark' && item.status === 'active');
    const total = state.files?.items?.length || 0;
    $('continuity-snapshot').textContent = `proposal ${short(state.details?.proposal)}`;
    const delta=state.annotations?.delta; const since = delta?.prior_proposal
      ? ` · ${delta.changed_paths.length}${delta.truncated?'+':''} paths changed since ${short(delta.prior_proposal)}; prior notes remain on that snapshot`
      : ' · no prior inspected proposal to compare';
    $('continuity-summary').textContent = `${reviewed.length} of ${total} loaded files reviewed · ${bookmarks.length} bookmarks · ${questions.length} unresolved questions${publicStatic?' · page memory only; reload clears this state':since}`;
    $('mark-all-reviewed').disabled = !total || reviewed.length >= total || state.loading.has('annotations');
    $('bookmark-selection').disabled = !state.selection || state.loading.has('annotations');
    const container = $('continuity-items'); container.replaceChildren();
    items.filter(item => ['question','bookmark','handoff'].includes(item.kind)).forEach(item => {
      const row = node('div','continuity-item');
      row.append(node('span','',`${item.kind} · ${item.target_key}`),node('small','',`${publicStatic?'page memory':item.visibility} · v${item.version} · ${item.status}`));
      if (item.body) row.append(node('span','',item.body)); container.append(row);
    });
  }

  function annotationId() { return globalThis.crypto?.randomUUID?.() || `annotation-${Date.now()}-${Math.random().toString(16).slice(2)}`; }
  function existingAnnotation(kind, targetKey) { return (workspace.state.annotations?.items || []).find(item => item.kind === kind && item.target_key === targetKey); }
  async function saveAnnotation({kind,targetKey,targetPath=null,body='',visibility='private',status='active'}) {
    const current = existingAnnotation(kind,targetKey);
    return workspace.writeAnnotation({id:current?.id || annotationId(),visibility,kind,target_path:targetPath,target_key:targetKey,body,status,expected_version:current?.version || 0});
  }

  function renderFindings(state) {
    const list = $('evidence-list'); list.replaceChildren();
    const findings = state.findings?.items || [];
    $('evidence-count').textContent = String(findings.length);
    $('evidence-status').textContent = state.loading.has('findings')
      ? 'Loading scoped findings…'
      : (!state.findings ? 'Findings unavailable for this review identity.' : findings.length ? '' : 'No findings were produced in the authorized scope. This does not prove no behavioral change.');
    groupFindings(findings).forEach(group => {
      const section = node('section', 'evidence-group'); section.append(node('h4', 'evidence-question', group.title));
      const cards = node('div', 'evidence-group-cards');
      group.items.forEach(finding => {
      const classification = finding.classification;
      const card = node('article', `evidence-card ${classification.tone}`);
      const heading = node('div', 'evidence-heading');
      heading.append(node('span', `evidence-category ${classification.tone}`, classification.category), node('span', 'evidence-resolution', finding.resolution || 'unresolved'));
      const title = node('h4', '', String(finding.kind || 'Structural relationship').replaceAll('_', ' '));
      const start = Number.isInteger(finding.source_start_line) ? finding.source_start_line : null;
      const end = Number.isInteger(finding.source_end_line) ? finding.source_end_line : start;
      const location = `${finding.source_path || 'Authorized path unavailable'}${start ? `:${start}${end && end !== start ? `-${end}` : ''}` : ''}`;
      const provenanceCount = finding.provenance.length > 1 ? ` · ${finding.provenance.length} equivalent records` : '';
      const provenance = node('p', 'evidence-location', `${location} · proposal ${short(state.details?.proposal)} · evidence #${finding.provenance.join(', #')}${provenanceCount}`);
      const limitation = node('p', 'evidence-limitation', classification.limitation);
      const action = node('button', 'quiet evidence-action', classification.action); action.type = 'button';
      action.disabled = !finding.source_path;
      action.addEventListener('click', async () => {
        setLayout('connected', {remember:false}); $('side').value = 'diff';
        await workspace.selectPath({path:finding.source_path, side:'diff'}); rememberRoute();
      });
      card.append(heading, title, provenance, limitation, action); cards.append(card);
      });
      section.append(cards); list.append(section);
    });
  }

  function renderChangedFiles(state) {
    const panel = $('changed-files-panel');
    panel.hidden = false;
    const list = $('changed-files-list'); list.replaceChildren();
    const files = state.files?.items || [];
    const selectedPath = state.selection?.path || null;
    const selectedFile = files.find(file => file.path === selectedPath) || null;
    const reviewedPaths = new Set((state.annotations?.items || []).filter(item => item.kind === 'reviewed' && item.status === 'active').map(item => item.target_key));
    $('changed-files-count').textContent = String(files.length);
    $('changed-files-status').textContent = state.loading.has('files') ? 'Loading changed files…' : (!state.files ? 'Changed files unavailable.' : files.length ? '' : 'No changed files are available in the selected scope.');
    const visibleFiles = filterChangedFiles(files, $('file-search').value);
    $('file-search-status').textContent = $('file-search').value ? `${visibleFiles.length} of ${files.length} loaded files` : '';
    visibleFiles.forEach(file => {
      const button = node('button', 'changed-file'); button.type = 'button';
      button.dataset.kind = file.kind;
      const marker = file.kind === 'added' ? '+' : file.kind === 'deleted' ? '−' : file.kind === 'renamed' ? '↪' : '~';
      const name = file.path.split('/').pop();
      const folder = file.path.includes('/') ? file.path.slice(0, file.path.lastIndexOf('/')) : '.';
      const delta = node('span','file-delta');
      delta.append(node('span','file-additions',`+${Number.isInteger(file.additions)?file.additions:'?'}`),node('span','file-deletions',`−${Number.isInteger(file.deletions)?file.deletions:'?'}`));
      button.append(node('span', 'file-kind', marker), node('span', 'file-label', name), delta, node('span', 'file-folder', folder));
      if (file.old_path) button.append(node('span', 'file-old-path', `from ${file.old_path}`));
      const isReviewed=reviewedPaths.has(file.path);button.classList.toggle('reviewed',isReviewed);button.setAttribute('aria-current',String(file.path===selectedPath));
      button.setAttribute('aria-label', `${file.kind} ${file.path}, ${file.additions ?? 'unknown'} additions, ${file.deletions ?? 'unknown'} deletions${isReviewed?', reviewed':''}`);
      button.addEventListener('click', async () => {
        setLayout('connected', {remember:false});
        const side = file.kind === 'deleted' ? 'baseline' : file.kind === 'added' ? 'proposal' : 'diff';
        $('side').value = side;
        await workspace.selectPath({path: file.path, side});
        if(publicStatic)await loadPublishedFile(file.path);
        rememberRoute();
      });
      list.append(button);
    });
    $('selected-file-actions').hidden=!selectedFile;
    if(selectedFile){const isReviewed=reviewedPaths.has(selectedFile.path);$('selected-file-name').textContent=selectedFile.path;$('selected-file-delta').textContent=`+${selectedFile.additions ?? '?'} −${selectedFile.deletions ?? '?'}`;$('toggle-file-reviewed').dataset.path=selectedFile.path;$('toggle-file-reviewed').setAttribute('aria-pressed',String(isReviewed));$('toggle-file-reviewed').classList.toggle('reviewed',isReviewed);$('toggle-file-reviewed').lastChild.textContent=isReviewed?' Reviewed':' Mark reviewed';}
    $('more-files').hidden = !state.files?.next_cursor;
  }

  function setLayout(next, {remember = true} = {}) {
    layout = ['connected', 'source', 'architecture'].includes(next) ? next : 'connected';
    for (const name of ['connected', 'source', 'architecture']) {
      const control = $(`layout-${name}`);
      control.classList.toggle('active', layout === name);
      control.setAttribute('aria-pressed', String(layout === name));
    }
    $('architecture-impact').hidden = layout !== 'architecture';
    $('workspace-split').hidden = layout === 'architecture';
    $('workspace-split').classList.toggle('source-only', layout === 'source');
    if (layout === 'architecture' && workspace.state.details && workspace.state.files) renderArchitectureImpact(workspace.state);
    if (remember) rememberRoute();
  }

  function renderGraph(graph, state) {
    graphCursor = graph.next_cursor || null;
    graphWindow = 40; expandedCluster = null;
    $('graph-kind').textContent = graph.kind;
    $('direction').hidden = graph.kind !== 'neighborhood';
    const kinds = graph.kind === 'neighborhood' ? [...new Set(graphItems(graph).map(item => item.kind).filter(Boolean))].sort() : [];
    const filter = $('edge-filter'); filter.replaceChildren(new Option('All types', ''));
    kinds.forEach(kind => filter.append(new Option(kind, kind)));
    edgeKindFilter = kinds.includes(edgeKindFilter) ? edgeKindFilter : ''; filter.value = edgeKindFilter;
    renderGraphItems(graph, state);
  }

  function renderGraphItems(graph, state) {
    const container = $('graph'); container.replaceChildren();
    const allItems = graphItems(graph);
    const items = filterGraphItems(graph, graphQuery, edgeKindFilter);
    const localMore = items.length > graphWindow;
    $('more-graph').hidden = !graphCursor && !localMore;
    const hidden = Math.max(0, items.length - graphWindow) + Number(graph.omitted || 0);
    $('more-graph').textContent = hidden ? `Show more (${hidden} omitted)` : 'Show more';
    if (graph.kind === 'children') items.slice(0, graphWindow).forEach(path => container.append(pathNode(path)));
    if (graph.kind === 'symbols') items.slice(0, graphWindow).forEach(symbol => container.append(symbolNode(symbol)));
    if (graph.kind === 'neighborhood' && items.length <= 40) items.forEach(edge => container.append(edgeNode(edge, state)));
    if (graph.kind === 'neighborhood' && items.length > 40) renderEdgeClusters(container, items, state);
    if (!container.children.length) container.append(node('div', 'status', 'No authorized items match this view. Clear filters or inspect the coverage limit.'));
    const selected = state.selection?.symbol || state.selection?.path || 'all authorized paths';
    $('graph-selection').textContent = selected;
    $('graph-overview').textContent = `${items.length} shown of ${allItems.length} loaded${graph.omitted ? ` · ${graph.omitted} omitted by service limit` : ''}`;
  }

  function renderEdgeClusters(container, edges, state) {
    clusterEdges(edges).slice(0, graphWindow).forEach(([key, group]) => {
      const cluster = node('button', 'graph-item'); cluster.type = 'button';
      cluster.append(node('span', 'kind', `cluster · ${group.length} paths`), node('span', 'path', key), node('span', 'flow-preview', 'Click to inspect the bounded edges in this cluster'));
      cluster.addEventListener('click', () => { expandedCluster = expandedCluster === key ? null : key; renderGraphItems(workspace.state.graph, state); });
      container.append(cluster);
      if (expandedCluster === key) group.slice(0, 40).forEach(edge => container.append(edgeNode(edge, state)));
    });
  }

  function pathNode(item) {
    const path = typeof item === 'string' ? item : (Array.isArray(item) ? item[0] : item.path);
    const kind = Array.isArray(item) ? item[1] : (item.kind || 'path');
    const button = node('button', 'graph-item'); button.type = 'button';
    button.dataset.identity = path; button.setAttribute('aria-current', String(workspace.state.selection?.path === path));
    button.setAttribute('aria-label', `${item.status || 'affected'} ${kind} ${path}`);
    button.append(node('span', 'kind', item.status ? `${kind} · ${item.status}` : kind), node('span', 'path', path), node('span', 'flow-preview', `Open ${path} → load files or symbols`));
    button.addEventListener('click', async () => {
      const side = item.status && item.status !== 'unchanged' ? item.side : $('side').value;
      if (kind === 'file') await workspace.selectPath({path, side}); else await workspace.expandComponent({path, side});
      if(publicStatic&&kind==='file')await loadPublishedFile(path);
      rememberRoute();
    });
    return button;
  }

  function symbolNode(symbol) {
    const button = node('button', 'graph-item'); button.type = 'button';
    button.dataset.identity = symbol.stable_key; button.setAttribute('aria-current', String(workspace.state.selection?.symbol === symbol.stable_key));
    button.setAttribute('aria-label', `${symbol.status || 'affected'} ${symbol.kind} ${symbol.name} in ${symbol.path}`);
    button.append(node('span', 'kind', symbol.status ? `${symbol.kind} · ${symbol.status}` : symbol.kind), node('span', 'path', symbol.name), node('span', 'flow-preview', `${symbol.path}:${symbol.start_line} · click for ${direction} paths`));
    button.addEventListener('mouseenter', () => loadSymbolSource(symbol));
    button.addEventListener('focus', () => loadSymbolSource(symbol));
    button.addEventListener('click', async () => { await workspace.selectSymbol({symbol: symbol.stable_key, path: symbol.path, side: $('side').value, direction}); rememberRoute(); });
    return button;
  }

  function edgeNode(edge) {
    const button = node('button', 'graph-item'); button.type = 'button';
    button.dataset.identity = edge.stable_key || `${edge.from_symbol}-${edge.to_symbol}`;
    button.setAttribute('aria-label', `${edge.kind} relationship from ${edge.from_symbol || 'unresolved source'} to ${edge.to_symbol || 'unresolved target'}; ${edge.resolution}`);
    button.append(node('span', 'kind', `${edge.kind} · ${edge.confidence}`), node('span', 'path', `${edge.from_symbol} → ${edge.to_symbol}`), node('span', 'flow-preview', edge.source_path || edge.resolution));
    button.addEventListener('click', async () => { await workspace.selectSymbol({symbol: direction === 'outgoing' ? edge.to_symbol : edge.from_symbol, path: edge.source_path, side: $('side').value, direction}); rememberRoute(); });
    return button;
  }

  function loadSymbolSource(symbol) {
    if (pinned) return;
    if (symbol.baseline || symbol.proposal) {
      workspace.loadSourceComparison({path:symbol.path,baseline:symbol.baseline,proposal:symbol.proposal}); return;
    }
    if (!symbol.blob_id || !Number.isInteger(symbol.start_line) || !Number.isInteger(symbol.end_line)) return;
    workspace.loadSource({path: symbol.path, blob: symbol.blob_id, side: symbol.side || ($('side').value==='diff'?'proposal':$('side').value), startLine: symbol.start_line, endLine: symbol.end_line});
  }

  async function loadPublishedFile(path) {
    const generation=workspace.generation,reviewId=workspace.state.reviewId;
    try{const descriptors=await publicTransport.fileDescriptors(reviewId,path);if(generation!==workspace.generation||workspace.state.selection?.path!==path)return;const file=workspace.state.files?.items.find(item=>item.path===path);if((file?.kind!=='added'&&!descriptors.baseline)||(file?.kind!=='deleted'&&!descriptors.proposal)){pinned=false;clearSourceInspector('Exact file source is not fully published for this pair. Inspect available HLD/LLD evidence; missing excerpts are never replaced.');return;}await workspace.loadSourceComparison({path,...descriptors});}
    catch(error){if(generation===workspace.generation){if(error.invalidateReview)workspace.clearReview(error.message);else{pinned=false;clearSourceInspector(error.message);}}}
  }

  function appendDiffLine(code, text, kind, number = null) {
    const line=node('span',`diff-line ${kind}`,text ?? ' '); if (number !== null) line.dataset.line=number; code.append(line);
  }

  function renderSourceComparison(comparison) {
    const before=comparison.baseline,after=comparison.proposal,rows=diffLines(before?.text || '',after?.text || '');
    $('source-title').textContent = comparison.path;
    $('source-meta').textContent = `${before?.commit?.slice(0,8) || 'absent'} baseline → ${after?.commit?.slice(0,8) || 'absent'} proposal · ${rows.filter(row=>row.kind!=='same').length} changed rows${before?.truncated||after?.truncated?' · bounded excerpt; changes outside these lines are not shown':''}`;
    $('baseline-source-label').textContent=`Baseline · ${before?.commit?.slice(0,8) || 'file absent'}`;$('proposal-source-label').textContent=`Proposal · ${after?.commit?.slice(0,8) || 'file absent'}`;
    const left=$('source-baseline').firstElementChild,right=$('source-proposal').firstElementChild,unified=$('source-unified').firstElementChild;left.replaceChildren();right.replaceChildren();unified.replaceChildren();
    let beforeLine=before?.start_line || 1,afterLine=after?.start_line || 1;
    rows.forEach(row=>{
      if(row.kind==='same'){appendDiffLine(left,row.before,'same',beforeLine++);appendDiffLine(right,row.after,'same',afterLine++);appendDiffLine(unified,`  ${row.after}`,'same');}
      if(row.kind==='removed'){appendDiffLine(left,row.before,'removed',beforeLine++);appendDiffLine(right,'','empty');appendDiffLine(unified,`- ${row.before}`,'removed');}
      if(row.kind==='added'){appendDiffLine(left,'','empty');appendDiffLine(right,row.after,'added',afterLine++);appendDiffLine(unified,`+ ${row.after}`,'added');}
      if(row.kind==='modified'){appendDiffLine(left,row.before,'removed',beforeLine++);appendDiffLine(right,row.after,'added',afterLine++);appendDiffLine(unified,`- ${row.before}`,'removed');appendDiffLine(unified,`+ ${row.after}`,'added');}
    });
    setCodeDiffMode(codeDiffMode);
    $('pin').disabled = false; $('pin').textContent = pinned ? 'Unpin' : 'Pin';
  }

  function renderSingleSource(source) {
    const comparison={review_id:source.review_id,path:source.path,baseline:null,proposal:null};
    if (source.commit===analysisBaseline(workspace.state.details || {})) comparison.baseline=source; else comparison.proposal=source;
    renderSourceComparison(comparison);
  }

  function setCodeDiffMode(mode) {
    codeDiffMode=mode==='unified'?'unified':'split';
    $('source-comparison').hidden=codeDiffMode!=='split';$('source-unified').hidden=codeDiffMode!=='unified';
    for(const name of ['split','unified']){$(`diff-${name}`).classList.toggle('active',codeDiffMode===name);$(`diff-${name}`).setAttribute('aria-pressed',String(codeDiffMode===name));}
  }

  function ensureMermaid() {
    if (mermaidLoading) return mermaidLoading;
    mermaidLoading=new Promise((resolve,reject)=>{const script=document.createElement('script');script.src=publicStatic?publicTransport.assetURL('mermaid.js'):'/assets/mermaid.js';script.onload=()=>{if(!window.mermaid){reject(new Error('Local Mermaid renderer unavailable'));return;}window.mermaid.initialize({startOnLoad:false,securityLevel:'strict',theme:'base',fontFamily:'ui-monospace, SFMono-Regular, Menlo, monospace',themeVariables:{background:'#101214',primaryColor:'#20252c',primaryTextColor:'#edf0f4',primaryBorderColor:'#718094',lineColor:'#8b99ad',tertiaryColor:'#171a1e'},flowchart:{htmlLabels:false,curve:'basis',useMaxWidth:false,wrappingWidth:320,nodeSpacing:32,rankSpacing:56,padding:18},maxTextSize:20000});resolve(window.mermaid);};script.onerror=()=>reject(new Error('Local Mermaid renderer could not load'));document.head.append(script);});
    return mermaidLoading;
  }

  function mountDiagramViewport(frame, host, svg, {initialMode = 1,initialSelector = null,hint = 'Select a component to inspect code. Scroll the canvas to pan.'} = {}) {
    if (!svg) return;
    const bounds = svg.viewBox.baseVal;
    if (!(bounds.width > 0 && bounds.height > 0)) return;
    svg.removeAttribute('style');
    host.tabIndex = 0;
    host.setAttribute('aria-label', 'Architecture canvas. Scroll to pan; use zoom controls to read nodes.');
    const tools = node('div', 'diagram-tools');
    const zoom = node('output', 'diagram-zoom');
    zoom.setAttribute('aria-live', 'polite');
    let mode = initialMode, scale = 1, framed = false;
    const apply = () => {
      if (!host.clientWidth) return;
      scale = mode === 'overview'
        ? Math.min(diagramScale(bounds.width, host.clientWidth), diagramScale(bounds.height, host.clientHeight))
        : diagramScale(bounds.width, host.clientWidth, mode);
      svg.setAttribute('width', String(Math.round(bounds.width * scale)));
      svg.setAttribute('height', String(Math.round(bounds.height * scale)));
      zoom.textContent = `${Math.round(scale * 100)}%`;
      if (!framed) {
        host.scrollTop = 0; host.scrollLeft = 0;
        // Start detail views on a readable component rather than empty connector space.
        if (typeof mode === 'number') {
          const component = initialSelector?(svg.querySelector('g.node[aria-current="true"]') || svg.querySelector(initialSelector)):svg.querySelector('g.node[id*="-flowchart-area"]');
          if (component) {
            const card = component.getBoundingClientRect(), canvas = host.getBoundingClientRect();
            host.scrollLeft = Math.max(0, card.left - canvas.left - (host.clientWidth - card.width) / 2);
            if(initialSelector)host.scrollTop = Math.max(0,card.top - canvas.top - 16);
          }
        }
        framed = true;
      }
    };
    const changeZoom = next => {
      const centerX = (host.scrollLeft + host.clientWidth / 2) / scale;
      const centerY = (host.scrollTop + host.clientHeight / 2) / scale;
      const relative = typeof next === 'function';
      mode = relative ? next(scale) : next;
      if (!relative) framed = false;
      apply();
      if (relative) {
        host.scrollLeft = Math.max(0, centerX * scale - host.clientWidth / 2);
        host.scrollTop = Math.max(0, centerY * scale - host.clientHeight / 2);
      }
    };
    const action = (label, next, accessible = label) => {
      const button = node('button', 'quiet', label);
      button.type = 'button'; button.setAttribute('aria-label', accessible);
      button.addEventListener('click', () => {
        changeZoom(next);
      });
      tools.append(button);
    };
    action('Fit all', 'overview', 'Fit the entire architecture diagram');
    action('Fit width', 'fit'); action('100%', 1, 'Read diagram at full size');
    action('−', value => value / 1.25, 'Zoom diagram out');
    tools.append(zoom);
    action('+', value => value * 1.25, 'Zoom diagram in');
    frame.insertBefore(tools, host);
    frame.append(node('p', 'diagram-hint', hint));
    apply();
    if (typeof ResizeObserver !== 'undefined') {
      const observer = new ResizeObserver(apply); observer.observe(host); diagramObservers.push(observer);
    }
    host.addEventListener('keydown', event => {
      if (event.target !== host) return;
      if (event.key === '+' || event.key === '=') changeZoom(value => value * 1.25);
      else if (event.key === '-') changeZoom(value => value / 1.25);
      else if (event.key === '0') changeZoom('fit');
      else return;
      event.preventDefault();
    });
  }

  async function renderArchitectureImpact(state) {
    if (designLevel === 'hld') { renderHld(state); return; }
    if (designLevel === 'lld') return;
    const files=state.files?.items || [],findings=state.findings?.items || [],areas=architectureAreas(files);
    const baseline=analysisBaseline(state.details || {}),selected=state.details?.selected_baseline;
    const key=[state.reviewId,baseline,selected,state.details?.proposal,architectureMode,JSON.stringify(files),JSON.stringify(findings)].join('|');
    const comparison=selected && selected!==baseline?`selected base ${short(selected)} · analyzed merge base ${short(baseline)} → ${short(state.details?.proposal)}`:`exact ${short(baseline)} → ${short(state.details?.proposal)}`;
    const unresolved = findings.filter(item=>item.resolution!=='resolved').length;
    $('architecture-source').disabled=!(state.details?.capabilities || []).includes('export');
    $('architecture-impact-summary').textContent=`${files.length} changed file${files.length === 1 ? '' : 's'} across ${areas.length} structural area${areas.length === 1 ? '' : 's'} · ${unresolved} unresolved impact${unresolved === 1 ? '' : 's'} · ${comparison}`;
    if(lastArchitectureKey===key)return;lastArchitectureKey=key;const generation=++architectureGeneration;
    diagramObservers.splice(0).forEach(observer => observer.disconnect());
    const modes=architectureMode==='compare'?['baseline','proposal']:[architectureMode],container=$('architecture-diagrams');container.replaceChildren();container.classList.toggle('compare',modes.length===2);$('architecture-status').textContent='Rendering local Mermaid diagrams…';
    const sources=modes.map(mode=>({mode,source:architectureMermaid({files,findings,details:state.details},mode)}));$('architecture-mermaid-source').firstElementChild.textContent=sources.map(item=>`## ${item.mode}\n\n\`\`\`mermaid\n${item.source}\n\`\`\``).join('\n\n');
    try { const renderer=await ensureMermaid(); if(generation!==architectureGeneration)return;
      for(const item of sources){await renderer.parse(item.source);const rendered=await renderer.render(`csb-architecture-${generation}-${item.mode}`,item.source);if(generation!==architectureGeneration)return;const frame=node('section','architecture-frame');frame.append(node('h4','',item.mode==='combined'?'Combined structural diff':item.mode));const host=node('div','architecture-svg');host.innerHTML=rendered.svg;frame.append(host);container.append(frame);const svg=host.querySelector('svg');if(svg)svg.setAttribute('aria-label',`${item.mode} structural architecture diagram for ${short(baseline)} to ${short(state.details?.proposal)}`);
        host.querySelectorAll('g.node text').forEach(label => label.querySelector('tspan')?.classList.add('node-title'));
        mountDiagramViewport(frame, host, svg);
        host.querySelectorAll('g.node').forEach(graphNode=>{const match=graphNode.id.match(/-flowchart-area(\d+)-/);if(!match)return;const area=areas[Number(match[1])];if(!area)return;const paths=area.paths.filter(path=>item.mode==='baseline'?files.find(file=>file.path===path)?.kind!=='added':item.mode==='proposal'?files.find(file=>file.path===path)?.kind!=='deleted':true);if(!paths.length)return;graphNode.setAttribute('tabindex','0');graphNode.setAttribute('role','button');graphNode.setAttribute('aria-label',`Inspect ${area.name}, ${paths.length} changed file${paths.length === 1 ? '' : 's'}`);const inspect=async()=>{setLayout('connected',{remember:false});const side=item.mode==='combined'?'diff':item.mode;$('side').value=side;await workspace.selectPath({path:paths[0],side});rememberRoute();};graphNode.addEventListener('click',inspect);graphNode.addEventListener('keydown',event=>{if(event.key==='Enter'||event.key===' '){event.preventDefault();inspect();}});});
      }
      $('architecture-status').textContent='Mermaid rendered locally from authorized changed paths. Select an area to open its code evidence.';
    } catch(error){if(generation!==architectureGeneration)return;lastArchitectureKey='';$('architecture-status').textContent=`Diagram unavailable: ${error.message}. Mermaid source remains inspectable.`;}
  }

  function setDesignLevel(next, {remember = true, load = true, inferParent = true} = {}) {
    if(inferParent&&next==='lld'&&designLevel==='hld'&&workspace.state.hld){const payload=workspace.state.hld;let version=hldMode==='accepted'?payload.accepted:hldMode==='baseline'?payload.baseline:payload.proposal;if(!version?.elements.some(element=>element.id===hldSelection))version=payload.baseline;const selected=version?.elements.find(element=>element.id===hldSelection);const component=selected?.kind==='component'?selected:version?.elements.find(element=>element.id===selected?.component_id&&element.kind==='component');if(component){hldSelection=component.id;lldComponent=component.id;lldParentHld=version.id;lldDetail='module';lldElement=null;lldNode=null;lldPinned=false;}}
    designLevel = ['structural','hld','lld'].includes(next) ? next : 'structural';
    ++architectureGeneration; ++hldSourceGeneration; lastHldKey = ''; lastArchitectureKey = '';
    ++lldSourceRequest;clearTimeout(lldSourceTimer);lastLldKey='';
    if(designLevel!=='lld')workspace.invalidateLld();
    diagramObservers.splice(0).forEach(observer=>observer.disconnect());
    for (const name of ['structural','hld','lld']) { const control=$('design-level-'+name); control.classList.toggle('active',name===designLevel); control.setAttribute('aria-pressed',String(name===designLevel)); }
    for (const element of [$('architecture-impact').querySelector('.architecture-impact-heading'),$('architecture-impact').querySelector('.architecture-legend'),$('architecture-status'),$('architecture-diagrams')]) element.hidden = designLevel !== 'structural';
    $('architecture-mermaid-source').hidden = true;
    $('hld-review').hidden = designLevel !== 'hld'; $('lld-review').hidden = designLevel !== 'lld';
    if(designLevel==='lld'){renderLld(workspace.state);if(load)loadSelectedLldCatalog().then(()=>{if(lldElement && designLevel==='lld')workspace.loadLldDetail(lldElement,{baselineEntry:lldBaselineEntry,correspondenceKind:lldCorrespondence});});}
    if (designLevel==='hld') { renderHld(workspace.state); if (load && !workspace.state.hld) workspace.loadHld(); }
    else if (designLevel==='structural' && workspace.state.details && workspace.state.files && layout==='architecture') renderArchitectureImpact(workspace.state);
    if (remember) rememberRoute();
  }

  function zoomHldToLld(componentId,version) {
    hldSelection=componentId;lldComponent=componentId;lldParentHld=version.id;lldDetail='module';lldElement=null;
    lldNode=null;lldPinned=false;lldBaselineEntry=null;lldCorrespondenceId=null;
    setDesignLevel('lld');
  }

  async function loadSelectedLldCatalog({cursor = null} = {}) {
    if(!lldParentHld){if(!workspace.state.hld)await workspace.loadHld();if(designLevel!=='lld')return;lldParentHld=workspace.state.hld?.proposal?.id || workspace.state.hld?.baseline?.id || null;}
    if(!lldParentHld){workspace.state.lldError='An exact HLD parent is required for LLD. Reload after confirming repository context.';workspace.emit();return;}
    await workspace.loadLldCatalog({parentHld:lldParentHld,component:lldComponent,cursor});
    rememberRoute({replace:true});
  }

  function renderLld(state) {
    if(designLevel!=='lld')return;
    const payload=state.lld,detail=state.lldDetail;
    const versions=[state.hld?.baseline,state.hld?.proposal,state.hld?.accepted].filter(Boolean),parent=versions.find(version=>version.id===lldParentHld),component=parent?.elements.find(item=>item.id===lldComponent);
    $('lld-component-name').textContent=component?.name||lldComponent||'Authorized inventory';
    $('lld-parent-identity').textContent=`baseline ${short(payload?.baseline?.snapshot || analysisBaseline(state.details || {}))} → proposal ${short(payload?.proposal?.snapshot || state.details?.proposal)} · scope ${short(payload?.scope_version)} · ${lldParentHld?'selected HLD component':'all authorized declarations'}`;
    $('lld-full-identity').textContent=`review ${state.reviewId} · scope ${payload?.scope_version || 'unavailable'} · HLD ${lldParentHld || 'unmapped inventory'} · component ${lldComponent || 'all authorized'} · baseline catalog ${payload?.baseline?.id || 'unavailable'} · proposal catalog ${payload?.proposal?.id || 'unavailable'} · selected entry ${lldElement || 'none'} · baseline detail ${detail?.baseline?.id || 'none'} · proposal detail ${detail?.proposal?.id || 'none'}`;
    $('lld-summary').textContent=state.loading.has('lld')?'Loading authorized LLD inventory…':state.loading.has('lld-detail')?'Loading selected implementation…':state.lldError?`LLD unavailable: ${state.lldError}`:payload?lldOutcome(detail?.delta || payload.delta):'No authorized LLD catalog loaded.';
    $('lld-reload').disabled=state.loading.has('lld');
    $('lld-load-history').hidden=publicStatic || !lldElement || !payload;$('lld-load-history').disabled=state.loading.has('lld-history');
    renderLldHistory(state.lldHistory);
    if(!payload){$('lld-confirmation-preview').hidden=true;$('lld-confirmation-preview').firstElementChild.textContent='';$('lld-invariant').value='';$('lld-known-gaps').checked=false;$('lld-activate-default').checked=false;}
    for(const mode of ['combined','compare','baseline','proposal']){$('lld-'+mode).classList.toggle('active',hldMode===mode);$('lld-'+mode).setAttribute('aria-pressed',String(hldMode===mode));}
    const exportAllowed=Boolean((detail || payload)?.can_export || (detail || payload)?.export_allowed);
    $('lld-mermaid').disabled=!detail || !exportAllowed;$('lld-export').disabled=!payload || !exportAllowed;
    if(publicStatic){$('lld-export').disabled=true;$('lld-export').title='Only the published full review Markdown is available.';}
    if(!exportAllowed){$('lld-mermaid-source').hidden=true;$('lld-mermaid-source').firstElementChild.textContent='';}
    const coverage=$('lld-coverage');coverage.replaceChildren();
    if(payload) {
      for(const [side,version] of [['Baseline',detail?.baseline || payload.baseline],['Proposal',detail?.proposal || payload.proposal]])if(version){const value=version.coverage || {};coverage.append(node('p','',`${side}: ${value.indexed || 0} declarations · inventory ${value.inventory_complete?'complete':'partial'} · structure ${value.structure_complete?'complete':'partial'} · ${version.graph?(version.graph.complete?'covered local syntax':'partial local syntax'):'local flow loaded on selection'}`));}
      const gaps=[...new Set([...(payload.delta?.limitations || []),...(detail?.delta?.limitations || []),...(detail?.baseline?.graph?.limitations || []),...(detail?.proposal?.graph?.limitations || [])])].slice(0,12);
      for(const gap of gaps)coverage.append(node('p','warning',String(gap)));
      if(payload.client_omitted)coverage.append(node('p','warning',`${payload.client_omitted} declarations omitted by the client bound.`));
    }
    renderLldCatalog(payload);
    const changes=$('lld-changes');changes.replaceChildren();
    if(detail) {
      const changed=detail.delta?.changes || [];
      if(changed.length){changes.append(node('h4','','Observed implementation changes'));for(const change of changed.slice(0,20))changes.append(node('p','',`${String(change.kind || change.classification).replaceAll('_',' ')} · ${change.classification || 'changed'} · ${change.resolution || 'static evidence'}`));}
      else changes.append(node('p','status',lldOutcome(detail.delta)));
      if(hldMode==='combined')changes.append(node('p','mono','Diff retains each snapshot’s local graph. Semantic classifications above do not imply runtime equivalence or a matched execution path.'));
    }
    const key=[state.reviewId,payload?.scope_version,lldParentHld,lldComponent,detail?.baseline?.id,detail?.proposal?.id,detail?.delta?.id,hldMode].join('|');
    if(!detail){if(lastLldKey!==key){++architectureGeneration;lastLldKey=key;$('lld-diagrams').replaceChildren();}lastLldInspectorKey='';activeLldEvidence=null;lldSourcePairView.clear();$('lld-inspector').replaceChildren(node('p','status',state.loading.has('lld-detail')?'Loading exact implementation evidence…':'Select a module, type or function from the authorized inventory.'));$('lld-acceptance').hidden=true;$('lld-detail-breadcrumb').textContent='';$('lld-confirmation-preview').hidden=true;$('lld-confirmation-preview').firstElementChild.textContent='';return;}
    if(lastLldKey!==key){lastLldKey=key;renderLldDiagrams(detail);}
    renderLldInspector(detail);
  }

  function renderLldCatalog(payload) {
    lldCatalogView.render(payload,{query:lldQuery,selection:lldElement,loading:workspace.state.loading.has('lld')});
  }

  function renderLldHistory(history) {
    const container=$('lld-history');container.replaceChildren();container.hidden=!history;if(!history)return;
    container.append(node('h3','','Accepted LLD history'),node('p','warning',`${workspace.state.lldDetail?'Historical records are separate from the current diagram above.':'Current local behavior is unavailable.'} These immutable records retain their original commits and assertions. They are not a current analysis result.`));
    let count=0;for(const [label,record] of [['As-built',history.accepted],['Intended',history.accepted_intended]])if(record){count++;const section=node('section');section.append(node('h4','',label),node('p','mono',`commit ${record.observed_design?.snapshot || 'unavailable'} · accepted ${record.accepted_id} · lifecycle ${record.lifecycle_state} · behavior ${record.observed_design?.lld?.behavior_fingerprint || 'unavailable'}`));appendHldList(section,'Recorded invariants',(record.design?.lld?.invariants || []).map(item=>`${item.statement} · ${item.resolution} · ${item.verification}`));container.append(section);}if(!count)container.append(node('p','status','No authorized accepted LLD record exists for this qualified declaration.'));
  }

  async function openLldEntry(id,entry = null,{remember = true} = {}) {
    lldElement=id;lldDetail=['file','module','import'].includes(entry?.kind)?'module':['type','struct','enum','trait','impl','type_alias','field','variant'].includes(entry?.kind)?'type':'function';lldNode=null;lldPinned=false;lldBaselineEntry=null;lldCorrespondenceId=null;activeLldEvidence=null;++lldSourceRequest;clearTimeout(lldSourceTimer);
    lastLldInspectorKey='';$('lld-invariant').value='';$('lld-known-gaps').checked=false;$('lld-activate-default').checked=false;lldInvariantId=null;
    $('lld-confirmation-status').textContent='Preview is required before confirmation.';
    if(remember)rememberRoute();await workspace.loadLldDetail(id);
  }

  async function renderLldDiagrams(payload) {
    const generation=++architectureGeneration;diagramObservers.splice(0).forEach(observer=>observer.disconnect());
    const modes=hldMode==='compare'?['baseline','proposal']:[hldMode==='accepted'?'combined':hldMode],container=$('lld-diagrams');container.replaceChildren();container.classList.toggle('compare',modes.length===2);
    const sources=modes.map(mode=>({mode,...lldMermaid(payload,mode)}));
    $('lld-mermaid-source').firstElementChild.textContent=(payload.can_export || payload.export_allowed)?sources.map(item=>`## ${item.mode}\n\n\`\`\`mermaid\n${item.source}\n\`\`\``).join('\n\n'):'';
    try {
      const renderer=await ensureMermaid();if(generation!==architectureGeneration||designLevel!=='lld')return;
      for(const item of sources) {
        await renderer.parse(item.source);const rendered=await renderer.render(`csb-lld-${generation}-${item.mode}`,item.source);if(generation!==architectureGeneration||designLevel!=='lld')return;
        const frame=node('section','architecture-frame');frame.append(node('h4','',item.mode==='combined'?'LLD PR comparison':item.mode));const host=node('div','architecture-svg lld-svg');host.innerHTML=rendered.svg;frame.append(host);container.append(frame);const svg=host.querySelector('svg');svg?.setAttribute('aria-label',`${item.mode} static local control flow`);
        let initialIndex=item.nodes.findIndex(record=>record.node.id===lldNode);if(initialIndex<0)initialIndex=item.nodes.findIndex(record=>record.side===(item.mode==='baseline'?'baseline':'proposal') && (record.node.operation==='entry' || record.entry?.id===record.detail.entry.id));if(initialIndex<0)initialIndex=0;
        if(item.nodes[initialIndex])host.dataset.initialDesignNode=item.nodes[initialIndex].node.id;
        mountDiagramViewport(frame,host,svg,{initialMode:LLD_INITIAL_SCALE,initialSelector:`g.node[id*="flowchart-lld${initialIndex}-"]`,hint:'Select a step to inspect code. Scroll the canvas to pan.'});
        host.querySelectorAll('g.node').forEach(graphNode=>{
          const match=graphNode.id.match(/-flowchart-lld(\d+)-/),record=match?item.nodes[Number(match[1])]:null;if(!record)return;
          graphNode.dataset.lldNode=record.node.id;graphNode.dataset.lldSide=record.side;graphNode.setAttribute('tabindex','0');graphNode.setAttribute('role','button');graphNode.setAttribute('aria-label',`${record.side}: ${record.node.label}. ${record.node.resolution}. Inspect exact source.`);graphNode.setAttribute('aria-current',String(record.node.id===lldNode));
          const evidence=lldEvidence(record.detail,record.node)[0];
          const preview=()=>{if(lldPinned&&lldNode!==record.node.id)return;showLldNode(record,evidence,false);};
          graphNode.addEventListener('pointerenter',()=>{clearTimeout(lldSourceTimer);lldSourceTimer=setTimeout(preview,100);});graphNode.addEventListener('focus',preview);
          graphNode.addEventListener('pointerleave',()=>{clearTimeout(lldSourceTimer);if(!lldPinned){++lldSourceRequest;++workspace.lldSourceGeneration;workspace.state.lldSource=null;host.querySelectorAll('.lld-node-source').forEach(item=>item.remove());}});
          const pin=()=>record.entry && record.entry.id!==record.detail.entry.id?openLldEntry(record.entry.id,record.entry):showLldNode(record,evidence,true);graphNode.addEventListener('click',pin);graphNode.addEventListener('keydown',event=>{if(event.key==='Enter'||event.key===' '){event.preventDefault();pin();}else if(event.key==='Escape'){lldPinned=false;lldNode=null;activeLldEvidence=null;++lldSourceRequest;++workspace.lldSourceGeneration;workspace.state.lldSource=null;host.querySelectorAll('.lld-node-source').forEach(item=>item.remove());rememberRoute();renderLldInspector(workspace.state.lldDetail);}});
          if(lldNode===record.node.id && lldPinned)showLldNode(record,evidence,true,{remember:false});
        });
      }
    } catch(error){if(generation!==architectureGeneration)return;lastLldKey='';container.replaceChildren(node('p','status',`Local diagram unavailable: ${error.message}. Declaration facts and exact source evidence remain available.`));}
  }

  async function showLldNode(record,evidence,pin,{remember = true} = {}) {
    if(!workspace.state.lldDetail || designLevel!=='lld')return;
    lldNode=record.node.id;lldDetail='flow';if(pin)lldPinned=true;activeLldEvidence=evidence || null;
    $('lld-diagrams').querySelectorAll('g.node[data-lld-node]').forEach(item=>item.setAttribute('aria-current',String(item.dataset.lldNode===lldNode)));
    $('lld-diagrams').querySelectorAll('.lld-node-source').forEach(item=>item.remove());
    if(remember&&pin)rememberRoute();
    if(!evidence){$('lld-source-status').textContent='This operation has no retained source evidence. Another revision is never substituted.';return;}
    const token=++lldSourceRequest,key=[workspace.generation,workspace.lldDetailGeneration,evidence.id].join('|');
    let promise=lldActiveSourceKey===key?lldActiveSourcePromise:null;
    if(!promise){promise=workspace.loadLldSource(evidence);lldActiveSourcePromise=promise;lldActiveSourceKey=key;promise.finally(()=>{if(lldActiveSourcePromise===promise){lldActiveSourcePromise=null;lldActiveSourceKey=null;}}).catch(()=>{});}
    const source=await promise;
    if(token!==lldSourceRequest || !source || source.evidence_id!==activeLldEvidence?.id || designLevel!=='lld')return;
    renderLldInspector(workspace.state.lldDetail);renderLldBoxSource(record.node.id,record.side,source);
  }

  function renderLldBoxSource(nodeId,side,source) {
    $('lld-diagrams').querySelectorAll('.lld-node-source').forEach(item=>item.remove());
    const target=[...$('lld-diagrams').querySelectorAll('g.node[data-lld-node]')].find(item=>item.dataset.lldNode===nodeId&&item.dataset.lldSide===side);if(!target)return;
    const box=target.getBBox(),preview=document.createElementNS('http://www.w3.org/2000/svg','foreignObject');preview.setAttribute('class','lld-node-source');preview.setAttribute('x',String(box.x));preview.setAttribute('y',String(box.y));preview.setAttribute('width',String(Math.max(box.width,280)));preview.setAttribute('height',String(Math.max(box.height,130)));
    const body=node('div','lld-box-preview');body.setAttribute('xmlns','http://www.w3.org/1999/xhtml');body.append(node('strong','',`${side} · ${source.path}:${source.start_line} · ${short(source.commit)}`),node('pre','',source.text.split('\n').slice(0,6).join('\n')));preview.append(body);target.append(preview);
  }

  function renderLldInspector(payload) {
    if(!payload)return;
    const selected=payload.proposal || payload.baseline,key=[payload.baseline?.id,payload.proposal?.id,payload.accepted?.accepted_id,payload.accepted_intended?.accepted_id,payload.accepted_status,hldMode].join('|'),container=$('lld-inspector');
    if(lastLldInspectorKey!==key){
      lastLldInspectorKey=key;container.replaceChildren();
      container.append(node('h3','',selected.entry.name),node('p','mono',`${selected.entry.logical_key} · ${selected.entry.path}`));
      $('lld-detail-breadcrumb').textContent=`/ ${selected.entry.module} / ${selected.entry.name}`;
      const facts=node('div','hld-facts');
      for(const [side,detail] of hldMode==='baseline'?[['Baseline',payload.baseline]]:hldMode==='proposal'?[['Proposal',payload.proposal]]:[['Baseline',payload.baseline],['Proposal',payload.proposal]]){const section=node('section');section.append(node('strong','',side));if(!detail)section.append(node('p','',side==='Baseline'?'Added in proposal.':'Removed from proposal.'));else{section.append(node('pre','lld-signature',detail.entry.signature),node('p','mono',`${detail.entry.visibility} · ${detail.entry.start_line}-${detail.entry.end_line} · parent ${detail.entry.parent?.state}`));for(const limitation of detail.entry.limitations || [])section.append(node('p','status',limitation));}facts.append(section);}container.append(facts);
      if(payload.proposal&&!publicStatic){
        const fields=node('div','hld-confirm-fields'),label=node('label','','Reviewer correspondence'),select=node('select');select.id='lld-baseline-counterpart';const automatic=node('option','','Automatic unique qualified correspondence');automatic.value='';select.append(automatic);
        const candidates=[...new Map(lldCatalogRows(workspace.state.lld).filter(row=>row.baseline?.kind===payload.proposal.entry.kind).map(row=>[row.baseline.id,row.baseline])).values()];
        for(const candidate of candidates){const option=node('option','',`${candidate.name} · ${candidate.container || candidate.module} · ${candidate.path}:${candidate.start_line}`);option.value=candidate.id;select.append(option);}
        if(lldBaselineEntry&&!candidates.some(item=>item.id===lldBaselineEntry)){const option=node('option','',`${lldBaselineEntry} · pinned authorized selection`);option.value=lldBaselineEntry;select.append(option);}select.value=lldBaselineEntry || '';label.append(select);fields.append(label);
        const kindLabel=node('label','','Correspondence assertion'),kind=node('select');kind.id='lld-correspondence-kind';for(const value of ['rename','move']){const option=node('option','',value==='rename'?'Rename':'Move');option.value=value;kind.append(option);}kind.value=lldCorrespondence;kind.disabled=!lldBaselineEntry;kindLabel.append(kind);fields.append(kindLabel);container.append(fields,node('p','mono','Correspondence is an explicit reviewer assertion. It links occurrences and does not establish equivalent behavior. Candidates come only from the loaded authorized catalog.'));
        select.addEventListener('change',async()=>{lldBaselineEntry=select.value || null;lldCorrespondence='rename';lldCorrespondenceId=null;lldNode=null;lldPinned=false;activeLldEvidence=null;++lldSourceRequest;invalidateLldConfirmation();rememberRoute();await workspace.loadLldDetail(lldElement,{baselineEntry:lldBaselineEntry,correspondenceKind:lldCorrespondence});});
        kind.addEventListener('change',async()=>{lldCorrespondence=kind.value;invalidateLldConfirmation();rememberRoute();await workspace.loadLldDetail(lldElement,{baselineEntry:lldBaselineEntry,correspondenceKind:lldCorrespondence});});
      }
      const children=selected.declarations || [];if(children.length){container.append(node('h4','','Declarations'));for(const entry of children.slice(0,100)){const button=node('button','quiet lld-child',`${entry.kind} · ${entry.name}`);button.type='button';button.addEventListener('click',()=>openLldEntry(entry.id,entry));container.append(button);}}
      appendHldList(container,'Possible callers',(payload.delta?.possible_callers || []).map(item=>`${item.logical_key} · ${item.resolution} · ${item.classification}`));
      for(const [label,accepted] of [['As-built',payload.accepted],['Intended',payload.accepted_intended]])if(accepted){const data=accepted.design?.lld,observed=accepted.observed_design?.lld,changed=observed?.behavior_fingerprint!==selected.behavior_fingerprint;appendHldList(container,`${label} recorded assertions`,[...(data?.invariants || []).map(item=>`${item.statement} · ${item.resolution} · ${item.verification}`),`Original commit ${accepted.observed_design?.snapshot || 'unavailable'} · ${changed?'Implementation changed; prior assertion needs review':'Same implementation fingerprint; runtime claim remains unverified'}`]);}
      container.append(node('h4','','Exact source evidence'));const evidenceList=node('div','hld-evidence');
      for(const [side,detail] of [['Baseline',payload.baseline],['Proposal',payload.proposal]])for(const evidence of lldEvidence(detail).slice(0,12)){const button=node('button','quiet',`${side} · ${evidence.path}:${evidence.start_line}-${evidence.end_line} · ${short(evidence.commit)}`);button.type='button';button.addEventListener('click',async()=>{activeLldEvidence=evidence;lldPinned=true;lldNode=null;const token=++lldSourceRequest;await workspace.loadLldSource(evidence);if(token===lldSourceRequest)renderLldInspector(workspace.state.lldDetail);});evidenceList.append(button);}container.append(evidenceList);
      const actions=node('div','hld-actions');const pin=node('button','quiet');pin.type='button';pin.id='lld-source-pin';pin.addEventListener('click',()=>{lldPinned=!lldPinned;pin.textContent=lldPinned?'Unpin source':'Pin source';rememberRoute();});actions.append(pin);container.append(actions);
      const pair=node('div','lld-source-pair');pair.id='lld-source-pair';container.append(pair);
      const status=node('p','status');status.id='lld-source-status';status.setAttribute('role','status');container.append(status);const source=node('pre','hld-source-text');source.id='lld-source-text';source.hidden=true;source.append(node('code'));container.append(source);
    }
    const source=workspace.state.lldSource;
    $('lld-source-pin').textContent=lldPinned?'Unpin source':'Pin source';$('lld-source-pin').disabled=!source;
    $('lld-source-status').textContent=workspace.state.loading.has('lld-source')?'Loading exact source evidence…':source?`${source.path}:${source.start_line}-${source.end_line} · exact commit ${source.commit} · blob ${source.blob_id}${source.truncated?' · truncated':''}`:workspace.state.lldError?`Source unavailable: ${workspace.state.lldError}`:'Hover or focus a flow box for code. Select it to pin the exact evidence.';
    $('lld-source-text').hidden=!source;if(source)$('lld-source-text').firstElementChild.textContent=source.text;
    lldSourcePairView.render($('lld-source-pair'),workspace.state.lldSourceComparison,payload);
    $('lld-acceptance').hidden=!payload.can_accept;$('lld-confirm').disabled=!payload.can_accept || !workspace.state.lldPreview;
    $('lld-confirm-side').querySelector('[value="baseline"]').disabled=!payload.baseline;$('lld-confirm-side').querySelector('[value="proposal"]').disabled=!payload.proposal;if(!payload[$('lld-confirm-side').value])$('lld-confirm-side').value=payload.proposal?'proposal':'baseline';
  }

  function clearHldView() {++architectureGeneration;++hldSourceGeneration;lastHldKey='';hldCorrections=[];hldCorrectionDesignId=null;clearHldEvidence(document);}

  function clearLldView() {
    ++architectureGeneration;++lldSourceRequest;clearTimeout(lldSourceTimer);lastLldKey='';lastLldInspectorKey='';lldNode=null;lldPinned=false;activeLldEvidence=null;lldActiveSourcePromise=null;lldActiveSourceKey=null;lldInvariantId=null;lldCorrespondenceId=null;lldBaselineEntry=null;lldParentHld=null;lldComponent=null;lldElement=null;lldDetail=null;lldQuery='';
    lldCatalogView.clear();lldSourcePairView.clear();clearLldEvidence(document);
  }

  function renderHld(state) {
    if (designLevel !== 'hld') return;
    if (renderedHldReview !== state.reviewId) { renderedHldReview=state.reviewId; lastHldKey=''; $('hld-confirmation-preview').hidden=true; $('hld-confirm').disabled=true; $('hld-confirmation-status').textContent='Preview is required before confirmation.'; }
    const payload = state.hld, loading = state.loading.has('hld');
    if(payload && hldMode==='accepted'&&!payload.accepted)hldMode='combined';
    $('hld-reload').disabled=loading;
    $('hld-summary').textContent = loading ? 'Loading commit-bound HLD evidence…' : state.hldError ? `HLD unavailable: ${state.hldError}. Reload after confirming repository context.` : !payload ? 'No generated HLD for this review. Reload after confirming repository context.' : hldOutcome(payload.delta);
    $('hld-accepted').disabled=!payload?.accepted;
    const exportAllowed=hldExportAllowed(payload,hldMode);
    $('hld-source').disabled=!exportAllowed;
    $('hld-export').disabled=!exportAllowed;
    for(const id of ['hld-source','hld-export'])$(id).title=exportAllowed?'':'Export requires permission from both the stored design policy and current scope.';
    if(publicStatic){$('hld-export').disabled=true;$('hld-export').title='Only the published full review Markdown is available.';}
    $('hld-acceptance').hidden=!payload?.can_accept;
    $('hld-confirm').disabled=!payload?.can_accept || !state.hldPreview;
    if (!payload) { clearHldView();return; }
    if(!exportAllowed){$('hld-mermaid-source').hidden=true;$('hld-mermaid-source').firstElementChild.textContent='';}
    if (hldMode==='accepted' && !payload.accepted) hldMode='combined';
    for (const mode of ['combined','compare','baseline','proposal','accepted']) { const button=$('hld-'+mode); button.classList.toggle('active',hldMode===mode); button.setAttribute('aria-pressed',String(hldMode===mode)); }
    $('hld-identity').textContent=`baseline ${short(payload.baseline?.snapshot)} → proposal ${short(payload.proposal?.snapshot)} · scope ${short(payload.baseline?.scope_version)} · ${publicStatic?'published static design':hldMode==='accepted'?'accepted as-built':'generated design'}`;
    $('hld-full-identity').textContent=`baseline commit ${payload.baseline?.snapshot || 'unavailable'} · proposal commit ${payload.proposal?.snapshot || 'unavailable'} · scope ${payload.baseline?.scope_version || 'unavailable'} · baseline design ${payload.baseline?.id || 'unavailable'} · proposal design ${payload.proposal?.id || 'unavailable'} · accepted as-built design ${payload.accepted?.id || 'none'} · accepted intended design ${payload.accepted_intended?.id || 'none'}`;
    $('hld-drift-status').textContent = publicStatic?'Read-only public snapshot. Accepted design records are not published.':(payload.accepted_drift ? `${hldOutcome(payload.accepted_drift)} before this PR · accepted ${short(payload.accepted?.snapshot)} → baseline ${short(payload.baseline?.snapshot)}` : payload.accepted ? 'Accepted as-built design drift has not been determined.' : 'No accepted as-built HLD baseline.')+(payload.accepted_intended?' · Accepted intended design is stored separately from observed code.':'');
    renderHldCoverage(payload);
    const comparison=hldComparison(payload,hldMode==='compare'?'combined':hldMode), elements=comparison.elements;
    if (!elements.some(element=>element.id===hldSelection)) hldSelection=elements[0]?.id || null;
    const list=$('hld-element-list'); list.replaceChildren();
    for (const element of elements) { const button=node('button','hld-element'); button.type='button'; button.setAttribute('aria-current',String(element.id===hldSelection)); button.append(node('span','',element.name),node('span','badge',comparison.status.get(element.id)||element.kind)); button.addEventListener('click',()=>selectHldElement(element.id)); list.append(button); }
    renderHldInspector(payload);
    updateHldConfirmationElements(payload);
    const modes=hldMode==='compare'?['baseline','proposal']:[hldMode];
    $('hld-mermaid-source').firstElementChild.textContent=exportAllowed?modes.map(mode=>`## ${mode}\n\n\`\`\`mermaid\n${hldMermaid(payload,mode)}\n\`\`\``).join('\n\n'):'';
    const key=[state.reviewId,hldMode,payload.baseline?.id,payload.proposal?.id,payload.accepted?.id,payload.delta?.id].join('|');
    if (lastHldKey!==key) { lastHldKey=key; renderHldDiagrams(payload); }
  }

  function renderHldCoverage(payload) {
    const container=$('hld-coverage'); container.replaceChildren();
    const selectedVersion=['accepted','baseline','proposal'].includes(hldMode)?payload[hldMode]:null;
    const coverage=selectedVersion?.coverage || payload.delta?.coverage || payload.proposal?.coverage || {};
    const states=(selectedVersion?[[hldMode,selectedVersion]]:[['baseline',payload.baseline],['proposal',payload.proposal]]).filter(([,version])=>version).map(([label,version])=>`${label}: ${publicStatic?'published static evidence':version.review_state}`).join(' · ');
    container.append(node('p','',states));
    const counts=['indexed','unsupported','failed','deferred'].filter(key=>Number.isFinite(coverage[key])).map(key=>`${coverage[key]} ${key}`).join(' · ');
    if (counts) container.append(node('p','',counts));
    if(coverage.profile)container.append(node('p','',coverage.profile));
    if (coverage.complete!==true || payload.delta?.outcome==='incomplete') container.append(node('p','warning','Coverage is partial. Unresolved or out-of-scope interactions can still affect system behavior.'));
    const gaps=[...new Set([...(coverage.gaps || []),...(payload.baseline?.coverage?.gaps || []),...(payload.proposal?.coverage?.gaps || [])])].slice(0,20);
    for (const gap of gaps) container.append(node('p','',String(gap)));
    const omitted=hldComparison(payload,hldMode==='compare'?'combined':hldMode).omitted;
    if (omitted) container.append(node('p','warning',`${omitted} design items omitted from this bounded view. Inspect the authorized export for the complete generated record.`));
    if (payload.generation_enabled===false) container.append(node('p','warning','New HLD generation is disabled. This is the authorized stored design history.'));
  }

  function selectHldElement(id) {
    hldSelection=id; ++hldSourceGeneration;
    renderHld(workspace.state);
    $('hld-diagrams').querySelectorAll('g.node[data-design-element]').forEach(element=>{element.classList.toggle('selected',element.dataset.designElement===id);element.setAttribute('aria-current',String(element.dataset.designElement===id));});
    $('hld-element-list').querySelector('[aria-current="true"]')?.focus();
    rememberRoute();
  }

  async function renderHldDiagrams(payload) {
    const generation=++architectureGeneration;
    diagramObservers.splice(0).forEach(observer=>observer.disconnect());
    const modes=hldMode==='compare'?['baseline','proposal']:[hldMode], container=$('hld-diagrams'); container.replaceChildren(); container.classList.toggle('compare',modes.length===2);
    const sources=modes.map(mode=>({mode,source:hldMermaid(payload,mode)}));
    $('hld-mermaid-source').firstElementChild.textContent=hldExportAllowed(payload,hldMode)?sources.map(item=>`## ${item.mode}\n\n\`\`\`mermaid\n${item.source}\n\`\`\``).join('\n\n'):'';
    try {
      const renderer=await ensureMermaid(); if (generation!==architectureGeneration || designLevel!=='hld') return;
      for (const item of sources) {
        await renderer.parse(item.source); const rendered=await renderer.render(`csb-hld-${generation}-${item.mode}`,item.source); if (generation!==architectureGeneration || designLevel!=='hld') return;
        const frame=node('section','architecture-frame'); frame.append(node('h4','',item.mode==='combined'?'HLD PR diff':item.mode));
        const host=node('div','architecture-svg'); host.innerHTML=rendered.svg; frame.append(host); container.append(frame);
        const svg=host.querySelector('svg'); if(svg)svg.setAttribute('aria-label',`${item.mode} high-level design diagram`);
        mountDiagramViewport(frame,host,svg,{initialMode:'overview'});
        const elements=hldDiagramElements(payload,item.mode);
        host.querySelectorAll('g.node').forEach(graphNode=>{const match=graphNode.id.match(/-flowchart-hld(\d+)-/); const element=match?elements[Number(match[1])]:null; if(!element)return; graphNode.dataset.designElement=element.id;graphNode.classList.toggle('selected',element.id===hldSelection);graphNode.setAttribute('aria-current',String(element.id===hldSelection));graphNode.setAttribute('tabindex','0'); graphNode.setAttribute('role','button'); graphNode.setAttribute('aria-label',`Inspect ${element.name}, ${element.kind}`); const inspect=()=>selectHldElement(element.id); graphNode.addEventListener('click',inspect); graphNode.addEventListener('keydown',event=>{if(event.key==='Enter'||event.key===' '){event.preventDefault();inspect();}});});
      }
    } catch(error) { if(generation!==architectureGeneration)return; lastHldKey=''; container.replaceChildren(node('p','status',`Diagram unavailable: ${error.message}. Component facts and authorized source evidence remain available below.`)); }
  }

  function appendHldList(container, title, values) {
    container.append(node('h4','',title));
    if (!values.length) { container.append(node('p','status','Unknown or not established by current evidence.')); return; }
    const list=node('ul'); for(const value of values.slice(0,50))list.append(node('li','',String(value))); container.append(list);
    if(values.length>50)container.append(node('p','status',`${values.length-50} additional entries omitted from this bounded section.`));
  }

  function renderHldInspector(payload) {
    const container=$('hld-inspector'); container.replaceChildren();
    const baseline=(hldMode==='accepted'?payload.accepted:payload.baseline)?.elements?.find(item=>item.id===hldSelection), proposal=payload.proposal?.elements?.find(item=>item.id===hldSelection);
    const selected=(hldMode==='baseline'||hldMode==='accepted'?baseline:proposal||baseline);
    if(!selected){container.append(node('p','status','Select a component to inspect its authorized design evidence.'));return;}
    container.append(node('h3','',selected.name),node('p','status',`${selected.kind} · ${selected.id}`));
    const parentVersion=hldMode==='accepted'?payload.accepted:hldMode==='baseline'?payload.baseline:proposal?payload.proposal:payload.baseline;
    const parentComponent=selected.kind==='component'?selected:parentVersion?.elements.find(element=>element.id===selected.component_id&&element.kind==='component');
    if(parentComponent&&parentVersion){const zoom=node('button','quiet',selected.kind==='component'?'Zoom to LLD':'Zoom to owning component LLD');zoom.type='button';zoom.id='hld-zoom-lld';zoom.addEventListener('click',()=>zoomHldToLld(parentComponent.id,parentVersion));container.append(zoom);}
    const facts=node('div','hld-facts');
    for(const [label,element] of hldMode==='accepted'?[['Accepted',baseline]]:hldMode==='baseline'?[['Baseline',baseline]]:hldMode==='proposal'?[['Proposal',proposal]]:[['Baseline',baseline],['Proposal',proposal]]) {
      const section=node('section');section.append(node('strong','',label));
      if(!element)section.append(node('p','',label==='Baseline'?'Added in proposal.':'Removed from proposal.'));
      else { section.append(node('p','',element.responsibility||'Responsibility not established.'),node('p','mono',(element.roots||[]).join(', ')||'Owning paths unknown.')); appendHldList(section,'Observed facts',(element.facts||[]).filter(fact=>fact.resolution!=='user_assertion').map(fact=>`${String(fact.kind).replaceAll('_',' ')}: ${fact.value} · ${fact.resolution}`)); appendHldList(section,'User assertions',[...(element.user_assertions||[]).map(value=>typeof value==='string'?value:JSON.stringify(value)),...(element.facts||[]).filter(fact=>fact.resolution==='user_assertion').map(fact=>`${fact.kind}: ${fact.value}`)]); }
      facts.append(section);
    }
    container.append(facts);
    const versions=hldMode==='accepted'?[payload.accepted]:hldMode==='baseline'?[payload.baseline]:hldMode==='proposal'?[payload.proposal]:[payload.baseline,payload.proposal];
    const relations=new Map();for(const version of versions)for(const relation of version?.relations||[])if(relation.from===hldSelection||relation.to===hldSelection)relations.set(`${version.id}\0${relation.id}`,{relation,version});
    appendHldList(container,'Interactions and contracts',[...relations.values()].map(({relation,version})=>`${version===payload.baseline?'Baseline':version===payload.proposal?'Proposal':'Accepted'}: ${version.elements.find(item=>item.id===relation.from)?.name||relation.from} → ${version.elements.find(item=>item.id===relation.to)?.name||relation.to} · ${relation.kind} · ${relation.protocol||'protocol unknown'} · ${relation.synchrony||'synchrony unknown'} · ${relation.resolution}`));
    const contracts=new Set(versions.flatMap(version=>version?.elements||[]).filter(element=>element.kind==='contract'&&element.component_id===hldSelection).map(element=>element.id));
    const changes=(payload.delta?.changes||[]).filter(change=>change.element_id===hldSelection||contracts.has(change.element_id)||[...relations.values()].some(item=>item.relation.id===change.relation_id));
    appendHldList(container,'PR design changes',changes.map(change=>`${change.kind}: ${change.reason}`));
    appendHldList(container,'Possible downstream impact',(payload.delta?.possible_impacts||[]).filter(impact=>impact.component_id===hldSelection||(impact.via_relation_ids||[]).some(id=>[...relations.values()].some(item=>item.relation.id===id))).map(impact=>`${impact.reason} · requires reviewer verification`));
    appendHldList(container,'Review questions',(payload.delta?.questions||[]).map(question=>typeof question==='string'?question:question.question||question.reason||JSON.stringify(question)));
    appendHldList(container,'Architectural constraint findings',(payload.delta?.findings||[]).map(finding=>`${finding.message} · ${finding.resolution} · constraint ${finding.constraint_id}`));
    appendHldList(container,'Selected design intent and decisions',versions.flatMap(version=>(version?.decision_records||[]).filter(decision=>decision.component_id===hldSelection).map(decision=>`${version===payload.baseline?'Baseline':version===payload.proposal?'Proposal':'Accepted'}: ${decision.intent} · ${decision.resolution} · ${decision.verification} · document ${decision.document_candidate_id}`)));
    appendHldList(container,'Unknowns',[...new Set([...(baseline?.unknowns||[]),...(proposal?.unknowns||[])])]);
    container.append(node('h4','','Exact source evidence')); const evidenceList=node('div','hld-evidence');
    const evidencePage=hldEvidenceCollection(payload,hldSelection,hldMode==='compare'?'combined':hldMode),evidence=evidencePage.items;
    for(const item of evidence){const button=node('button','quiet',`${item.path}:${item.start_line}-${item.end_line} · ${short(item.commit)} · ${item.relationship_type} · ${item.resolution}`);button.type='button';button.addEventListener('click',()=>inspectHldEvidence(item));evidenceList.append(button);}
    if(!evidence.length)evidenceList.append(node('p','status','No source span supports this element. User assertions are separate from code facts.'));
    if(evidencePage.omitted)evidenceList.append(node('p','status',`${evidencePage.omitted} additional source spans omitted from this bounded view.`));
    container.append(evidenceList);
    const sourceStatus=node('p','status');sourceStatus.id='hld-source-status';sourceStatus.setAttribute('role','status');container.append(sourceStatus);
    const source=node('pre','hld-source-text');source.id='hld-source-text';source.hidden=true;source.append(node('code'));container.append(source);
  }

  async function inspectHldEvidence(evidence) {
    const generation=++hldSourceGeneration, reviewId=workspace.state.reviewId, requestGeneration=workspace.generation;
    $('hld-source-status').textContent=`Loading exact ${short(evidence.commit)} source evidence…`; $('hld-source-text').hidden=true;
    try {
      const query=new URLSearchParams({design_id:evidence.design_id,evidence_id:evidence.id});
      const source=await workspace.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/hld/source?${query}`,requestGeneration);
      if(generation!==hldSourceGeneration||reviewId!==workspace.state.reviewId||designLevel!=='hld')return;
      if(source.review_id!==reviewId||source.commit!==evidence.commit||source.path!==evidence.path||source.blob_id!==evidence.blob||source.start_line!==evidence.start_line||!Number.isSafeInteger(source.end_line)||source.end_line<source.start_line||source.end_line>evidence.end_line||typeof source.text!=='string')throw new Error('Source evidence does not match the selected design snapshot');
      $('hld-source-status').textContent=`${source.path}:${source.start_line}-${source.end_line} · exact commit ${source.commit} · blob ${evidence.blob}`;
      $('hld-source-text').firstElementChild.textContent=source.text; $('hld-source-text').hidden=false;
    } catch(error) { if(generation!==hldSourceGeneration)return; if([401,403].includes(error.status)){renderHld(workspace.state);return;} $('hld-source-status').textContent=`Source unavailable: ${error.message}. This evidence remains pinned to ${short(evidence.commit)}; another revision is never substituted.`; }
  }

  function updateHldConfirmationElements(payload) {
    const version=payload[$('hld-confirm-design').value], key=version?.id||'';
    if($('hld-correction-element').dataset.designId!==key){$('hld-correction-element').dataset.designId=key;updateHldCorrectionChoices();}
    if(hldCorrectionDesignId!==key){hldCorrectionDesignId=key;hldCorrections=[];$('hld-corrections-list').replaceChildren();$('hld-responsibility').value='';$('hld-known-gaps').checked=false;invalidateHldConfirmation();}
  }

  function updateHldCorrectionChoices() {
    const version=workspace.state.hld?.[$('hld-confirm-design').value],kind=$('hld-correction-kind').value, select=$('hld-correction-element'), previous=select.value;
    select.replaceChildren();$('hld-related-elements').replaceChildren();$('hld-flow-source').replaceChildren();$('hld-flow-target').replaceChildren();
    const targetKind=kind==='contract_ownership'?'contract':kind==='data_ownership'?'schema':'component';
    const relatedKind=kind==='constraint'&&$('hld-constraint-kind').value==='required_contract'?'contract':'component';
    for(const element of version?.elements||[]) {
      if(element.kind===targetKind||kind==='correspondence'){const option=node('option','',`${element.name} · ${element.kind}`);option.value=element.id;select.append(option);}
      if(element.kind===relatedKind){const option=node('option','',element.name);option.value=element.id;$('hld-related-elements').append(option);}
      if(element.kind==='component'){for(const field of ['hld-flow-source','hld-flow-target']){const option=node('option','',element.name);option.value=element.id;$(field).append(option);}}
    }
    if([...select.options].some(option=>option.value===previous))select.value=previous;
    $('hld-add-correction').disabled=!select.value;
    $('hld-boundary-field').hidden=kind!=='boundary';$('hld-constraint-field').hidden=kind!=='constraint';$('hld-correspondence-field').hidden=kind!=='correspondence';
    $('hld-target-field').hidden=kind==='major_flow';$('hld-related-field').hidden=kind==='major_flow'||kind==='correspondence'||kind==='document_reference';$('hld-flow-source-field').hidden=kind!=='major_flow';$('hld-flow-target-field').hidden=kind!=='major_flow';$('hld-assertion-field').hidden=kind==='correspondence';
    $('hld-correspondence-source-field').hidden=kind!=='correspondence';$('hld-correspondence-target-field').hidden=kind!=='correspondence';
    $('hld-document-field').hidden=kind!=='document_reference';$('hld-document-evidence').replaceChildren();
    for(const candidate of version?.document_candidates||[]){const evidence=version.evidence.find(item=>item.id===candidate.evidence_id);if(!evidence)continue;const option=node('option','',`${candidate.title} · ${evidence.path}:${evidence.start_line}-${evidence.end_line} · ${short(evidence.commit)}`);option.value=candidate.evidence_id;$('hld-document-evidence').append(option);}
    if(kind==='document_reference'&&!$('hld-document-evidence').value)$('hld-add-correction').disabled=true;
    if(kind==='correspondence')updateHldCorrespondenceChoices();
  }

  function updateHldCorrespondenceChoices() {
    const side=$('hld-confirm-design').value,version=workspace.state.hld?.[side],reference=workspace.state.hld?.[side==='baseline'?'proposal':'baseline'];
    const target=version?.elements.find(element=>element.id===$('hld-correction-element').value);
    const sources=$('hld-correspondence-sources'),targets=$('hld-correspondence-targets');sources.replaceChildren();targets.replaceChildren();
    for(const element of reference?.elements||[])if(element.kind===target?.kind){const option=node('option','',element.name);option.value=element.id;sources.append(option);}
    for(const element of version?.elements||[])if(element.kind===target?.kind){const option=node('option','',element.name);option.value=element.id;option.selected=element.id===target.id;targets.append(option);}
    $('hld-correspondence-source-identity').textContent=`source ${reference?.snapshot||'unavailable'} · ${reference?.id||'unavailable'}`;
    $('hld-correspondence-target-identity').textContent=`target ${version?.snapshot||'unavailable'} · ${version?.id||'unavailable'}`;
  }

  function hldConfirmationRequest() {
    const payload=workspace.state.hld, version=payload?.[$('hld-confirm-design').value];
    if(!version)throw new Error('Select a generated design first');
    if($('hld-responsibility').value.trim())throw new Error('Add the current correction before previewing, or clear the assertion field');
    return {design_id:version.id,interpretation:$('hld-interpretation').value,corrections:hldCorrections};
  }

  function invalidateHldConfirmation() { workspace.invalidateHldPreview(); $('hld-confirm').disabled=true; $('hld-confirmation-preview').hidden=true; $('hld-confirmation-status').textContent='Design selection or corrections changed. Preview again before confirmation.'; }

  function addHldCorrection() {
    try {
      if(hldCorrections.length>=100)throw new Error('This review supports at most 100 corrections');
      const version=workspace.state.hld?.[$('hld-confirm-design').value],kind=$('hld-correction-kind').value;
      const reference=workspace.state.hld?.[$('hld-confirm-design').value==='baseline'?'proposal':'baseline'];
      const correction=hldCorrection({id:`correction-${crypto.randomUUID()}`,kind,targetId:kind==='major_flow'?$('hld-flow-source').value:$('hld-correction-element').value,value:kind==='correspondence'?$('hld-correspondence-kind').value:$('hld-responsibility').value,paths:kind==='boundary'?parseScopePaths($('hld-boundary-paths').value):[],relatedIds:kind==='major_flow'?[$('hld-flow-source').value,$('hld-flow-target').value]:kind==='document_reference'?[]:[...$(kind==='correspondence'?'hld-correspondence-sources':'hld-related-elements').selectedOptions].map(option=>option.value),constraintKind:$('hld-constraint-kind').value,referenceDesignId:kind==='correspondence'?reference?.id:null,correspondenceTargets:kind==='correspondence'?[...$('hld-correspondence-targets').selectedOptions].map(option=>option.value):[],documentEvidenceId:kind==='document_reference'?$('hld-document-evidence').value:null},version,reference);
      hldCorrections.push(correction);$('hld-responsibility').value='';invalidateHldConfirmation();
      const names=(ids,design)=>ids.map(id=>design?.elements.find(item=>item.id===id)?.name||id).join(', ');
      const relationship=correction.kind==='correspondence'?`${names(correction.related_ids,reference)} → ${names(correction.correspondence_targets,version)}`:correction.kind==='major_flow'?`${names([correction.related_ids[0]],version)} → ${names([correction.related_ids[1]],version)}`:names([correction.target_id],version)+(correction.related_ids.length?' → '+names(correction.related_ids,version):'');
      const row=node('p','mono',`${correction.kind} · ${relationship} · ${correction.value}${correction.paths.length?' · '+correction.paths.join(', '):''}`);$('hld-corrections-list').append(row);$('hld-confirmation-status').textContent=`${hldCorrections.length} correction${hldCorrections.length===1?'':'s'} queued. Preview the exact interpretation before accepting.`;
    } catch(error){$('hld-confirmation-status').textContent=error.message;}
  }

  function clearSourceInspector(message='Focus a symbol to load its bounded source span.') {
    if (pinned) return;
    workspace.state.source=null;workspace.state.sourceComparison=null;$('source-title').textContent='Code diff';$('source-meta').textContent=message;
    $('baseline-source-label').textContent='Baseline';$('proposal-source-label').textContent='Proposal';
    $('source-baseline').firstElementChild.textContent='No baseline source selected.';$('source-proposal').firstElementChild.textContent='No proposal source selected.';$('source-unified').firstElementChild.textContent='No source selected.';$('pin').disabled=true;$('pin').textContent='Pin';
  }

  $('session-form').addEventListener('submit', event => { event.preventDefault(); token = $('session-token').value; $('session-token').value = ''; loadInbox(); });
  document.querySelector('.skip-link').addEventListener('click', event => {
    event.preventDefault();
    const target = $('review-content').hidden ? $('empty') : $('review-content');
    target.focus(); target.scrollIntoView({block:'start'});
  });
  $('file-search').addEventListener('input', () => renderChangedFiles(workspace.state));
  if (publicStatic) {
    $('session-form').hidden=true;$('session-token').value='';token='';$('onboarding-link').hidden=true;
    for(const id of ['baseline-revision','proposal-revision','compare-revisions','scope-primary-include','scope-primary-exclude','add-scope-context','preview-scope','save-scope','save-handoff','handoff-reviewer']){$(id).disabled=true;$(id).title='Read-only published comparison. Request a new analysis to change this input.';}
    $('question-visibility').value='private';$('question-visibility').disabled=true;$('question-visibility').querySelector('[value="private"]').textContent='Page memory';$('review-question').maxLength=4000;
    $('continuity-status').textContent='Review ticks, notes and bookmarks stay in this page memory. Reload clears them. They do not grant merge approval.';
    const exportReport=node('button','quiet','Export published review');exportReport.type='button';exportReport.id='public-export-report';exportReport.disabled=true;$('open-feedback').before(exportReport);
    exportReport.addEventListener('click',async()=>{if(!workspace.state.details?.capabilities.includes('export'))return;const generation=workspace.generation,reviewId=workspace.state.reviewId;try{const value=await workspace.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/export?view=diff`,generation);if(generation!==workspace.generation||value.review_id!==reviewId)return;const url=URL.createObjectURL(new Blob([value.markdown],{type:'text/markdown'})),link=node('a');link.href=url;link.download=`codeshouldbehave-public-${reviewId.slice(0,12)}.md`;link.click();URL.revokeObjectURL(url);}catch(error){$('inbox-status').textContent=error.message;}});
    loadInbox();
  } else if (token) { $('session-form').hidden = true; loadInbox(); }
  $('open-feedback').addEventListener('click',()=>{clearFeedback();feedbackDraft.record=betaFeedbackRecord({category:'confusing',layout,side:$('side').value,hasReview:Boolean(workspace.state.reviewId),feedbackId:typeof crypto?.randomUUID === 'function' ? crypto.randomUUID() : `local-${Date.now()}`,createdAt:new Date().toISOString()});previewFeedback();$('feedback-dialog').showModal();});
  $('feedback-dialog').addEventListener('close',()=>{if(!$('feedback-dialog').open)clearFeedback();});
  $('feedback-form').addEventListener('submit',event=>event.preventDefault());
  $('cancel-feedback').addEventListener('click',()=>$('feedback-dialog').close());
  $('preview-feedback').addEventListener('click',previewFeedback);
  $('feedback-category').addEventListener('change',invalidateFeedback);
  $('feedback-description').addEventListener('input',invalidateFeedback);
  for (const id of ['feedback-triage-status','feedback-triage-severity','feedback-duplicate','feedback-resolution']) $(id).addEventListener('input',invalidateFeedback);
  $('feedback-image-consent').addEventListener('change',()=>{feedbackDraft.setConsent($('feedback-image-consent').checked);invalidateFeedback();});
  $('remove-feedback-image').addEventListener('click',()=>{feedbackDraft.imageGeneration++;feedbackDraft.processing=false;feedbackDraft.setScreenshot(null);$('feedback-image').value='';$('feedback-image-consent').checked=false;$('feedback-image-content').removeAttribute('src');$('feedback-image-preview').hidden=true;invalidateFeedback();});
  $('feedback-image').addEventListener('change',async()=>{
    const generation=++feedbackDraft.imageGeneration;
    feedbackDraft.processing=true;
    feedbackDraft.setScreenshot(null);$('feedback-image-consent').checked=false;$('feedback-image-content').removeAttribute('src');$('feedback-image-preview').hidden=true;invalidateFeedback();
    try { const image=await prepareFeedbackImage($('feedback-image').files[0]);if(generation!==feedbackDraft.imageGeneration)return;feedbackDraft.setScreenshot(image);$('feedback-image-content').src=image.data_url;$('feedback-image-preview').hidden=false;$('feedback-status').textContent='Inspect the image and confirm inclusion, then refresh the preview.'; }
    catch(error){if(generation===feedbackDraft.imageGeneration)$('feedback-status').textContent=error.message;}
    finally{if(generation===feedbackDraft.imageGeneration)feedbackDraft.processing=false;}
  });
  $('feedback-import').addEventListener('change',async()=>{
    const file=$('feedback-import').files[0],generation=++feedbackDraft.imageGeneration;feedbackDraft.processing=true;invalidateFeedback();
    try {
      if(!file || file.size>4500000)throw new Error('Feedback JSON exceeds the size limit');
      const record=importBetaFeedback(await file.text());if(generation!==feedbackDraft.imageGeneration)return;
      let image=null;
      if(record.screenshot){const bytes=Uint8Array.from(atob(record.screenshot.data_url.split(',')[1]),char=>char.charCodeAt(0));image=await prepareFeedbackImage(new Blob([bytes],{type:'image/png'}));if(generation!==feedbackDraft.imageGeneration)return;}
      feedbackDraft.record=record;feedbackDraft.setScreenshot(image);$('feedback-category').value=record.category;$('feedback-description').value=record.description;$('feedback-triage-status').value=record.status;$('feedback-triage-severity').value=record.severity;$('feedback-duplicate').value=record.duplicate_of||'';$('feedback-resolution').value=record.resolution_commit||'';
      $('feedback-image-consent').checked=false;$('feedback-image-preview').hidden=!image;if(image)$('feedback-image-content').src=image.data_url;else $('feedback-image-content').removeAttribute('src');
      $('feedback-status').textContent=`Imported ${record.feedback_id}. Inspect the report and any image, apply triage, then preview.`;
    }catch(error){if(generation===feedbackDraft.imageGeneration)$('feedback-status').textContent=error.message;}
    finally{if(generation===feedbackDraft.imageGeneration)feedbackDraft.processing=false;}
  });
  $('apply-feedback-triage').addEventListener('click',()=>{
    try { const fields={...buildFeedbackRecord(),status:$('feedback-triage-status').value,severity:$('feedback-triage-severity').value,duplicateOf:$('feedback-duplicate').value||null,resolutionCommit:$('feedback-resolution').value||null};feedbackDraft.record=betaFeedbackRecord({...fields,screenshot:null});previewFeedback(); }
    catch(error){invalidateFeedback();$('feedback-status').textContent=error.message;}
  });
  $('new-feedback').addEventListener('click',()=>{clearFeedback();previewFeedback();});
  $('copy-feedback').addEventListener('click',async()=>{
    try{const record=feedbackDraft.exportRecord();await navigator.clipboard.writeText(JSON.stringify(record,null,2)+'\n');$('feedback-status').textContent=`Feedback ${record.feedback_id} copied to your clipboard. No external submission occurred.`;}
    catch(error){$('feedback-status').textContent=`Copy unavailable: ${error.message}. Use the local download instead.`;}
  });
  $('export-feedback').addEventListener('click',()=>{
    try{const record=feedbackDraft.exportRecord();const blob=new Blob([JSON.stringify(record,null,2)+'\n'],{type:'application/json'});const url=URL.createObjectURL(blob);const link=document.createElement('a');link.href=url;link.download=`csb-beta-feedback-${record.feedback_id}.json`;link.hidden=true;document.body.append(link);link.click();setTimeout(()=>{URL.revokeObjectURL(url);link.remove();},30000);$('feedback-status').textContent=`Local download requested for feedback ${record.feedback_id}. No external submission occurred.`;}
    catch(error){$('feedback-status').textContent=error.message;}
  });
  $('refresh').addEventListener('click', () => loadInbox());
  $('more-prs').addEventListener('click', () => loadInbox({append: true}));
  $('more-files').addEventListener('click', () => workspace.loadMoreFiles());
  $('toggle-file-reviewed').addEventListener('click',async()=>{const path=$('toggle-file-reviewed').dataset.path;if(!path)return;const current=existingAnnotation('reviewed',path);$('toggle-file-reviewed').disabled=true;try{await saveAnnotation({kind:'reviewed',targetKey:path,targetPath:path,status:current?.status==='active'?'resolved':'active'});}finally{$('toggle-file-reviewed').disabled=false;}});
  $('side').addEventListener('change', async () => { await workspace.expandComponent({side: $('side').value}); rememberRoute(); });
  $('compare-revisions').addEventListener('click', compareRevisions);
  $('add-scope-context').addEventListener('click', addScopeContext);
  $('preview-scope').addEventListener('click', previewScope);
  $('save-scope').addEventListener('click', saveScope);
  $('scope-primary-include').addEventListener('input', invalidateScopePreview);
  $('scope-primary-exclude').addEventListener('input', invalidateScopePreview);
  $('direction').addEventListener('click', () => { direction = direction === 'outgoing' ? 'incoming' : 'outgoing'; $('direction').textContent = direction[0].toUpperCase() + direction.slice(1); const selection = workspace.state.selection; if (selection?.kind === 'symbol') workspace.selectSymbol({...selection, side: $('side').value, direction}); });
  $('pin').addEventListener('click', () => { pinned = !pinned; $('pin').textContent = pinned ? 'Unpin' : 'Pin'; });
  $('layout-connected').addEventListener('click', () => setLayout('connected'));
  $('layout-source').addEventListener('click', () => setLayout('source'));
  $('layout-architecture').addEventListener('click', () => setLayout('architecture'));
  $('design-source-fallback').addEventListener('click', async () => {
    if(designLevel==='hld' && workspace.state.hld){const evidence=hldEvidenceFor(workspace.state.hld,hldSelection,hldMode==='compare'?'combined':hldMode);const file=(workspace.state.files?.items||[]).find(item=>evidence.some(source=>source.path===item.path));if(file){$('side').value='diff';await workspace.selectPath({path:file.path,side:'diff'});}else clearSourceInspector('This HLD component has no changed file in this PR. Inspect its exact source evidence in the HLD view.');}
    if(designLevel==='lld' && workspace.state.lldDetail){const detail=workspace.state.lldDetail.proposal || workspace.state.lldDetail.baseline,path=detail.entry.path,file=(workspace.state.files?.items || []).find(item=>item.path===path);if(file){$('side').value='diff';await workspace.selectPath({path,side:'diff'});}else clearSourceInspector('This LLD declaration has no changed file in this PR. Inspect its exact source evidence in the LLD view.');}
    setLayout('source');
  });
  $('diff-split').addEventListener('click',()=>setCodeDiffMode('split'));
  $('diff-unified').addEventListener('click',()=>setCodeDiffMode('unified'));
  for(const mode of ['combined','compare','baseline','proposal']) $('architecture-'+mode).addEventListener('click',()=>{architectureMode=mode;lastArchitectureKey='';for(const name of ['combined','compare','baseline','proposal']){$('architecture-'+name).classList.toggle('active',name===mode);$('architecture-'+name).setAttribute('aria-pressed',String(name===mode));}renderArchitectureImpact(workspace.state);});
  for(const level of ['structural','hld','lld'])$('design-level-'+level).addEventListener('click',()=>setDesignLevel(level));
  $('lld-open-hld').addEventListener('click',()=>setDesignLevel('hld'));
  $('lld-back-hld').addEventListener('click',()=>setDesignLevel('hld'));
  $('lld-back-catalog').addEventListener('click',()=>{lldElement=null;lldNode=null;lldPinned=false;lldBaselineEntry=null;lldCorrespondenceId=null;lldDetail='module';activeLldEvidence=null;++lldSourceRequest;workspace.invalidateLld({catalog:false});workspace.state.lldBaselineEntry=null;renderLld(workspace.state);rememberRoute();});
  $('lld-filter').addEventListener('input',event=>{lldQuery=event.target.value;renderLldCatalog(workspace.state.lld);});
  $('lld-more').addEventListener('click',()=>loadSelectedLldCatalog({cursor:workspace.state.lld?.next_cursor}));
  $('lld-reload').addEventListener('click',async()=>{if(lldElement){await workspace.loadLldDetail(lldElement,{baselineEntry:lldBaselineEntry,correspondenceKind:lldCorrespondence});return;}await loadSelectedLldCatalog();});
  $('lld-load-history').addEventListener('click',()=>workspace.loadLldHistory(lldElement));
  for(const mode of ['combined','compare','baseline','proposal'])$('lld-'+mode).addEventListener('click',()=>{hldMode=mode;++architectureGeneration;lastLldKey='';++lldSourceRequest;++workspace.lldSourceGeneration;workspace.state.lldSource=null;renderLld(workspace.state);rememberRoute();});
  $('lld-mermaid').addEventListener('click',()=>{if(!workspace.state.lldDetail?.can_export && !workspace.state.lldDetail?.export_allowed)return;$('lld-mermaid-source').hidden=!$('lld-mermaid-source').hidden;});
  $('lld-export').addEventListener('click',async()=>{
    const payload=workspace.state.lldDetail || workspace.state.lld;if(!payload?.can_export && !payload?.export_allowed)return;
    const generation=workspace.generation,token=workspace.lldDetailGeneration,reviewId=workspace.state.reviewId,selection=workspace.state.lldSelection,button=$('lld-export');button.disabled=true;
    const query=new URLSearchParams({view:['baseline','proposal'].includes(hldMode)?hldMode:'diff',parent_hld:selection.parentHld});if(selection.component)query.set('component',selection.component);if(lldElement)query.set('entry',lldElement);
    if(lldBaselineEntry){query.set('baseline_entry',lldBaselineEntry);query.set('correspondence_kind',lldCorrespondence);}
    try{const result=await workspace.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/lld/export?${query}`,generation);if(token!==workspace.lldDetailGeneration || designLevel!=='lld')return;validateLldResponse(result,{reviewId,...selection,scopeVersion:payload.scope_version});if(typeof result.markdown!=='string'||result.markdown.length>2000000)throw new Error('Invalid LLD export');const url=URL.createObjectURL(new Blob([result.markdown],{type:'text/markdown'})),anchor=node('a');anchor.href=url;anchor.download='codeshouldbehave-lld.md';anchor.click();URL.revokeObjectURL(url);}
    catch(error){if(generation===workspace.generation && token===workspace.lldDetailGeneration)$('lld-summary').textContent=`Export unavailable: ${error.message}`;}
    finally{if(generation===workspace.generation)button.disabled=!Boolean((workspace.state.lldDetail || workspace.state.lld)?.can_export || (workspace.state.lldDetail || workspace.state.lld)?.export_allowed);}
  });
  function lldConfirmationRequest() {
    const payload=workspace.state.lldDetail,detail=payload?.[$('lld-confirm-side').value],statement=$('lld-invariant').value.trim();if(!detail)throw new Error('Select a retained implementation first');if(!statement && !lldBaselineEntry)throw new Error('State an invariant or choose an explicit baseline counterpart to record');
    const corrections=[];
    if(statement){if(!lldInvariantId)lldInvariantId=`lld-invariant-${crypto.randomUUID()}`;corrections.push({id:lldInvariantId,kind:'lld_invariant',target_id:detail.entry.id,value:statement,paths:[],related_ids:[],constraint_kind:null,reference_design_id:null,correspondence_targets:[]});}
    if(lldBaselineEntry){if(!payload.baseline || !payload.proposal || detail.id!==payload.proposal.id)throw new Error('Record correspondence on the proposal implementation');if(!lldCorrespondenceId)lldCorrespondenceId=`lld-correspondence-${crypto.randomUUID()}`;corrections.push({id:lldCorrespondenceId,kind:'correspondence',target_id:payload.proposal.entry.id,value:lldCorrespondence,paths:[],related_ids:[payload.baseline.entry.id],constraint_kind:null,reference_design_id:payload.baseline.id,correspondence_targets:[payload.proposal.entry.id]});}
    return {design_id:detail.id,entry:payload.entry_id,interpretation:$('lld-interpretation').value,corrections};
  }
  function invalidateLldConfirmation() {workspace.invalidateLldPreview();$('lld-confirm').disabled=true;$('lld-confirmation-preview').hidden=true;$('lld-confirmation-status').textContent='The assertion or consent changed. Preview again before confirmation.';}
  for(const id of ['lld-confirm-side','lld-interpretation','lld-invariant','lld-known-gaps','lld-activate-default'])$(id).addEventListener(id==='lld-invariant'?'input':'change',invalidateLldConfirmation);
  $('lld-preview-acceptance').addEventListener('click',async()=>{const button=$('lld-preview-acceptance'),generation=workspace.generation,token=workspace.lldDetailGeneration;button.disabled=true;$('lld-confirm').disabled=true;try{const preview=await workspace.previewLld(lldConfirmationRequest());if(generation!==workspace.generation||token!==workspace.lldDetailGeneration)return;$('lld-confirmation-preview').firstElementChild.textContent=JSON.stringify(preview,null,2);$('lld-confirmation-preview').hidden=false;$('lld-confirmation-status').textContent='Review the exact implementation fingerprint and assertion. Acceptance requires your explicit action.';$('lld-confirm').disabled=false;}catch(error){if(generation===workspace.generation&&token===workspace.lldDetailGeneration)$('lld-confirmation-status').textContent=`Assertion preview unavailable: ${error.message}`;}finally{if(generation===workspace.generation&&token===workspace.lldDetailGeneration)button.disabled=false;}});
  $('lld-confirm').addEventListener('click',async()=>{const generation=workspace.generation,token=workspace.lldDetailGeneration;$('lld-confirm').disabled=true;try{if(!$('lld-known-gaps').checked)throw new Error('Review and acknowledge the unresolved calls and local-flow gaps before accepting');await workspace.acceptLld(lldConfirmationRequest(),{knownGapsAccepted:true,activateDefault:$('lld-activate-default').checked});if(generation!==workspace.generation||token!==workspace.lldDetailGeneration)return;$('lld-confirmation-preview').hidden=true;$('lld-confirmation-status').textContent='Assertion accepted for this exact implementation and interpretation. Changed bodies require review again.';await workspace.loadLldDetail(lldElement,{baselineEntry:lldBaselineEntry,correspondenceKind:lldCorrespondence});}catch(error){if(generation===workspace.generation&&token===workspace.lldDetailGeneration){invalidateLldConfirmation();$('lld-confirmation-status').textContent=`LLD acceptance unavailable: ${error.message}`;}}});
  for(const mode of ['combined','compare','baseline','proposal','accepted'])$('hld-'+mode).addEventListener('click',()=>{hldMode=mode;lastHldKey='';++hldSourceGeneration;renderHld(workspace.state);rememberRoute();});
  $('hld-reload').addEventListener('click',()=>{workspace.state.hldDraft=null;invalidateHldConfirmation();workspace.loadHld();});
  $('hld-source').addEventListener('click',()=>{if(!hldExportAllowed(workspace.state.hld,hldMode))return;$('hld-mermaid-source').hidden=!$('hld-mermaid-source').hidden;});
  $('hld-export').addEventListener('click',async()=>{
    if(!hldExportAllowed(workspace.state.hld,hldMode))return;const reviewId=workspace.state.reviewId,generation=workspace.generation;const button=$('hld-export');button.disabled=true;
    try{const view=hldMode==='combined'||hldMode==='compare'?'diff':hldMode;const result=await workspace.fetch(`/v1/reviews/${encodeURIComponent(reviewId)}/hld/export?view=${encodeURIComponent(view)}`,generation);if(result.review_id!==reviewId||typeof result.markdown!=='string'||result.markdown.length>2000000)throw new Error('Invalid HLD export');const url=URL.createObjectURL(new Blob([result.markdown],{type:'text/markdown'}));const anchor=node('a');anchor.href=url;anchor.download=`codeshouldbehave-hld-${view}.md`;anchor.click();URL.revokeObjectURL(url);}
    catch(error){if(generation===workspace.generation)$('hld-summary').textContent=`Export unavailable: ${error.message}`;}
    finally{if(generation===workspace.generation)button.disabled=!hldExportAllowed(workspace.state.hld,hldMode);}
  });
  for(const id of ['hld-confirm-design','hld-interpretation','hld-correction-element','hld-correction-kind','hld-responsibility','hld-boundary-paths','hld-related-elements','hld-flow-source','hld-flow-target','hld-constraint-kind','hld-correspondence-kind','hld-correspondence-sources','hld-correspondence-targets','hld-document-evidence','hld-known-gaps','hld-activate-default'])$(id).addEventListener(id==='hld-responsibility'||id==='hld-boundary-paths'?'input':'change',()=>{invalidateHldConfirmation();if(id==='hld-confirm-design')updateHldConfirmationElements(workspace.state.hld||{});if(id==='hld-correction-kind'||id==='hld-constraint-kind')updateHldCorrectionChoices();if(id==='hld-correction-element'&&$('hld-correction-kind').value==='correspondence')updateHldCorrespondenceChoices();});
  $('hld-add-correction').addEventListener('click',addHldCorrection);
  $('hld-clear-corrections').addEventListener('click',()=>{hldCorrections=[];$('hld-corrections-list').replaceChildren();$('hld-responsibility').value='';invalidateHldConfirmation();});
  $('hld-preview-acceptance').addEventListener('click',async()=>{
    const button=$('hld-preview-acceptance'),generation=workspace.generation;button.disabled=true;$('hld-confirm').disabled=true;
    try{const preview=await workspace.previewHld(hldConfirmationRequest());$('hld-confirmation-preview').firstElementChild.textContent=JSON.stringify(preview,null,2);$('hld-confirmation-preview').hidden=false;$('hld-confirmation-status').textContent='Review the exact design, interpretation and corrections above. Acceptance requires your explicit action.';$('hld-confirm').disabled=false;}
    catch(error){if(generation===workspace.generation)$('hld-confirmation-status').textContent=`Confirmation preview unavailable: ${error.message}. Reload if the design changed.`;}
    finally{if(generation===workspace.generation)button.disabled=false;}
  });
  $('hld-confirm').addEventListener('click',async()=>{
    const button=$('hld-confirm'),generation=workspace.generation;button.disabled=true;
    try{const body=hldConfirmationRequest(),activateDefault=$('hld-activate-default').checked,knownGapsAccepted=$('hld-known-gaps').checked;if(!knownGapsAccepted)throw new Error('Review and acknowledge the coverage gaps and unresolved questions before accepting');await workspace.acceptHld(body,{activateDefault,knownGapsAccepted});hldCorrections=[];$('hld-corrections-list').replaceChildren();$('hld-responsibility').value='';$('hld-known-gaps').checked=false;$('hld-activate-default').checked=false;$('hld-confirmation-preview').hidden=true;$('hld-confirmation-status').textContent=body.interpretation==='intended'?(activateDefault?'Accepted intended HLD stored as the intended baseline, separately from observed as-built architecture.':'Accepted intended HLD stored. Defaults remain unchanged.'):(activateDefault?'Accepted as-built HLD stored and explicitly set as the default baseline.':'Accepted as-built HLD stored. The default baseline was not changed.');}
    catch(error){if(generation===workspace.generation){invalidateHldConfirmation();$('hld-confirmation-status').textContent=`HLD acceptance unavailable: ${error.message}. Refresh the design and preview again.`;}}
  });
  $('architecture-source').addEventListener('click',()=>{if(!(workspace.state.details?.capabilities || []).includes('export'))return;$('architecture-mermaid-source').hidden=!$('architecture-mermaid-source').hidden;$('architecture-source').textContent=$('architecture-mermaid-source').hidden?'Mermaid source':'Hide Mermaid source';});
  $('brief-action').addEventListener('click', () => {
    setLayout('connected');
    $('architecture-title').focus?.();
  });
  $('graph-search').addEventListener('input', event => { graphQuery = event.target.value; if (workspace.state.graph) renderGraphItems(workspace.state.graph, workspace.state); });
  $('edge-filter').addEventListener('change', event => { edgeKindFilter = event.target.value; if (workspace.state.graph) renderGraphItems(workspace.state.graph, workspace.state); });
  $('graph-root').addEventListener('click', async () => { graphQuery = ''; edgeKindFilter = ''; $('graph-search').value = ''; clearSourceInspector(); await workspace.expandComponent({side:$('side').value}); rememberRoute(); });
  $('reset-graph').addEventListener('click', async () => { graphQuery = ''; edgeKindFilter = ''; graphWindow = 40; $('graph-search').value = ''; clearSourceInspector('Source cleared when the graph view reset.'); await workspace.expandComponent({side:$('side').value}); rememberRoute(); });
  $('fit-selection').addEventListener('click', () => { const current = $('graph').querySelector('[aria-current="true"]'); if (current) { current.scrollIntoView({block:'center'}); current.focus(); } else $('graph').focus?.(); });
  $('pane-width').addEventListener('input', event => $('workspace-split').style.setProperty('--diagram-width', `${event.target.value}%`));
  $('graph').addEventListener('keydown', event => {
    if (!['ArrowDown','ArrowUp','Home','End'].includes(event.key)) return;
    const items=Array.from($('graph').querySelectorAll('button:not(:disabled)')); if (!items.length) return;
    const current=Math.max(0,items.indexOf(document.activeElement));
    const next=event.key==='Home'?0:event.key==='End'?items.length-1:event.key==='ArrowDown'?Math.min(items.length-1,current+1):Math.max(0,current-1);
    event.preventDefault(); items[next].focus();
  });
  $('mark-all-reviewed').addEventListener('click', async () => {
    const button = $('mark-all-reviewed'); button.disabled = true; $('continuity-status').textContent = 'Saving exact-snapshot inspection progress…';
    try { for (const file of workspace.state.files?.items || []) await saveAnnotation({kind:'reviewed',targetKey:file.path,targetPath:file.path}); $('continuity-status').textContent = 'All loaded files are marked reviewed. Merge approval remains separate.'; }
    catch (error) { $('continuity-status').textContent = error.message; }
  });
  $('bookmark-selection').addEventListener('click', async () => {
    const selection = workspace.state.selection; if (!selection) return;
    try { await saveAnnotation({kind:'bookmark',targetKey:selection.symbol || selection.path,targetPath:selection.path}); $('continuity-status').textContent = 'Bookmark saved to this exact proposal snapshot.'; }
    catch (error) { $('continuity-status').textContent = error.message; }
  });
  $('save-question').addEventListener('click', async () => {
    const body = $('review-question').value.trim(); if (!body) { $('continuity-status').textContent = 'Enter a reviewer question first.'; return; }
    const selection = workspace.state.selection; const key = selection?.symbol || selection?.path || `review:${workspace.state.reviewId}`;
    try { await saveAnnotation({kind:'question',targetKey:key,targetPath:selection?.path || null,body,visibility:$('question-visibility').value}); $('review-question').value=''; $('continuity-status').textContent = publicStatic?'Question saved in page memory. Reload clears it.':'Question saved. Shared visibility remains subject to current export permission.'; }
    catch (error) { $('continuity-status').textContent = error.message; }
  });
  $('save-handoff').addEventListener('click', async () => {
    const reviewer = $('handoff-reviewer').value.trim(); if (!reviewer) { $('continuity-status').textContent='Enter an authorized reviewer identity.'; return; }
    const selection=workspace.state.selection; const key=`handoff:${reviewer}:${selection?.symbol || selection?.path || workspace.state.reviewId}`;
    try { await saveAnnotation({kind:'handoff',targetKey:key,targetPath:selection?.path || null,body:reviewer,visibility:'shared'}); $('handoff-reviewer').value=''; $('continuity-status').textContent='Handoff saved with this exact review link for an authorized reviewer.'; }
    catch(error) { $('continuity-status').textContent=error.message; }
  });
  $('more-graph').addEventListener('click', () => {
    const graph = workspace.state.graph;
    const items = graphItems(graph);
    if (items?.length > graphWindow) { graphWindow += 40; renderGraphItems(graph, workspace.state); return; }
    const selection = workspace.state.selection;
    if (selection?.kind === 'path') workspace.selectPath({path: selection.path, cursor: graphCursor, side: $('side').value});
    else workspace.expandComponent({path: selection?.path, cursor: graphCursor, side: $('side').value});
  });
  document.addEventListener('keydown', event => { if (event.key === 'Escape' && pinned) { pinned = false; $('pin').textContent = 'Pin'; } });
  window.addEventListener('popstate', () => restoreRoute());
}
