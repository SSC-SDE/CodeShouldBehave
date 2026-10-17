"""Sanitized OAuth/installation contracts. No live credentials or network."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
import base64
from unittest.mock import patch
from types import SimpleNamespace
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"scripts"))
from github_app_auth import AppError,DeviceAuth,Transport,config,check_fetch_budget

SETTINGS={"schema_version":"csb-github-app-config-v1","app_id":5203296,"slug":"codeshouldbehave","client_id":"Iv23liFILwUL4090Zfys"}
DEVICE={"device_code":"d"*40,"user_code":"ABCD-EFGH","verification_uri":"https://github.com/login/device","expires_in":900,"interval":5}
TOKEN={"access_token":"ghu_"+"TOKEN_SENTINEL"*3,"refresh_token":"REFRESH_SENTINEL","expires_in":28800,"token_type":"bearer","scope":""}

class Fake:
    def __init__(self,values): self.values=list(values);self.calls=[]
    def request(self,method,url,**kwargs):
        self.calls.append((method,url,kwargs))
        result=self.values.pop(0)
        if callable(result): return result()
        if isinstance(result,Exception): raise result
        return result

class AuthTest(unittest.TestCase):
    def auth(self,values):
        clock=[0];transport=Fake(values);auth=DeviceAuth(SETTINGS,transport,lambda:clock[0]);return auth,transport,clock
    def login(self,extra=None):
        auth,transport,clock=self.auth([DEVICE,TOKEN]+(extra or []));auth.begin();clock[0]=5;self.assertFalse(auth.poll_once());return auth,transport,clock
    def test_polling_respects_intervals_and_slow_down_without_exposing_secrets(self):
        auth,transport,clock=self.auth([DEVICE,{"error":"authorization_pending"},{"error":"slow_down"},TOKEN]);auth.begin()
        self.assertTrue(auth.poll_once());self.assertEqual(len(transport.calls),1)
        clock[0]=5;self.assertTrue(auth.poll_once());clock[0]=10;self.assertTrue(auth.poll_once())
        clock[0]=15;self.assertTrue(auth.poll_once());self.assertEqual(len(transport.calls),3)
        clock[0]=20;self.assertFalse(auth.poll_once())
        status=json.dumps(auth.public_status());self.assertIn('true',status)
        for value in ['TOKEN_SENTINEL','REFRESH_SENTINEL','d'*40]: self.assertNotIn(value,status)
        self.assertNotIn('client_secret',transport.calls[-1][2]['data'])
        clock[0]=28821;self.assertFalse(auth.public_status()['authenticated'])
        with self.assertRaises(AppError):auth.api('/user')
    def test_cancelled_or_expired_device_flow_cannot_authenticate(self):
        auth,_,clock=self.auth([DEVICE,{'error':'access_denied'}]);auth.begin();clock[0]=5;self.assertFalse(auth.poll_once());self.assertFalse(auth.public_status()['authenticated'])
        auth,transport,clock=self.auth([DEVICE]);auth.begin();clock[0]=901;self.assertFalse(auth.poll_once());self.assertEqual(len(transport.calls),1)
    def test_foreign_verification_destination_and_nonexpiring_token_are_rejected(self):
        auth,_,_=self.auth([{**DEVICE,'verification_uri':'https://attacker.invalid'}])
        with self.assertRaises(AppError):auth.begin()
        auth,_,clock=self.auth([DEVICE,{**TOKEN,'expires_in':None}]);auth.begin();clock[0]=5;auth.poll_once();self.assertFalse(auth.public_status()['authenticated'])
    def test_logout_fences_late_auth_responses(self):
        auth,transport,clock=self.auth([DEVICE]);auth.begin();clock[0]=5
        transport.values.append(lambda:(auth.logout() or TOKEN));auth.poll_once();self.assertFalse(auth.public_status()['authenticated'])
    def installation(self,**extra):
        return {"id":12,"app_id":5203296,"suspended_at":None,"permissions":{"contents":"read","pull_requests":"read","metadata":"read"},**extra}
    def repo(self,**extra):
        return {"id":34,"full_name":"SSC-SDE/codeshouldbehave","permissions":{"pull":True,"push":False},**extra}
    def pull(self,**extra):
        return {"number":33,"base":{"sha":"a"*40,"repo":self.repo()},
                "head":{"sha":"b"*40,"repo":{"id":50,"full_name":"fork/repo","clone_url":"https://attacker.invalid/TOKEN_SENTINEL"},
                        "ref":"--upload-pack=hostile"},**extra}
    def grant(self):
        return {'principal':'github-user:99','repository_id':34,'installation_id':12}
    def test_access_requires_matching_app_installation_and_current_viewer_repo_permission(self):
        auth,transport,_=self.login([{'id':99,'login':'viewer'},{'installations':[self.installation()]},{'repositories':[self.repo()]}])
        access=auth.access('SSC-SDE/codeshouldbehave');self.assertEqual(access['principal'],'github-user:99');self.assertFalse(access['maintainer'])
        transport.values.extend([{'installations':[self.installation()]},{'repositories':[]}])
        with self.assertRaisesRegex(AppError,'repository_unavailable'):auth.access('SSC-SDE/codeshouldbehave')
        self.assertNotIn('/repos/SSC-SDE/codeshouldbehave',str(transport.calls))
    def test_suspended_wrong_app_and_missing_contents_permission_do_not_grant_access(self):
        for item in [self.installation(app_id=1),self.installation(suspended_at='now'),self.installation(permissions={'metadata':'read'})]:
            auth,_,_=self.login([{'id':99,'login':'viewer'},{'installations':[item]}])
            with self.assertRaises(AppError):auth.access('SSC-SDE/codeshouldbehave')
    def test_account_change_during_metadata_requests_does_not_mix_identity_and_grants(self):
        auth,transport,_=self.login([{'id':99,'login':'viewer'},{'installations':[self.installation()]}])
        transport.values.append(lambda:(auth.logout() or {'repositories':[self.repo()]}))
        with self.assertRaises(AppError):auth.access('SSC-SDE/codeshouldbehave')
    def test_public_config_rejects_secrets_and_transport_rejects_arbitrary_urls(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'app.json';path.write_text(json.dumps(SETTINGS));self.assertEqual(config(path),SETTINGS)
            path.write_text(json.dumps({**SETTINGS,'client_secret':'SECRET_SENTINEL'}))
            with self.assertRaises(AppError):config(path)
        with self.assertRaises(AppError):Transport().request('GET','https://attacker.invalid')
        auth,_,_=self.login()
        with self.assertRaises(AppError):auth.api('//attacker.invalid')

    def test_malformed_permissions_and_device_responses_fail_closed(self):
        auth,_,_=self.auth([[]])
        with self.assertRaisesRegex(AppError,'invalid_device_response'):auth.begin()
        auth,_,_=self.login([{'id':99,'login':'viewer'},{'installations':[self.installation(permissions=None)]}])
        with self.assertRaisesRegex(AppError,'invalid_installation_response'):auth.access('SSC-SDE/codeshouldbehave')
        auth,_,_=self.login([{'id':99,'login':'viewer'},{'installations':[self.installation()]},{'repositories':[self.repo(permissions=None)]}])
        with self.assertRaisesRegex(AppError,'repository_unavailable'):auth.access('SSC-SDE/codeshouldbehave')
        missing_suspension=self.installation();del missing_suspension['suspended_at']
        for installation in [self.installation(app_id=[]),missing_suspension,self.installation(permissions={'contents':[],'pull_requests':'read','metadata':'read'})]:
            auth,_,_=self.login([{'id':99,'login':'viewer'},{'installations':[installation]}])
            with self.assertRaises(AppError):auth.access('SSC-SDE/codeshouldbehave')
        auth,_,_=self.login([{'id':99,'login':'viewer'},{'installations':[self.installation()]},{'repositories':[self.repo(full_name=[])]}])
        with self.assertRaisesRegex(AppError,'invalid_repository_response'):auth.access('SSC-SDE/codeshouldbehave')

    def test_installation_enumeration_stops_at_request_budget(self):
        auth,transport,_=self.login([{'id':99,'login':'viewer'},{'installations':[self.installation(id=i+1) for i in range(30)]}]+[{'repositories':[]}]*30)
        before=len(transport.calls)
        with self.assertRaisesRegex(AppError,'access_check_limit_exceeded'):auth.access('SSC-SDE/codeshouldbehave')
        self.assertEqual(len(transport.calls)-before,20)

    def test_revoked_token_clears_authentication_without_erasing_a_new_session(self):
        auth,transport,_=self.login([AppError('github_authentication_required')])
        with self.assertRaises(AppError):auth.api('/user')
        self.assertFalse(auth.public_status()['authenticated'])
        auth,transport,_=self.login()
        def newer_login_then_old_failure():
            auth.logout()
            with auth.lock:
                auth._token='ghu_'+'NEW_SESSION_SENTINEL'*2
                auth._token_deadline=100
            raise AppError('github_authentication_required')
        transport.values.append(newer_login_then_old_failure)
        with self.assertRaises(AppError):auth.api('/user')
        self.assertTrue(auth.public_status()['authenticated'])
        self.assertIsNone(auth.public_status()['auth_error'])
        self.assertNotIn('NEW_SESSION_SENTINEL',json.dumps(auth.public_status()))

    def test_fetch_credential_is_repository_scoped_environment_only(self):
        auth,transport,_=self.login([self.pull(),self.pull()]);auth.access=lambda slug:self.grant()
        baseline,proposal='a'*40,'b'*40
        with tempfile.TemporaryDirectory() as directory,patch.dict('github_app_auth.os.environ',{'GH_TOKEN':'AMBIENT_SECRET','OPENAI_API_KEY':'AMBIENT_SECRET','GIT_SSH_COMMAND':'AMBIENT_SECRET','GIT_TEMPLATE_DIR':'AMBIENT_SECRET','GIT_CONFIG_PARAMETERS':'AMBIENT_SECRET'}),patch('github_app_auth.subprocess.run') as init,patch('github_app_auth.subprocess.Popen') as fetch,patch('github_app_auth.subprocess.check_output',side_effect=[baseline+'\n',proposal+'\n']) as resolve:
            fetch.return_value=SimpleNamespace(poll=lambda:0,returncode=0,wait=lambda:0)
            auth.fetch_commits(Path(directory),'SSC-SDE/codeshouldbehave',33,baseline,proposal)
            self.assertNotIn('TOKEN_SENTINEL',str(fetch.call_args.args))
            self.assertNotIn('TOKEN_SENTINEL',str(init.call_args.args))
            self.assertIn('--template=',init.call_args.args[0])
            command=fetch.call_args.args[0]
            self.assertIn('https://github.com/SSC-SDE/codeshouldbehave.git',command)
            self.assertIn('pull/33/head:refs/csb/proposal',command)
            self.assertIn('--no-recurse-submodules',command)
            self.assertIn('--no-auto-maintenance',command)
            self.assertIn('core.hooksPath=/dev/null',command)
            self.assertIs(fetch.call_args.kwargs['start_new_session'],True)
            self.assertNotIn('attacker.invalid',str(command));self.assertNotIn('upload-pack',str(command))
            environment=fetch.call_args.kwargs['env']
            self.assertEqual(environment['GIT_CONFIG_KEY_4'],'http.https://github.com/SSC-SDE/codeshouldbehave.git.extraHeader')
            self.assertEqual(environment['GIT_CONFIG_VALUE_4'],'AUTHORIZATION: basic '+base64.b64encode(('x-access-token:'+TOKEN['access_token']).encode()).decode())
            self.assertEqual(environment['GIT_CONFIG_VALUE_0'],'')
            for worker in [init.call_args.kwargs['env'],resolve.call_args.kwargs['env']]:
                self.assertNotIn('AUTHORIZATION',str(worker));self.assertNotIn('TOKEN_SENTINEL',str(worker))
            self.assertNotIn('AMBIENT_SECRET',str(environment))
            self.assertEqual(list(Path(directory).iterdir()),[])
        self.assertEqual([call[1] for call in transport.calls[2:]],['https://api.github.com/repos/SSC-SDE/codeshouldbehave/pulls/33']*2)

    def test_malformed_or_foreign_pull_metadata_cannot_start_fetch(self):
        for metadata,code in [
            (self.pull(number=True),'invalid_pr_response'),
            (self.pull(base=None),'invalid_pr_response'),
            (self.pull(base={'sha':'a'*40,'repo':self.repo(id=999)}),'repository_unavailable'),
            (self.pull(base={'sha':'a'*40,'repo':self.repo(full_name='foreign/private')}),'repository_unavailable'),
            (self.pull(head={'sha':'main'}),'invalid_revision'),
            (self.pull(head={'sha':'c'*40}),'superseded_revision'),
        ]:
            with self.subTest(code=code),tempfile.TemporaryDirectory() as directory,patch('github_app_auth.subprocess.run') as init,patch('github_app_auth.subprocess.Popen') as fetch:
                auth,_,_=self.login([metadata]);auth.access=lambda slug:self.grant()
                with self.assertRaisesRegex(AppError,code):auth.fetch_commits(Path(directory),'SSC-SDE/codeshouldbehave',33,'a'*40,'b'*40)
                init.assert_not_called();fetch.assert_not_called()

    def test_existing_checkout_and_symlink_are_rejected_without_fetch(self):
        with tempfile.TemporaryDirectory() as directory,patch('github_app_auth.subprocess.run') as init:
            root=Path(directory);(root/'local-config').write_text('HOSTILE_SENTINEL')
            auth,_,_=self.login([self.pull()]);auth.access=lambda slug:self.grant()
            with self.assertRaisesRegex(AppError,'invalid_fetch_workspace'):auth.fetch_commits(root,'SSC-SDE/codeshouldbehave',33,'a'*40,'b'*40)
            link=root/'mirror';link.symlink_to(root,target_is_directory=True)
            auth,_,_=self.login([self.pull()]);auth.access=lambda slug:self.grant()
            with self.assertRaisesRegex(AppError,'invalid_fetch_workspace'):auth.fetch_commits(link,'SSC-SDE/codeshouldbehave',33,'a'*40,'b'*40)
            init.assert_not_called();self.assertEqual((root/'local-config').read_text(),'HOSTILE_SENTINEL')

    def test_moved_head_or_revoked_installation_fences_fetched_objects(self):
        for after,metadata,code in [
            (self.grant(),self.pull(head={'sha':'c'*40}),'superseded_revision'),
            (AppError('repository_unavailable'),self.pull(),'repository_unavailable'),
            ({**self.grant(),'installation_id':99},self.pull(),'repository_unavailable'),
        ]:
            with self.subTest(code=code),tempfile.TemporaryDirectory() as directory,patch('github_app_auth.subprocess.run'),patch('github_app_auth.subprocess.Popen') as fetch,patch('github_app_auth.subprocess.check_output',side_effect=['a'*40+'\n','b'*40+'\n']):
                auth,_,_=self.login([self.pull(),metadata])
                with patch.object(auth,'access',side_effect=[self.grant(),after]) as access:
                    fetch.return_value=SimpleNamespace(poll=lambda:0,returncode=0,wait=lambda:0)
                    with self.assertRaisesRegex(AppError,code):auth.fetch_commits(Path(directory),'SSC-SDE/codeshouldbehave',33,'a'*40,'b'*40)
                    self.assertEqual(access.call_count,2)

    def test_pr_ref_that_moves_during_fetch_is_rejected(self):
        auth,_,_=self.login([self.pull()]);auth.access=lambda slug:self.grant()
        with tempfile.TemporaryDirectory() as directory,patch('github_app_auth.subprocess.run'),patch('github_app_auth.subprocess.Popen') as fetch,patch('github_app_auth.subprocess.check_output',side_effect=['a'*40+'\n','c'*40+'\n']):
            fetch.return_value=SimpleNamespace(poll=lambda:0,returncode=0,wait=lambda:0)
            with self.assertRaisesRegex(AppError,'superseded_revision'):auth.fetch_commits(Path(directory),'SSC-SDE/codeshouldbehave',33,'a'*40,'b'*40)

    def test_logout_cancels_running_fetch_without_retaining_worker_credentials(self):
        auth,_,_=self.login([self.pull()]);auth.access=lambda slug:self.grant()
        class Child:
            running=True
            returncode=None
            pid=12345
            def poll(self):
                if self.running:auth.logout();return None
                return 0
            def kill(self):self.running=False
            def wait(self):return 0
        child=Child()
        with tempfile.TemporaryDirectory() as directory,patch('github_app_auth.subprocess.run'),patch('github_app_auth.subprocess.Popen',return_value=child),patch('github_app_auth.subprocess.check_output') as resolve,patch('github_app_auth.os.killpg',side_effect=lambda *_:child.kill()) as kill_group:
            with self.assertRaisesRegex(AppError,'github_authentication_required'):auth.fetch_commits(Path(directory),'SSC-SDE/codeshouldbehave',33,'a'*40,'b'*40)
            self.assertFalse(child.running);resolve.assert_not_called()
            kill_group.assert_called_once()
            self.assertNotIn('TOKEN_SENTINEL',json.dumps(auth.public_status()))

    def test_fetch_timeout_and_object_store_size_are_bounded(self):
        auth,_,_=self.login([self.pull()]);auth.access=lambda slug:self.grant()
        child=SimpleNamespace(poll=lambda:None,returncode=None,pid=12345,wait=lambda:0)
        with tempfile.TemporaryDirectory() as directory,patch('github_app_auth.subprocess.run'),patch('github_app_auth.subprocess.Popen',return_value=child),patch('github_app_auth.time.monotonic',side_effect=[0,61]),patch('github_app_auth.os.killpg') as kill_group:
            with self.assertRaisesRegex(AppError,'fetch_timeout'):auth.fetch_commits(Path(directory),'SSC-SDE/codeshouldbehave',33,'a'*40,'b'*40)
            kill_group.assert_called_once()
        with tempfile.TemporaryDirectory() as directory:
            pack=Path(directory)/'objects.pack'
            with pack.open('wb') as sparse:sparse.truncate(256*1024*1024+1)
            with self.assertRaisesRegex(AppError,'fetch_budget_exceeded'):check_fetch_budget(directory)

    def test_optional_hosting_and_model_destinations_have_zero_transport_egress(self):
        transport=Transport()
        with patch.object(transport.opener,'open') as network:
            for destination in ['https://api.vercel.com/v1/projects','https://api.netlify.com/api/v1/sites','https://backboard.railway.app/graphql/v2','https://api.openai.com/v1/responses','http://127.0.0.1:11434/api/chat','https://api.github.com.attacker.invalid/user']:
                with self.subTest(destination=destination),self.assertRaisesRegex(AppError,'unsupported_destination'):
                    transport.request('GET',destination,token=TOKEN['access_token'])
            network.assert_not_called()

if __name__=='__main__':unittest.main()
