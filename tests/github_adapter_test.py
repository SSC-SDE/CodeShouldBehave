"""Typed-artifact bridge contract tests; no live GitHub requests."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"scripts"))
from types import SimpleNamespace
from unittest.mock import patch
import threading
spec=importlib.util.spec_from_file_location("github_mvp_server",Path(__file__).resolve().parents[1]/"scripts/github_mvp_server.py")
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
class ArtifactTest(unittest.TestCase):
    def test_architecture_handoff_requires_an_immutable_review_and_keeps_the_diff_view(self):
        review='pr-35-'+'a'*64
        self.assertEqual(module.architecture_workspace_url(review),f'/workspace#review={review}&layout=architecture&side=diff')
        for identity in ['pr-35', 'pr-0-'+'a'*64, 'pr-35-'+'A'*64, 'pr-35-'+'a'*64+'&review=other', '../private', None]:
            with self.assertRaises(ValueError):module.architecture_workspace_url(identity)
    def test_revision_routes_resolve_the_requested_pr_and_never_the_first_report(self):
        with tempfile.TemporaryDirectory() as temporary:
            state=module.State(Path(temporary),Path(temporary))
            state.reports=lambda:[{'id':'pr-33-'+'c'*64}]
            state.report=lambda identity:{'id':identity} if identity=='pr-31' else None
            self.assertEqual(state.routed_report('/v1/prs/PR-31/revisions')['id'],'pr-31')
            self.assertIsNone(state.routed_report('/v1/prs/PR-99/revisions'))
            self.assertIsNone(state.routed_report('/v1/prs/other'))
            self.assertIsNone(state.routed_report('/v1/reviews/pr-31/files'))
    def test_app_profile_does_not_fall_back_to_cli_and_checks_all_contexts(self):
        with tempfile.TemporaryDirectory() as temporary:
            state=module.State(Path(temporary),Path(temporary));state.repository_slug=lambda:'owner/repo'
            calls=[]
            def access(slug):
                calls.append(slug)
                if slug=='external/repo':raise module.AppError('repository_unavailable')
                return {'principal':'github-user:99'}
            state.app_auth=SimpleNamespace(public_status=lambda:{'authenticated':False},access=access)
            state.command=lambda *args,**kwargs:self.fail('App profile must not use GitHub CLI')
            self.assertFalse(state.authenticated())
            state.active_context=lambda:{'include':['.'],'exclude':[]}
            report={'files':[],'architecture':{'nodes':[]},'artifact':{'onboarding_context':{},'scope_manifest':{'contexts':[{'repository':{'provider':'github','owner':'external','name':'repo'}}]}}}
            with self.assertRaises(module.AppError):state.require_app_access(report)
            self.assertEqual(calls,['owner/repo','external/repo'])

    def test_inbox_omits_a_report_with_revoked_context_access(self):
        with tempfile.TemporaryDirectory() as temporary:
            state=module.State(Path(temporary),Path(temporary));state.repository_slug=lambda:'owner/repo'
            def report(number):return {'id':f'pr-{number}-'+('c'*64),'number':number,'observation':{'baseline':'a'*40,'proposal':'b'*40},'architecture':{},'artifact':{'scope_version':'scope','selected_baseline':'a'*40}}
            allowed,denied=report(1),report(2);state.reports=lambda:[allowed,denied]
            def require(item):
                if item['number']==2:raise module.AppError('repository_unavailable')
            state.require_app_access=require
            request=module.handler(state).__new__(module.handler(state))
            value=request.workspace_json(allowed,'/v1/prs',{})
            self.assertEqual([item['provider_pr_id'] for item in value['items']],['PR-1'])

    def test_authentication_change_fences_publication_and_failure_finishes_job(self):
        class InlineThread:
            def __init__(self,target,**kwargs):self.target=target
            def start(self):self.target()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);state=module.State(root,root);state.repository_slug=lambda:'owner/repo'
            state.app_auth=SimpleNamespace(generation=1,lock=threading.Lock(),_token='TOKEN_SENTINEL',_token_deadline=100,clock=lambda:0,access=lambda slug:{'principal':'github-user:99'})
            state.report=lambda *args:{'id':'pr-1-'+'c'*64}
            def review(pr,pending):
                (pending/'pr-1.json').write_text('{}');(pending/'pr-1.html').write_text('review')
                state.app_auth.generation+=1
                return SimpleNamespace(returncode=0)
            state.app_review=review
            with patch.object(module.threading,'Thread',InlineThread):state.start_review(1)
            self.assertFalse((root/'pr-1.json').exists());self.assertFalse(state.review_running);self.assertIsNone(state.review_url);self.assertIsNotNone(state.review_error)
            state.app_review=lambda *args:(_ for _ in ()).throw(KeyError('malformed provider response'))
            with patch.object(module.threading,'Thread',InlineThread):state.start_review(1)
            self.assertFalse(state.review_running);self.assertIsNotNone(state.review_error)

    def test_completed_job_hands_off_the_published_digest_to_architecture_view(self):
        class InlineThread:
            def __init__(self,target,**kwargs):self.target=target
            def start(self):self.target()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);state=module.State(root,root);state.repository_slug=lambda:'owner/repo'
            state.app_auth=SimpleNamespace(generation=1,lock=threading.Lock(),_token='TOKEN_SENTINEL',_token_deadline=100,clock=lambda:0,access=lambda slug:{'principal':'github-user:99'})
            artifact={'artifact_version':'csb-review-artifact-v1','review_id':'c'*64,'baseline':'a'*40,'selected_baseline':'a'*40,'proposal':'b'*40,'scope_manifest':{'primary_repository':{'provider':'github','owner':'owner','name':'repo'}},'files':[],'architecture':{'nodes':[],'edges':[]},'warnings':[]}
            context={'session_id':'confirmed','session_revision':7,'input_fingerprint':'a'*64,'proposal_fingerprint':'b'*64,'accepted_scope_fingerprint':'c'*64,'include':['src'],'exclude':[],'contexts':[],'export_allowed':False}
            state.app_auth._verify_pull_revision=lambda *args:None
            state.active_context=lambda:context
            artifact['onboarding_context']=context.copy()
            artifact['scope_manifest'].update({key:context[key] for key in ('include','exclude','contexts','export_allowed')})
            def review(pr,pending):
                (pending/'pr-1.json').write_text(json.dumps(artifact));(pending/'pr-1.html').write_text('review')
                return SimpleNamespace(returncode=0)
            state.app_review=review
            with patch.object(module.threading,'Thread',InlineThread):state.start_review(1)
            self.assertFalse(state.review_running);self.assertIsNone(state.review_error)
            self.assertEqual(state.review_url,'/workspace#review=pr-1-'+'c'*64+'&layout=architecture&side=diff')
            self.assertTrue((root/'pr-1.json').is_file())
            self.assertNotIn('TOKEN_SENTINEL',state.review_url)

    def test_private_annotations_are_not_visible_to_a_different_account(self):
        with tempfile.TemporaryDirectory() as temporary:
            state=module.State(Path(temporary),Path(temporary));state.repository_slug=lambda:'owner/repo'
            report={'id':'pr-1-'+'c'*64,'number':1,'observation':{'baseline':'a'*40,'proposal':'b'*40},'architecture':{}}
            state.annotations[report['id']]=[{'id':'private','visibility':'private','owner_principal':'github-user:1'},{'id':'shared','visibility':'team','owner_principal':'github-user:1'}]
            request=module.handler(state).__new__(module.handler(state));request.current_access={'principal':'github-user:2'}
            value=request.workspace_json(report,'/v1/reviews/'+report['id']+'/annotations',{})
            self.assertEqual([item['id'] for item in value['items']],['shared'])

    def test_clean_worker_environment_excludes_ambient_credentials(self):
        with tempfile.TemporaryDirectory() as temporary:
            state=module.State(Path(temporary),Path(temporary))
            with patch.dict(module.os.environ,{'GH_TOKEN':'SECRET_SENTINEL','GITHUB_TOKEN':'SECRET_SENTINEL','OPENAI_API_KEY':'SECRET_SENTINEL'}),patch.object(module.subprocess,'run') as run:
                state.command(['trusted-worker'],clean_environment=True)
                self.assertNotIn('SECRET_SENTINEL',str(run.call_args.kwargs['env']))
    def test_local_host_and_origin_cannot_be_rebound_to_an_external_site(self):
        with tempfile.TemporaryDirectory() as temporary:
            state=module.State(Path(temporary),Path(temporary));Handler=module.handler(state);request=Handler.__new__(Handler)
            request.server=SimpleNamespace(server_port=3195)
            for host,origin,allowed in [("127.0.0.1:3195",None,True),("localhost:3195","http://localhost:3195",True),("attacker.invalid:3195",None,False),("127.0.0.1:3195","https://attacker.invalid",False),("127.0.0.1:3196",None,False)]:
                request.headers={"Host":host}
                if origin: request.headers["Origin"]=origin
                self.assertEqual(request.loopback_origin_allowed(),allowed)
    def test_typed_report_does_not_depend_on_html_and_rejects_unknown_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);state=module.State(root,root);state.repository_slug=lambda:"owner/repo"
            artifact={"artifact_version":"csb-review-artifact-v1","review_id":"c"*64,"baseline":"a"*40,"selected_baseline":"a"*40,"proposal":"b"*40,"scope_manifest":{"primary_repository":{"provider":"github","owner":"owner","name":"repo"}},"files":[{"path":"src/main.rs","kind":"modified","additions":7,"deletions":2}],"architecture":{"nodes":[],"edges":[]},"warnings":[]}
            (root/"pr-1.json").write_text(json.dumps(artifact))
            report=state.report("pr-1");self.assertEqual(report["files"][0]["additions"],7)
            self.assertFalse((root/"pr-1.html").exists())
            self.assertIsNotNone(state.report("pr-1-"+"c"*64))
            artifact["review_id"]="d"*64;(root/"pr-1.json").write_text(json.dumps(artifact))
            self.assertIsNone(state.report("pr-1-"+"c"*64))
            artifact["proposal"]="main";(root/"pr-1.json").write_text(json.dumps(artifact));self.assertIsNone(state.report("pr-1"))
            self.assertIsNone(state.report("../../private"))
            (root/"pr-1.json").write_text('[]');self.assertIsNone(state.report("pr-1"))
    def test_html_only_reports_are_not_scraped(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);state=module.State(root,root);state.repository_slug=lambda:"owner/repo"
            (root/"pr-1.html").write_text("<textarea id='arch-data'>sensitive</textarea>")
            self.assertIsNone(state.report("pr-1"));self.assertEqual(state.reports(),[])
    def test_revision_does_not_invent_provider_state_and_keeps_selected_baseline(self):
        with tempfile.TemporaryDirectory() as temporary:
            state=module.State(Path(temporary),Path(temporary));state.repository_slug=lambda:"owner/repo"
            Handler=module.handler(state);request=Handler.__new__(Handler)
            report={"id":"pr-1-"+"c"*64,"number":1,"observation":{"baseline":"a"*40,"proposal":"b"*40},"architecture":{},"artifact":{"scope_version":"scope","selected_baseline":"d"*40}}
            response=request.workspace_json(report,"/v1/reviews/"+report["id"],{})
            self.assertEqual(response["revision"]["state"],"unknown")
            self.assertEqual(response["selected_baseline"],"d"*40)
            self.assertEqual(response["analysis_baseline"],"a"*40)
            state.provider_status_cache={"observed_at_ms":123,"prs":[{"number":1,"state":"MERGED","isDraft":False}]}
            response=request.workspace_json(report,"/v1/reviews/"+report["id"],{})
            self.assertEqual(response["revision"]["state"],"merged")
            self.assertEqual(response["revision"]["observed_at_ms"],123)
if __name__=="__main__":unittest.main()
