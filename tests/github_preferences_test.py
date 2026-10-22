"""Profile adapter uses server-established numeric identity, never a browser target."""
import io
import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"scripts"))
import github_mvp_server as host

class PreferencesTests(unittest.TestCase):
    def request(self, body=None, authorized=True, app=True):
        calls=[]
        state=SimpleNamespace(repository_slug=lambda:'owner/repo',app_auth=SimpleNamespace(access=lambda slug:{'principal':'github-user:7'}) if app else None)
        state.native_onboarding_request=lambda *args:(calls.append(args) or (200,{'revision':1}))
        cls=host.handler(state);handler=cls.__new__(cls)
        raw=json.dumps(body or {}).encode();handler.headers={'Content-Length':str(len(raw))};handler.rfile=io.BytesIO(raw)
        handler.api_authorized=lambda:authorized
        responses=[];handler.json_status=lambda status,value:responses.append((status,value));handler.send_error=lambda status:responses.append((status,None))
        return handler,calls,responses
    def test_numeric_account_forwarded_to_native_storage(self):
        handler,calls,responses=self.request({'revision':0,'preferences':{}})
        handler.preferences(write=True)
        self.assertEqual(calls[0][0],'preferences-write');self.assertEqual(calls[0][4],'github-user:7');self.assertEqual(responses[0][0],200)
    def test_missing_auth_and_cli_fallback_do_not_read_profiles(self):
        for authorized,app,expected in [(False,True,403),(True,False,401)]:
            handler,calls,responses=self.request(authorized=authorized,app=app);handler.preferences()
            self.assertEqual(responses[0][0],expected);self.assertFalse(calls)
    def test_bounded_body_before_native_invocation(self):
        handler,calls,responses=self.request();handler.headers['Content-Length']='65537';handler.preferences(write=True)
        self.assertEqual(responses[0][0],400);self.assertFalse(calls)
