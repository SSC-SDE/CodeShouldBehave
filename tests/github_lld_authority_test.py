"""LLD request identity/fresh-authority regression tests with inert providers."""
import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import github_mvp_server as host


class LldAuthorityTest(unittest.TestCase):
    def fixture(self, root):
        state = host.State(root, root)
        self.addCleanup(state._host_lock.close)
        current = {"origin": "owner/repo", "login": "reviewer", "permissions": {"pull": True, "push": True}}
        context = {"version": "csb-accepted-context-v1", "usable": True,
            "session_id": "confirmed", "session_revision": 2,
            "input_fingerprint": "a" * 64, "proposal_fingerprint": "b" * 64,
            "accepted_scope_fingerprint": "c" * 64,
            "include": ["src"], "exclude": [], "contexts": [], "export_allowed": True}
        report = {"id": "pr-1-" + "d" * 64, "number": 1,
            "artifact_path": root / "immutable-fixture.json",
            "files": [{"path": "src/lib.rs"}], "architecture": {"nodes": []},
            "artifact": {"onboarding_context": copy.deepcopy(context), "lld": {},
                "selected_baseline": "e" * 40, "proposal": "f" * 40,
                "scope_manifest": {"primary_repository": {"provider": "github", "owner": "owner", "name": "repo"},
                    "contexts": [], "export_allowed": True},
                "hld": {"baseline": {"id": "design-baseline"}, "proposal": {"id": "design-proposal"}}}}
        state.repository_slug = lambda: current["origin"]
        state._lld_parent_registered.add(report["id"])
        provider_calls, native_calls, delivered = [], [], []
        lock = threading.Lock()

        def command(args, **kwargs):
            with lock:
                provider_calls.append((list(args), dict(kwargs)))
            if args[1:3] == ["api", "user"]:
                value = current["login"]
            elif args[1:3] == ["api", "repos/owner/repo"]:
                value = json.dumps(current["permissions"])
            elif args[1:3] == ["api", "repos/owner/repo/pulls/1"]:
                value = json.dumps({"base": {"sha": "e" * 40}, "head": {"sha": "f" * 40}})
            else:
                raise AssertionError("Unexpected fixture provider operation")
            return subprocess.CompletedProcess(args, 0, value, "")

        def native(action, identity, request, slug, principal, maintainer, *, publication_fence=None, **kwargs):
            self.assertEqual(slug, "owner/repo")
            if action == "defaults":
                self.assertEqual(request["repository"]["owner"], "owner")
                self.assertEqual(request["repository"]["name"], "repo")
                return 200, copy.deepcopy(context)
            native_calls.append(action)
            if current.get("after_native"):
                current["after_native"]()
            if publication_fence:
                publication_fence()
            delivered.append(action)
            return 200, {"identity": {"parent_hld": "design-proposal"}, "result": "bounded-fixture"}

        state.command, state.native_onboarding_request = command, native
        return state, current, context, report, provider_calls, native_calls, delivered

    def read(self, state, report):
        return state.lld_request(report, "lld-detail", {"parent_hld": "design-proposal", "entry": "fixture-entry"})

    def write_report(self, root, report):
        artifact=copy.deepcopy(report["artifact"])
        artifact.update({"artifact_version":"csb-review-artifact-v1", "review_id":"d"*64,
            "baseline":"e"*40, "files":report["files"], "architecture":report["architecture"], "warnings":[]})
        (root / "pr-1.json").write_text(json.dumps(artifact))

    def get_catalog(self, state, report):
        request=object.__new__(host.handler(state))
        request.path=f"/v1/reviews/{report['id']}/lld/catalog?parent_hld=design-proposal"
        request.loopback_origin_allowed=lambda:True
        request.api_authorized=lambda:True
        responses=[]
        request.json_status=lambda status,value:responses.append((status,value))
        request.send_error=lambda status,*args:responses.append((status,{}))
        request.do_GET()
        self.assertEqual(len(responses),1)
        return responses[0]

    def test_cli_lld_get_checks_origin_inside_lock_and_at_delivery_without_duplicate_lookup(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            state,current,_,report,calls,native,delivered=self.fixture(root)
            self.write_report(root,report)
            origins=[]
            def origin():
                origins.append(state.context_lock._is_owned())
                return current["origin"]
            state.repository_slug=origin
            self.assertEqual(self.get_catalog(state,report)[0],200)
            self.assertEqual(origins,[True,False],"Initial origin must be read under the request lock; delivery rereads it")
            self.assertEqual(len(calls),4,"Actor and grants remain fresh before and after native work")
            self.assertEqual(native,["lld-catalog"])
            self.assertEqual(delivered,["lld-catalog"])

    def test_wrong_initial_origin_get_denies_before_provider_or_native_in_cli_and_app(self):
        for app in (False,True):
            with self.subTest(app=app),tempfile.TemporaryDirectory() as temporary:
                root=Path(temporary)
                state,current,_,report,calls,native,delivered=self.fixture(root)
                self.write_report(root,report)
                current["origin"]="owner/other"
                if app:
                    state.app_auth=SimpleNamespace(access=lambda *args:self.fail("Wrong origin must precede App reads"))
                self.assertEqual(self.get_catalog(state,report)[0],404)
                self.assertEqual((calls,native,delivered),([],[],[]))

    def test_only_cli_lld_get_routing_defers_report_origin_and_never_other_routes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            state,current,_,report,_,_,_=self.fixture(root)
            self.write_report(root,report)
            current["origin"]="owner/other"
            prefix=f"/v1/reviews/{report['id']}"
            self.assertIsNotNone(state.routed_report(prefix+"/lld/catalog",lld_get=True))
            self.assertIsNone(state.routed_report(prefix+"/lld/catalog"),"Default/direct routing validates origin")
            self.assertIsNone(state.report(report["id"]))
            for path in (prefix,prefix+"/hld",prefix+"/source",prefix+"/lld/accept",prefix+"/lld/preview",prefix+"/lld/unknown"):
                with self.subTest(path=path):
                    self.assertIsNone(state.routed_report(path,lld_get=True))
            state.app_auth=SimpleNamespace()
            self.assertIsNone(state.routed_report(prefix+"/lld/catalog",lld_get=True),"App report validation stays unchanged")
            self.assertIsNone(state.report(report["id"],_defer_lld_origin=True),"Private opt-in cannot defer App origin checks")

    def test_delivery_origin_and_current_access_really_overlap_and_both_join(self):
        with tempfile.TemporaryDirectory() as temporary:
            state,_,_,_,_,_,_=self.fixture(Path(temporary))
            barrier=threading.Barrier(2)
            entered=[]
            def origin():
                entered.append("origin")
                barrier.wait(timeout=2)
                return "owner/repo"
            def access(**kwargs):
                self.assertEqual(kwargs,{"slug":"owner/repo","parallel_cli":True})
                entered.append("access")
                barrier.wait(timeout=2)
                return {"principal":"github:reviewer","maintainer":True}
            state.repository_slug=origin
            state.fresh_hld_access=access
            self.assertEqual(state._lld_delivery_access("owner/repo"),{"principal":"github:reviewer","maintainer":True})
            self.assertCountEqual(entered,["origin","access"])

    def test_failed_parallel_delivery_read_never_releases_completed_native_result(self):
        for cause in ("origin_os_error","origin_timeout","actor_timeout","malformed_grant"):
            with self.subTest(cause=cause),tempfile.TemporaryDirectory() as temporary:
                state,current,_,report,_,native,delivered=self.fixture(Path(temporary))
                def after():
                    if cause.startswith("origin_"):
                        def rejected_origin():
                            if cause=="origin_timeout":raise subprocess.TimeoutExpired(["git"],10)
                            raise OSError("inert fixture failure")
                        state.repository_slug=rejected_origin
                    else:
                        original=state.command
                        def rejected_provider(args,**kwargs):
                            if cause=="actor_timeout" and args[2]=="user":
                                raise subprocess.TimeoutExpired(args,10)
                            if cause=="malformed_grant" and args[2]=="repos/owner/repo":
                                return subprocess.CompletedProcess(args,0,"not-json","")
                            return original(args,**kwargs)
                        state.command=rejected_provider
                current["after_native"]=after
                status,value=self.read(state,report)
                self.assertEqual(status,404)
                self.assertNotIn("result",value)
                self.assertEqual(native,["lld-detail"])
                self.assertEqual(delivered,[])

    def test_parallel_provider_reads_really_overlap_and_both_validate(self):
        with tempfile.TemporaryDirectory() as temporary:
            state, _, _, _, _, _, _ = self.fixture(Path(temporary))
            barrier = threading.Barrier(2)
            entered, lock = [], threading.Lock()

            def provider(args, **kwargs):
                self.assertEqual(kwargs["timeout"], 10)
                with lock:
                    entered.append(args[2])
                barrier.wait(timeout=2)
                return subprocess.CompletedProcess(args, 0, "reviewer" if args[2] == "user" else '{"pull":true,"push":true}', "")

            state.command = provider
            state.repository_slug = lambda: self.fail("Bounded parallel access must use its already verified slug")
            value = state.fresh_hld_access(slug="owner/repo", parallel_cli=True)
            self.assertEqual(value, {"principal": "github:reviewer", "maintainer": True})
            self.assertCountEqual(entered, ["user", "repos/owner/repo"])

    def test_each_read_rechecks_actor_permissions_and_context_before_and_after_native_work(self):
        with tempfile.TemporaryDirectory() as temporary:
            state, _, _, report, calls, native, delivered = self.fixture(Path(temporary))
            self.assertEqual(self.read(state, report)[0], 200)
            self.assertEqual(native, ["lld-detail"])
            self.assertEqual(delivered, ["lld-detail"])
            self.assertEqual(sum(args[2] == "user" for args, _ in calls), 2)
            self.assertEqual(sum(args[2] == "repos/owner/repo" for args, _ in calls), 2)
            self.assertTrue(all(kwargs.get("timeout") == 10 for _, kwargs in calls))
            self.assertEqual(self.read(state, report)[0], 200)
            self.assertEqual(len(calls), 8, "A second read must not reuse an authorization grant")

    def test_before_native_origin_or_artifact_repository_mismatch_is_denied(self):
        for field in ("origin", "artifact_repository"):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as temporary:
                state, current, _, report, calls, native, delivered = self.fixture(Path(temporary))
                if field == "origin":
                    current["origin"] = "owner/other"
                else:
                    report["artifact"]["scope_manifest"]["primary_repository"]["name"] = "other"
                self.assertEqual(self.read(state, report)[0], 404)
                self.assertEqual(calls, [])
                self.assertEqual(native, [])
                self.assertEqual(delivered, [])

    def test_changed_origin_actor_grant_or_context_after_work_never_delivers_source_or_detail(self):
        for cause in ("origin", "actor", "grant", "context"):
            with self.subTest(cause=cause), tempfile.TemporaryDirectory() as temporary:
                state, current, context, report, _, native, delivered = self.fixture(Path(temporary))
                def after():
                    if cause == "origin":
                        current["origin"] = "owner/other"
                    elif cause == "actor":
                        current["login"] = "other-reviewer"
                    elif cause == "grant":
                        current["permissions"]["pull"] = False
                    else:
                        context["session_revision"] += 1
                current["after_native"] = after
                for action in ("lld-detail", "lld-source"):
                    current["origin"], current["login"] = "owner/repo", "reviewer"
                    current["permissions"]["pull"] = True
                    context["session_revision"] = 2
                    status, value = state.lld_request(report, action, {"parent_hld": "design-proposal", "entry": "fixture-entry"})
                    self.assertEqual(status, 404)
                    self.assertNotIn("result", value)
                self.assertTrue(native)
                self.assertEqual(delivered, [])

    def test_either_failed_malformed_or_timed_out_parallel_provider_read_denies_work(self):
        for cause in ("login_failure", "permissions_failure", "login_invalid", "permissions_invalid", "permissions_non_object", "permissions_not_boolean", "timeout"):
            with self.subTest(cause=cause), tempfile.TemporaryDirectory() as temporary:
                state, _, _, report, _, native, delivered = self.fixture(Path(temporary))
                def provider(args, **kwargs):
                    is_login = args[2] == "user"
                    if cause == "timeout" and is_login:
                        raise subprocess.TimeoutExpired(args, 10)
                    result = "reviewer" if is_login else '{"pull":true,"push":true}'
                    if cause == "login_invalid" and is_login:
                        result = "not a valid login"
                    if cause == "permissions_invalid" and not is_login:
                        result = "malformed-json"
                    if cause == "permissions_non_object" and not is_login:
                        result = "[]"
                    if cause == "permissions_not_boolean" and not is_login:
                        result = '{"pull":1,"push":true}'
                    failed = (cause == "login_failure" and is_login) or (cause == "permissions_failure" and not is_login)
                    return subprocess.CompletedProcess(args, 1 if failed else 0, result, "")
                state.command = provider
                self.assertEqual(self.read(state, report)[0], 404)
                self.assertEqual(native, [])
                self.assertEqual(delivered, [])

    def test_related_repository_and_current_path_checks_are_not_skipped(self):
        for cause in ("related", "path", "unsupported_context"):
            with self.subTest(cause=cause), tempfile.TemporaryDirectory() as temporary:
                state, _, context, report, _, native, delivered = self.fixture(Path(temporary))
                def app_access(slug):
                    if slug != "owner/repo":
                        raise host.AppError("repository_unavailable")
                    return {"principal": "github:reviewer", "maintainer": True}
                state.app_auth = SimpleNamespace(access=app_access, public_status=lambda: {"authenticated": True})
                if cause == "related":
                    report["artifact"]["scope_manifest"]["contexts"] = [{"repository": {"provider": "github", "owner": "owner", "name": "related"}}]
                elif cause == "path":
                    context["include"] = ["src/allowed"]
                else:
                    context["contexts"] = [{"repository": {"provider": "github", "owner": "owner", "name": "related"}}]
                self.assertEqual(self.read(state, report)[0], 404)
                self.assertEqual(native, [])
                self.assertEqual(delivered, [])

    def test_legacy_hld_access_and_default_context_callers_keep_origin_discovery(self):
        with tempfile.TemporaryDirectory() as temporary:
            state, _, context, _, calls, _, _ = self.fixture(Path(temporary))
            discoveries = []
            state.repository_slug = lambda: discoveries.append("origin") or "owner/repo"
            access = state.fresh_hld_access()
            self.assertEqual(discoveries, ["origin"])
            self.assertTrue(all(kwargs == {} for _, kwargs in calls))
            self.assertEqual(state.active_context(access=access), context)
            self.assertEqual(discoveries, ["origin", "origin"])


if __name__ == "__main__":
    unittest.main()
