"""Private HLD source and publication contracts with synthetic authority only."""
import copy
import io
import json
from pathlib import Path
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"scripts"))
import github_mvp_server as module


class HldSecurityTest(unittest.TestCase):
    def fixture(self,root):
        state=module.State(root,root)
        self.addCleanup(state._host_lock.close)
        state.repository_slug=lambda:"owner/repo"
        context={"session_id":"accepted","session_revision":8,"input_fingerprint":"a"*64,
                 "proposal_fingerprint":"b"*64,"accepted_scope_fingerprint":"c"*64,
                 "include":["src"],"exclude":["src/private"],"contexts":[],"export_allowed":False}
        state.active_context=lambda **kwargs:copy.deepcopy(context)
        access={"principal":"github-user:9","maintainer":True}
        revision_calls=[]
        state.app_auth=SimpleNamespace(access=lambda slug:dict(access),
            _verify_pull_revision=lambda *args:revision_calls.append(args))
        evidence={"id":"evidence-"+"d"*64,"commit":"e"*40,"path":"src/lib.rs","blob":"f"*40,
                  "start_line":2,"end_line":3}
        source={"evidence_id":evidence["id"],"commit":evidence["commit"],"path":evidence["path"],
                "blob_id":evidence["blob"],"start_line":2,"end_line":3,
                "text":"pub fn review() {\n}\n","truncated":False}
        design={"id":"design-"+"1"*64,"snapshot":"e"*40,"evidence":[evidence],"markdown":"# Observed HLD"}
        packet={"baseline":copy.deepcopy(design),"proposal":{**copy.deepcopy(design),"id":"design-"+"2"*64},
                "accepted":None,"accepted_intended":None,"delta":{"outcome":"change_detected","mermaid":"flowchart LR"}}
        report={"id":"pr-1-"+"3"*64,"number":1,"artifact_path":state.output/"fixture.json",
                "files":[{"path":"src/lib.rs"}],"architecture":{"nodes":[{"path":"src/lib.rs"}]},
                "artifact":{"selected_baseline":"4"*40,"proposal":"e"*40,
                            "onboarding_context":copy.deepcopy(context),"scope_manifest":copy.deepcopy(context),
                            "hld":{"sources":[source],"source_gaps":[]}}}
        calls=[];published=[]
        def native(action,identity,payload,slug,principal,maintainer,*,publication_fence=None,**kwargs):
            calls.append((action,copy.deepcopy(payload)))
            if publication_fence:publication_fence()
            published.append(action)
            return 200,copy.deepcopy(packet)
        state.native_onboarding_request=native
        return state,report,packet,context,access,calls,published,revision_calls

    def test_real_evidence_prefix_is_retained_privately_and_resolves_exact_span(self):
        with tempfile.TemporaryDirectory() as temporary:
            state,report,packet,*_=self.fixture(Path(temporary))
            self.assertTrue(state.preserve_hld_sources(report))
            source=report["artifact"]["hld"]["sources"][0];evidence_id=source["evidence_id"]
            directory=state.output/".design-evidence";target=directory/(evidence_id+".json")
            self.assertEqual(directory.stat().st_mode&0o777,0o700)
            self.assertEqual(target.stat().st_mode&0o777,0o600)
            self.assertEqual(state.hld_source(report,packet,packet["baseline"]["id"],evidence_id),
                             {"api_version":"v1","review_id":report["id"],**source})
            state.preserve_hld_sources(report)
            self.assertEqual(len(list(directory.iterdir())),1)

    def test_missing_wrong_design_or_wrong_commit_never_falls_back_to_worktree(self):
        with tempfile.TemporaryDirectory() as temporary:
            state,report,packet,*_=self.fixture(Path(temporary));evidence_id=report["artifact"]["hld"]["sources"][0]["evidence_id"]
            (state.repository/"src").mkdir();(state.repository/"src/lib.rs").write_text("PRIVATE_WORKTREE_SENTINEL")
            self.assertIsNone(state.hld_source(report,packet,packet["baseline"]["id"],evidence_id))
            state.preserve_hld_sources(report)
            self.assertIsNone(state.hld_source(report,packet,"other-design",evidence_id))
            packet["baseline"]["evidence"][0]["commit"]="0"*40
            self.assertIsNone(state.hld_source(report,packet,packet["baseline"]["id"],evidence_id))

    def test_source_cache_tampering_and_identity_replacement_fail_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            state,report,packet,*_=self.fixture(Path(temporary));source=report["artifact"]["hld"]["sources"][0]
            state.preserve_hld_sources(report)
            target=state.output/".design-evidence"/(source["evidence_id"]+".json")
            original=target.read_bytes();retained=json.loads(original)
            retained["source"]["text"]="FORGED_SOURCE_SENTINEL";target.write_text(json.dumps(retained))
            self.assertIsNone(state.hld_source(report,packet,packet["baseline"]["id"],source["evidence_id"]))
            target.write_bytes(original)
            source["text"]="different immutable bytes"
            with self.assertRaises(OSError):state.preserve_hld_sources(report)
            self.assertEqual(target.read_bytes(),original)

    def test_evidence_store_symlink_is_not_followed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);state,report,packet,*_=self.fixture(root)
            elsewhere=root/"outside";elsewhere.mkdir();(state.output/".design-evidence").symlink_to(elsewhere)
            with self.assertRaises(OSError):state.preserve_hld_sources(report)
            source=report["artifact"]["hld"]["sources"][0]
            self.assertIsNone(state.hld_source(report,packet,packet["baseline"]["id"],source["evidence_id"]))
            self.assertEqual(list(elsewhere.iterdir()),[])

    def test_source_retention_budget_becomes_visible_gap_without_unsafe_fallback(self):
        with tempfile.TemporaryDirectory() as temporary:
            state,report,packet,*_=self.fixture(Path(temporary))
            directory=state.output/".design-evidence";directory.mkdir(mode=0o700)
            with (directory/"budget.json").open("wb") as handle:handle.truncate(64*1024*1024)
            self.assertFalse(state.preserve_hld_sources(report))
            status,value=state.hld_request(report)
            self.assertEqual(status,200)
            self.assertTrue(any("retention budget" in gap for gap in value["source_gaps"]))
            source=report["artifact"]["hld"]["sources"][0]
            self.assertIsNone(state.hld_source(report,packet,packet["baseline"]["id"],source["evidence_id"]))

    def test_lost_grant_or_principal_after_preview_prevents_publication(self):
        for change in ("maintainer","principal"):
            with self.subTest(change=change),tempfile.TemporaryDirectory() as temporary:
                state,report,_,_,access,calls,published,_=self.fixture(Path(temporary))
                native=state.native_onboarding_request
                def revoke(*args,**kwargs):
                    access[change]=False if change=="maintainer" else "other-principal"
                    return native(*args,**kwargs)
                state.native_onboarding_request=revoke
                status,value=state.hld_request(report,"hld-preview",{"design_id":"design-"+"2"*64,"expected_revision":0,"corrections":[],"interpretation":"as_built"})
                self.assertEqual(status,404);self.assertEqual(published,[])
                self.assertNotIn("baseline",value)
                self.assertEqual(len(calls),1)

    def test_changed_context_or_pr_commits_prevents_preview_publication(self):
        for change in ("context","pr"):
            with self.subTest(change=change),tempfile.TemporaryDirectory() as temporary:
                state,report,_,context,_,_,published,_=self.fixture(Path(temporary))
                if change=="pr":
                    state.app_auth._verify_pull_revision=lambda *args:(_ for _ in ()).throw(module.AppError("superseded_revision"))
                else:
                    native=state.native_onboarding_request
                    def mutate(*args,**kwargs):
                        context["session_revision"]+=1
                        return native(*args,**kwargs)
                    state.native_onboarding_request=mutate
                self.assertEqual(state.hld_request(report,"hld-preview",{"design_id":"design-"+"2"*64,"expected_revision":0,"corrections":[],"interpretation":"as_built"})[0],404)
                self.assertEqual(published,[])

    def test_invalid_defaults_or_narrowed_paths_do_not_invoke_hld_adapter(self):
        for cause in ("unavailable","narrowed"):
            with self.subTest(cause=cause),tempfile.TemporaryDirectory() as temporary:
                state,report,_,context,_,calls,*_=self.fixture(Path(temporary))
                if cause=="unavailable":state.active_context=lambda **kwargs:(_ for _ in ()).throw(module.AppError("confirmed_repository_context_required"))
                else:context["exclude"].append("src/lib.rs")
                self.assertEqual(state.hld_request(report)[0],404)
                self.assertEqual(calls,[])

    def test_export_requires_both_current_and_immutable_review_policy(self):
        for immutable,current in ((False,True),(True,False),(False,False)):
            with self.subTest(immutable=immutable,current=current),tempfile.TemporaryDirectory() as temporary:
                state,report,_,context,*_=self.fixture(Path(temporary))
                report["artifact"]["scope_manifest"]["export_allowed"]=immutable
                context["export_allowed"]=current
                status,value=state.hld_request(report)
                self.assertEqual(status,200)
                self.assertFalse(value["can_export"])

    def test_optional_retention_failure_does_not_reject_a_committed_acceptance(self):
        with tempfile.TemporaryDirectory() as temporary:
            state,report,_,_,_,calls,published,_=self.fixture(Path(temporary))
            state.preserve_hld_sources=lambda *args:(_ for _ in ()).throw(OSError("fixture retention failed"))
            status,view=state.hld_request(report)
            self.assertEqual(status,200);self.assertTrue(view["source_gaps"])
            state.hld_request(report,"hld-preview",{"design_id":"design-"+"2"*64,"expected_revision":0,"corrections":[],"interpretation":"as_built"})
            preview_id=calls[-1][1]["preview_id"]
            status,_=state.hld_request(report,"hld-accept",{"preview_id":preview_id,"expected_revision":1,"input_fingerprint":"5"*64,"known_gaps_accepted":True,"activate_default":True})
            self.assertEqual(status,200)
            self.assertEqual(published[-1],"hld-accept")

    def test_preview_cannot_be_reused_by_another_pr_sharing_same_baseline(self):
        with tempfile.TemporaryDirectory() as temporary:
            state,report,_,_,_,calls,published,_=self.fixture(Path(temporary))
            self.assertEqual(state.hld_request(report,"hld-preview",{"design_id":"design-"+"1"*64,"expected_revision":0,"corrections":[],"interpretation":"as_built"})[0],200)
            preview_id=calls[-1][1]["preview_id"]
            other={**copy.deepcopy(report),"id":"pr-2-"+"6"*64,"number":2}
            before=len(calls)
            self.assertEqual(state.hld_request(other,"hld-accept",{"preview_id":preview_id,"expected_revision":1,"input_fingerprint":"5"*64,"known_gaps_accepted":True,"activate_default":True})[0],404)
            self.assertEqual(len(calls),before)
            self.assertEqual(published,["hld-preview"])

    def test_historical_accepted_export_requires_its_native_immutable_policy(self):
        with tempfile.TemporaryDirectory() as temporary:
            state,report,packet,context,*_=self.fixture(Path(temporary))
            context["export_allowed"]=True
            report["artifact"]["scope_manifest"]["export_allowed"]=True
            packet["accepted"]={**copy.deepcopy(packet["baseline"]),"id":"accepted-old","markdown":"PRIVATE_EXPORT_SENTINEL"}
            packet["export_views"]={"baseline":True,"proposal":True,"accepted":False}
            request=module.handler(state).__new__(module.handler(state))
            status,value=request.hld_json(report,f"/v1/reviews/{report['id']}/hld/export",{"view":["accepted"]})
            self.assertEqual(status,404)
            self.assertNotIn("PRIVATE_EXPORT_SENTINEL",str(value))
            packet["export_views"]["accepted"]=True
            status,value=request.hld_json(report,f"/v1/reviews/{report['id']}/hld/export",{"view":["accepted"]})
            self.assertEqual(status,200);self.assertEqual(value["markdown"],"PRIVATE_EXPORT_SENTINEL")

    def test_http_mutation_rejects_operator_paths_and_has_no_database_copy_route(self):
        with tempfile.TemporaryDirectory() as temporary:
            state,report,*_=self.fixture(Path(temporary));state.report=lambda identity:report
            Handler=module.handler(state)
            for suffix,body,status in (("preview",{"expected_revision":0,"corrections":[],"artifact_path":"/PRIVATE_SENTINEL"},400),
                                       ("accept",{"expected_revision":1,"artifact_path":"/PRIVATE_SENTINEL"},400),
                                       ("database-copy",{},404)):
                request=Handler.__new__(Handler);request.path=f"/v1/reviews/{report['id']}/hld/{suffix}"
                request.server=SimpleNamespace(server_port=1234)
                encoded=json.dumps(body).encode();request.rfile=io.BytesIO(encoded)
                request.headers={"Host":"127.0.0.1:1234","Authorization":"Bearer "+state.session,"X-CSB-Session":state.session,"Content-Length":str(len(encoded))}
                errors=[];request.send_error=lambda code,*args:errors.append(code)
                request.do_POST()
                self.assertEqual(errors,[status])


if __name__=="__main__":unittest.main()
