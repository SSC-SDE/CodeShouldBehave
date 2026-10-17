"""Confirmed context and immutable dashboard links; no live provider calls."""
import json
from pathlib import Path
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"scripts"))
import github_mvp_server as module


class ContextTest(unittest.TestCase):
    def state(self, root):
        state=module.State(root,root)
        state.repository_slug=lambda:"owner/repo"
        return state

    def artifact(self, digest="c", proposal="b"):
        return {"artifact_version":"csb-review-artifact-v1","review_id":digest*64,
                "baseline":"a"*40,"selected_baseline":"a"*40,"proposal":proposal*40,
                "scope_manifest":{"primary_repository":{"provider":"github","owner":"owner","name":"repo"}},
                "files":[],"architecture":{"nodes":[],"edges":[]},"warnings":[]}

    def test_exact_link_survives_latest_pr_replacement(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary); state=self.state(root); typed=root/"pr-1.json"
            original=json.dumps(self.artifact()); typed.write_text(original)
            old=state.report("pr-1"); state.preserve_report(old,typed)
            typed.write_text(json.dumps(self.artifact("d","e")))
            self.assertEqual(state.report(old["id"])["artifact"]["proposal"],"b"*40)
            self.assertEqual(state.report("pr-1")["artifact"]["proposal"],"e"*40)
            self.assertEqual((root/"reviews"/old["id"]/"review.json").read_text(),original)
            self.assertEqual(len(state.reports()),1)

    def test_same_identity_cannot_be_replaced_with_different_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary); state=self.state(root); typed=root/"pr-1.json"
            typed.write_text(json.dumps(self.artifact())); report=state.report("pr-1")
            state.preserve_report(report,typed); state.preserve_report(report,typed)
            typed.write_text(json.dumps(self.artifact(proposal="e")))
            with self.assertRaises(OSError):state.preserve_report(report,typed)
            self.assertEqual(state.report(report["id"])["artifact"]["proposal"],"b"*40)

    def test_symlink_artifact_store_is_not_read_or_written(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary); state=self.state(root); typed=root/"pr-1.json"
            typed.write_text(json.dumps(self.artifact())); report=state.report("pr-1")
            outside=root/"elsewhere"; outside.mkdir(); (root/"reviews").symlink_to(outside)
            self.assertIsNone(state.report(report["id"]))
            with self.assertRaises(OSError):state.preserve_report(report,typed)
            self.assertEqual(list(outside.iterdir()),[])

    def test_defaults_lookup_uses_current_viewer_and_has_no_broad_scope_fallback(self):
        with tempfile.TemporaryDirectory() as temporary:
            state=self.state(Path(temporary)); calls=[]
            def request(action,identity,body):calls.append((action,identity,body));return 404,{"error":"DeniedOrMissing"}
            state.onboarding_request=request
            with self.assertRaisesRegex(module.AppError,"confirmed_repository_context_required"):state.active_context()
            self.assertEqual(calls[0][0:2],("defaults","active"))
            self.assertEqual(calls[0][2]["repository"],{"security_domain":"local","provider":"github","owner":"owner","name":"repo"})
            for context in ({"version":"csb-accepted-context-v1","usable":False},
                            {"version":"csb-accepted-context-v1","usable":True,"contexts":[{"repository":"unapproved"}]}):
                state.onboarding_request=lambda *args:(200,context)
                with self.assertRaises(module.AppError):state.active_context()

    def test_review_process_gets_scoped_context_but_no_broker_credentials(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary); state=self.state(root)
            context={"version":"csb-accepted-context-v1","usable":True,"contexts":[],"include":["src"],"exclude":["src/private"],
                     "session_id":"confirmed","session_revision":7,"commit":"a"*40,
                     "input_fingerprint":"c"*64,"proposal_fingerprint":"d"*64,"accepted_scope_fingerprint":"e"*64}
            state.active_context=lambda:context
            state.app_auth=SimpleNamespace(access=lambda slug:{"principal":"github-user:99","maintainer":True},api=lambda route:{"base":{"sha":"a"*40},"head":{"sha":"b"*40}},fetch_commits=lambda *args:None)
            state.native_onboarding_request=lambda *args,**kwargs:(200,{"target_commit":"b"*40,"status":"unchanged"})
            with patch.dict(module.os.environ,{"GITHUB_TOKEN":"SECRET_SENTINEL"}),patch.object(module.subprocess,"run",return_value=SimpleNamespace(returncode=0)) as run:
                state.app_review(1,root)
            command=run.call_args.args[0]; self.assertIn("--onboarding-context",command)
            received=json.loads((root/"accepted-context.json").read_text())
            self.assertEqual({key:value for key,value in received.items() if key!="drift"},context)
            self.assertEqual(received["drift"]["target_commit"],"b"*40)
            self.assertEqual((root/"accepted-context.json").stat().st_mode&0o777,0o600)
            self.assertNotIn("SECRET_SENTINEL",str(run.call_args.kwargs["env"]))

    def test_readonly_drift_uses_fetched_mirror_and_disabled_checks_preserve_scope(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);state=self.state(root)
            context={"session_id":"accepted","session_revision":9,"commit":"a"*40,"include":["src"],
                     "input_fingerprint":"c"*64,"proposal_fingerprint":"d"*64,"accepted_scope_fingerprint":"e"*64}
            state.active_context=lambda:context
            events=[]
            state.app_auth=SimpleNamespace(access=lambda slug:{"principal":"viewer","maintainer":False},
                api=lambda route:{"base":{"sha":"a"*40},"head":{"sha":"b"*40}},
                fetch_commits=lambda *args:events.append("fetched"))
            def assess(*args,**kwargs):
                self.assertEqual(events,["fetched"])
                self.assertEqual(args[0:2],("drift","accepted"))
                self.assertEqual(args[2]["reference"],"b"*40)
                self.assertEqual(args[4:6],("viewer",False))
                self.assertEqual(kwargs["source_repository"],root/"mirror.git")
                return 200,{"status":"incomplete","target_commit":"b"*40}
            state.native_onboarding_request=assess
            with patch.object(module.subprocess,"run",return_value=SimpleNamespace(returncode=0)):
                state.app_review(1,root)
                (root/"accepted-context.json").unlink()
                state.onboarding_enabled=False
                state.native_onboarding_request=lambda *args,**kwargs:self.fail("disabled drift reader called")
                state.app_review(1,root)
            received=json.loads((root/"accepted-context.json").read_text())
            self.assertEqual(received["include"],["src"])
            self.assertEqual(received["drift"]["status"],"incomplete")
            self.assertFalse(received["drift"]["reuse_allowed"])
            self.assertFalse(received["drift"]["defaults_changed"])

    def test_context_change_before_publish_keeps_previous_pointer(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary); state=self.state(root)
            state.app_auth=SimpleNamespace(lock=threading.Lock(),generation=1,_token="sentinel",clock=lambda:0,_token_deadline=100)
            typed=root/"pending.json";candidate=root/"pending.html";typed.write_text("{}");candidate.write_text("report")
            provenance={"session_id":"old","session_revision":7,"proposal_fingerprint":"a"*64}
            state.active_context=lambda:{**provenance,"session_id":"new"}
            report={"id":"pr-1-"+"c"*64,"artifact":{"onboarding_context":provenance}}
            with self.assertRaisesRegex(module.AppError,"superseded_context"):state.publish_report(1,typed,candidate,report,1)
            self.assertFalse((root/"pr-1.json").exists());self.assertFalse((root/"reviews").exists())

    def test_missing_confirmation_finishes_job_with_actionable_message(self):
        class InlineThread:
            def __init__(self,target,**kwargs):self.target=target
            def start(self):self.target()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);state=self.state(root)
            state.app_auth=SimpleNamespace(generation=1)
            state.app_review=lambda *args:(_ for _ in ()).throw(module.AppError("confirmed_repository_context_required"))
            with patch.object(module.threading,"Thread",InlineThread):state.start_review(1)
            self.assertFalse(state.review_running)
            self.assertIn("Use this confirmed context for PR reviews",state.review_error)
            self.assertIsNone(state.review_url);self.assertFalse((root/"pr-1.json").exists())

    def test_app_candidate_without_receipt_cannot_publish(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary); state=self.state(root);state.app_auth=SimpleNamespace()
            typed=root/"pending.json";candidate=root/"pending.html";typed.write_text("{}");candidate.write_text("report")
            with self.assertRaisesRegex(module.AppError,"confirmed_repository_context_required"):
                state.publish_report(1,typed,candidate,{"artifact":{}},1)
            self.assertFalse((root/"pr-1.json").exists())

    def test_final_provider_revision_fence_prevents_current_publication(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);state=self.state(root)
            context={"session_id":"accepted","session_revision":7,"input_fingerprint":"a"*64,
                     "proposal_fingerprint":"b"*64,"accepted_scope_fingerprint":"c"*64,
                     "include":["src"],"exclude":[],"contexts":[],"export_allowed":False}
            state.active_context=lambda:context
            def superseded(*args):raise module.AppError("superseded_revision")
            state.app_auth=SimpleNamespace(access=lambda slug:{"principal":"viewer"},_verify_pull_revision=superseded)
            typed=root/"pending.json";candidate=root/"pending.html";typed.write_text("{}");candidate.write_text("new")
            (root/"pr-1.json").write_text("old")
            artifact={"onboarding_context":context,"scope_manifest":context,"selected_baseline":"a"*40,"proposal":"b"*40}
            with self.assertRaisesRegex(module.AppError,"superseded_revision"):
                state.publish_report(1,typed,candidate,{"artifact":artifact},1)
            self.assertEqual((root/"pr-1.json").read_text(),"old")
            self.assertFalse((root/"reviews").exists())

    def test_narrowed_active_scope_denies_historical_source_without_rewriting_it(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);state=self.state(root)
            state.app_auth=SimpleNamespace(access=lambda slug:{"principal":"github-user:99"})
            state.active_context=lambda:{"include":["src"],"exclude":["src/rejected"]}
            report={"files":[{"path":"src/rejected/private.rs"}],"architecture":{"nodes":[]},"artifact":{"onboarding_context":{},"scope_manifest":{"contexts":[]}}}
            with self.assertRaisesRegex(module.AppError,"context_unavailable"):state.require_app_access(report)
            self.assertEqual(report["files"][0]["path"],"src/rejected/private.rs")
            report["files"]=[];report["architecture"]["nodes"]=[{"path":"outside/unchanged.rs"}]
            with self.assertRaises(module.AppError):state.require_app_access(report)
            report["architecture"]["nodes"]=[{"path":"src/allowed.rs"}]
            self.assertEqual(state.require_app_access(report)["principal"],"github-user:99")

    def test_zero_exit_without_new_output_never_reuses_an_old_success(self):
        class InlineThread:
            def __init__(self,target,**kwargs):self.target=target
            def start(self):self.target()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);state=self.state(root)
            (root/"pr-1.html").write_text("old review")
            (root/"pr-1.json").write_text(json.dumps(self.artifact()))
            state.app_auth=SimpleNamespace(generation=1)
            state.app_review=lambda *args:SimpleNamespace(returncode=0)
            with patch.object(module.threading,"Thread",InlineThread):state.start_review(1)
            self.assertFalse(state.review_running);self.assertIsNone(state.review_url);self.assertIsNotNone(state.review_error)

    def test_kill_switch_disables_new_scans_but_keeps_confirmed_authorized_defaults(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);state=self.state(root);state.onboarding_enabled=False
            state.authenticated=lambda:True
            state.cli_onboarding_request=lambda action,*args:(200,{"action":action})
            for action in ("create","scan","detect","inspect","decisions","confirm","provider-decisions","integration-decisions","drift"):
                self.assertEqual(state.onboarding_request(action,"saved",{})[0],503)
            for action in ("read","defaults","activate","cancel"):
                self.assertEqual(state.onboarding_request(action,"saved",{})[0],200)
            state.authenticated=lambda:False
            self.assertEqual(state.onboarding_request("defaults","active",{})[0],401)


if __name__=="__main__":unittest.main()
