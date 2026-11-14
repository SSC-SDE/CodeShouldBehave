"""The loopback development host serves only the approved embedded Focus assets."""
import io
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import github_mvp_server as host


class WorkspaceAssetsTests(unittest.TestCase):
    def request(self, name, allowed=True):
        cls = host.handler(SimpleNamespace())
        handler = cls.__new__(cls)
        handler.path = "/assets/" + name
        handler.loopback_origin_allowed = lambda: allowed
        handler.wfile = io.BytesIO()
        responses = []
        handler.send_body_headers = lambda kind, length: responses.append((200, kind, length))
        handler.send_error = lambda status: responses.append((status, None, 0))
        handler.do_GET()
        return responses, handler.wfile.getvalue()

    def test_focus_assets_match_source_and_mime_without_api_authorization(self):
        for name, content_type in (
            ("workspace-focus-model.js", "text/javascript; charset=utf-8"),
            ("workspace-focus.js", "text/javascript; charset=utf-8"),
            ("workspace-focus.css", "text/css; charset=utf-8"),
            ("workspace-moria.js", "text/javascript; charset=utf-8"),
            ("workspace-moria.css", "text/css; charset=utf-8"),
            ("workspace-moria-palette.css", "text/css; charset=utf-8"),
            ("workspace-moria.json", "application/json"),
        ):
            with self.subTest(name=name):
                expected = (host.ROOT / "src" / name).read_bytes()
                responses, body = self.request(name)
                self.assertEqual(responses, [(200, content_type, len(expected))])
                self.assertEqual(body, expected)

    def test_focus_assets_keep_the_loopback_origin_gate(self):
        responses, body = self.request("workspace-moria.json", allowed=False)
        self.assertEqual(responses[0][0], 403)
        self.assertEqual(body, b"")


if __name__ == "__main__":
    unittest.main()
