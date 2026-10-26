"""Bounded Pages diagnostics and exact signed-intent recovery contracts."""
import base64
import copy
from datetime import datetime, timezone, timedelta
import json
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import patch
from public_ci_test import ci, configuration, run


class PagesRecoveryTests(unittest.TestCase):
    def setUp(self):
        self.config = configuration()
        self.config['operations'].update(analysis=True, publication=True, checks=True)
        self.previous = ci.empty_state(self.config)
        self.previous['budget_activation'] = None
        self.origin = run(self.config)
        publication = {'previous_state': self.previous, 'state': self.previous,
                       'run': self.origin, 'site': {'site_sha256': 'e' * 64}}
        self.subject = ci.begin_promotion(self.config, publication, 44, now='2026-10-17T06:00:00Z')
        self.state = self.subject['state']
        self.current = dict(self.origin, id=222, event='workflow_dispatch')
        self.calls = []
        self.provider = SimpleNamespace(allow_writes=True, prefix='/repos/example/public',
            repository_info=lambda: {}, restore_state=lambda: copy.deepcopy(self.state),
            trusted_run=lambda *args, **kwargs: self.origin, api=self.api)

    def api(self, path, **kwargs):
        self.calls.append((path, kwargs))
        if '/contents/' in path:
            data = ci.canonical(self.config)
            return {'type': 'file', 'encoding': 'base64', 'size': len(data), 'content': base64.b64encode(data).decode()}
        if '/actions/artifacts/' in path:
            return {'name': 'csb-public-pages-111-2', 'expired': False, 'size_in_bytes': 100,
                    'workflow_run': {'id': 111, 'head_sha': 'c' * 40}}
        return {'status': 'succeed'}

    def test_approval_requires_manual_exact_unexpired_state(self):
        now = datetime(2026, 10, 17, 6, tzinfo=timezone.utc)
        approval = {'schema_version': 'csb-pages-recovery-v1', 'repository': self.config['repository'],
            'action': 'resume_exact_intent', 'state_sha256': ci.digest(ci.canonical(self.state)),
            'intent_sha256': ci.digest(ci.canonical(self.state['promotion_intent'])),
            'expires_at': '2026-10-17T07:00:00Z'}
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder); (root / '.codeshouldbehave').mkdir()
            file = root / '.codeshouldbehave/pages-recovery.json'
            file.write_text(json.dumps(approval))
            self.assertEqual(ci.recovery_authorization(self.config, root, self.state, self.current, now), approval)
            for state, current, when in [(self.state, self.origin, now),
                (dict(self.state, sequence=99), self.current, now),
                (self.state, self.current, now + timedelta(hours=2))]:
                with self.assertRaises(ci.Refusal):
                    ci.recovery_authorization(self.config, root, state, current, when)

    def test_succeeded_recovery_requires_readback_and_preserves_chain(self):
        with patch.object(ci, 'restore_public_site', return_value=Path('verified')) as readback:
            final = ci.recover_pages_intent(self.config, self.state, self.current, provider=self.provider)
        readback.assert_called_once()
        self.assertEqual(final['kind'], 'site_promotion')
        self.assertEqual(final['state']['previous_receipt_sha256'], ci.digest(ci.canonical(self.state)))
        self.assertEqual(final['state']['sequence'], self.state['sequence'] + 1)
        self.assertEqual(final['state']['withdrawn'], self.state['withdrawn'])
        self.assertEqual(final['state']['pending'], self.state['pending'])
        self.assertIsNone(final['state']['promotion_intent'])
        self.assertFalse(any(k.get('method') == 'POST' for _, k in self.calls))

    def test_failed_readback_preserves_original_intent(self):
        saved = copy.deepcopy(self.state)
        with patch.object(ci, 'restore_public_site', side_effect=ci.Refusal('digest mismatch')):
            with self.assertRaises(ci.Refusal):
                ci.recover_pages_intent(self.config, self.state, self.current, provider=self.provider)
        self.assertEqual(self.state, saved)

    def test_changed_scope_refuses_before_deployment(self):
        changed = copy.deepcopy(self.config); changed['policy_sha256'] = 'f' * 64
        with self.assertRaisesRegex(ci.Refusal, 'changed scope'):
            ci.recover_pages_intent(changed, self.state, self.current, provider=self.provider)
        self.assertFalse(any('/pages/' in p for p, _ in self.calls))

    def test_status_404_is_bounded_without_reposting(self):
        statuses = iter([None, {'status': ''}, {'status': 'succeed'}])
        def api(path, **kwargs):
            if '/actions/artifacts/' in path:
                return self.api(path, **kwargs)
            self.calls.append((path, kwargs))
            if kwargs.get('method') == 'POST':
                return {'id': 'known-deployment'}
            return next(statuses)
        self.provider.api = api
        result = ci.deploy_pages(self.config, self.subject, provider=self.provider,
                                 token=lambda: 'SECRET', sleep=lambda _: None)
        self.assertEqual(result['status'], 'succeed')
        self.assertEqual(sum(k.get('method') == 'POST' for _, k in self.calls), 1)

    def test_diagnostics_hide_provider_body_and_token(self):
        provider = ci.GitHub(self.config, allow_writes=True, transport=lambda *args: (404, {'message': 'SECRET'}))
        with self.assertRaisesRegex(ci.Refusal, 'pages_create: HTTP 404') as failure:
            provider.api(provider.prefix + '/pages/deployments', method='POST', body={'oidc_token': 'SECRET'})
        self.assertNotIn('SECRET', str(failure.exception))

    def test_resume_uses_existing_artifact_and_version_only(self):
        initial = True
        def api(path, **kwargs):
            nonlocal initial
            if '/contents/' in path or '/actions/artifacts/' in path:
                return self.api(path, **kwargs)
            self.calls.append((path, kwargs))
            if kwargs.get('method') == 'POST':
                self.assertEqual(kwargs['body']['artifact_id'], 44)
                self.assertEqual(kwargs['body']['pages_build_version'], self.state['promotion_intent']['pages_build_version'])
                return {'id': 'recovered-version'}
            if initial:
                initial = False
                return None
            return {'status': 'succeed'}
        self.provider.api = api
        with patch.object(ci, 'restore_public_site', return_value=Path('verified')):
            final = ci.recover_pages_intent(self.config, self.state, self.current,
                provider=self.provider, token=lambda: 'SECRET', sleep=lambda _: None)
        self.assertEqual(final['state']['site_artifact_id'], 44)
        self.assertEqual(sum(k.get('method') == 'POST' for _, k in self.calls), 1)

    def test_expired_artifact_never_recreates_or_clears_intent(self):
        def api(path, **kwargs):
            value = self.api(path, **kwargs)
            if '/actions/artifacts/' in path:
                value['expired'] = True
            if '/pages/deployments/' in path:
                return None
            return value
        self.provider.api = api
        before = copy.deepcopy(self.state)
        with self.assertRaisesRegex(ci.Refusal, 'artifact'):
            ci.recover_pages_intent(self.config, self.state, self.current,
                provider=self.provider, token=lambda: 'SECRET', sleep=lambda _: None)
        self.assertEqual(before, self.state)
        self.assertFalse(any(k.get('method') == 'POST' for _, k in self.calls))

    def test_concurrent_receipt_refuses_promotion(self):
        states = iter([copy.deepcopy(self.state), dict(self.state, sequence=999)])
        self.provider.restore_state = lambda: next(states)
        with patch.object(ci, 'restore_public_site', return_value=Path('verified')):
            with self.assertRaisesRegex(ci.Refusal, 'state changed'):
                ci.recover_pages_intent(self.config, self.state, self.current, provider=self.provider)

    def test_recovery_plan_has_no_analysis_and_stage_revalidates_authorization(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.provider.trusted_run = lambda *args, **kwargs: self.current
            self.provider.calls = 0
            approval = {'fixture': 'exact authorization'}
            with patch.object(ci, 'load_config'), patch.object(ci, 'verify_installed_inputs'), \
                 patch.object(ci, 'budget_activation'), patch.object(ci, 'public_policy_digest'), \
                 patch.object(ci, 'recovery_authorization', return_value=approval) as authorize, \
                 patch.dict(ci.os.environ, {'GITHUB_RUN_ID': '222'}):
                plan = ci.github_plan(self.config, root, root / 'plan.json', provider=self.provider)
                self.assertEqual(plan['requests'], [])
                final = {'state': dict(self.state)}
                with patch.object(ci, 'recover_pages_intent', return_value=final) as resume:
                    staged = ci.stage_publication(self.config, plan, root / 'handoffs', root / 'out',
                        provider=self.provider, project_root=root, trusted_root=root, verifier=root / 'verifier')
                self.assertIsNone(staged['site'])
                self.assertEqual(staged['recovery_final'], final)
                self.assertEqual(authorize.call_count, 2)
                resume.assert_called_once()
