"""Local GitHub App credential broker. Tokens never enter analysis or browser JSON."""
from __future__ import annotations

import base64
import json
import os
from pathlib import Path
import re
import signal
import ssl
import subprocess
import threading
import time
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode
from urllib.request import HTTPSHandler, HTTPRedirectHandler, ProxyHandler, Request, build_opener


class AppError(Exception):
    """Only stable public error codes; never attach response bodies or tokens."""


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class Transport:
    def __init__(self):
        self.opener = build_opener(ProxyHandler({}), NoRedirect(), HTTPSHandler(context=ssl.create_default_context()))

    def request(self, method, url, *, data=None, token=None):
        if not (url.startswith("https://api.github.com/") or url in
                {"https://github.com/login/device/code", "https://github.com/login/oauth/access_token"}):
            raise AppError("unsupported_destination")
        headers = {"Accept": "application/json", "User-Agent": "CodeShouldBehave", "X-GitHub-Api-Version": "2026-03-10"}
        if token: headers["Authorization"] = f"Bearer {token}"
        body = urlencode(data).encode() if data is not None else None
        try:
            with self.opener.open(Request(url, data=body, headers=headers, method=method), timeout=15) as response:
                raw = response.read(2 * 1024 * 1024 + 1)
                if len(raw) > 2 * 1024 * 1024: raise AppError("provider_response_too_large")
                value = json.loads(raw)
                if not isinstance(value, (dict,list)): raise AppError("invalid_provider_response")
                return value
        except HTTPError as error:
            if error.code==401: raise AppError("github_authentication_required") from None
            if error.code in (403,429) and (error.headers.get("X-RateLimit-Remaining") == "0" or error.headers.get("Retry-After")):
                raise AppError("github_rate_limited") from None
            raise AppError("github_access_unavailable") from None
        except (URLError, OSError, ValueError):
            raise AppError("github_unavailable") from None


def config(path):
    try:
        source = Path(path)
        if source.is_symlink() or source.stat().st_size > 4096: raise ValueError
        value = json.loads(source.read_text())
        if set(value) != {"schema_version","app_id","slug","client_id"}: raise ValueError
        if value["schema_version"] != "csb-github-app-config-v1": raise ValueError
        if type(value["app_id"]) is not int or value["app_id"] <= 0: raise ValueError
        if not re.fullmatch(r"[a-z0-9-]{1,64}",value["slug"]): raise ValueError
        if not re.fullmatch(r"Iv[0-9A-Za-z]{10,64}",value["client_id"]): raise ValueError
        return value
    except (OSError,ValueError,TypeError):
        raise AppError("invalid_public_app_configuration") from None


class DeviceAuth:
    def __init__(self, settings, transport=None, clock=time.monotonic):
        self.settings = settings
        self.transport = transport or Transport()
        self.clock = clock
        self.lock = threading.Lock()
        self.generation = 0
        self._token = None
        self._token_deadline = 0
        self._device_code = None
        self._device_deadline = 0
        self._next_poll = 0
        self._interval = 5
        self._user_code = None
        self._identity = None
        self.error = None

    def logout(self):
        with self.lock:
            self.generation += 1
            self._token = self._device_code = self._user_code = self._identity = None
            self.error = None

    def public_status(self):
        with self.lock:
            authenticated = bool(self._token and self.clock() < self._token_deadline)
            pending = bool(self._device_code and self.clock() < self._device_deadline)
            return {"authenticated":authenticated,"auth_running":pending,"auth_code":self._user_code if pending else None,
                    "verification_url":"https://github.com/login/device" if pending else None,
                    "auth_error":self.error,"auth_profile":"github-app","app_id":self.settings["app_id"],
                    "install_url":f"https://github.com/apps/{self.settings['slug']}/installations/new"}

    def begin(self):
        self.logout()
        with self.lock: generation = self.generation
        value = self.transport.request("POST","https://github.com/login/device/code",data={"client_id":self.settings["client_id"]})
        if not isinstance(value,dict): raise AppError("invalid_device_response")
        if "error" in value:
            raise AppError("device_flow_disabled" if value["error"]=="device_flow_disabled" else "github_authorization_unavailable")
        if (not isinstance(value.get("device_code"),str) or not re.fullmatch(r"[A-Za-z0-9_-]{20,512}",value["device_code"])
            or not isinstance(value.get("user_code"),str) or not re.fullmatch(r"[A-Z0-9]{4}-[A-Z0-9]{4}",value["user_code"])
            or value.get("verification_uri") != "https://github.com/login/device"
            or type(value.get("expires_in")) is not int or not 1 <= value["expires_in"] <= 900
            or type(value.get("interval")) is not int or not 1 <= value["interval"] <= 60):
            raise AppError("invalid_device_response")
        with self.lock:
            if generation != self.generation: return
            self._device_code, self._user_code = value["device_code"], value["user_code"]
            self._device_deadline = self.clock() + value["expires_in"]
            self._interval = value["interval"]
            self._next_poll = self.clock() + self._interval

    def poll_once(self):
        with self.lock:
            if not self._device_code: return False
            if self.clock() >= self._device_deadline:
                self._device_code = self._user_code = None
                self.error = "expired_device_code"
                return False
            if self.clock() < self._next_poll: return True
            generation, code = self.generation, self._device_code
            self._next_poll = self.clock() + self._interval
        value = self.transport.request("POST","https://github.com/login/oauth/access_token",
            data={"client_id":self.settings["client_id"],"device_code":code,"grant_type":"urn:ietf:params:oauth:grant-type:device_code"})
        if not isinstance(value,dict): raise AppError("invalid_token_response")
        with self.lock:
            if generation != self.generation: return False
            error = value.get("error")
            if error == "authorization_pending": return True
            if error == "slow_down":
                self._interval += 5
                self._next_poll = self.clock() + self._interval
                return True
            self._device_code = self._user_code = None
            if error:
                self.error = error if error in {"access_denied","expired_token","device_flow_disabled"} else "github_authorization_unavailable"
                return False
            token = value.get("access_token")
            if (not isinstance(token,str) or not re.fullmatch(r"ghu_[A-Za-z0-9_]{20,512}",token)
                or value.get("token_type")!="bearer" or value.get("scope")!=""
                or type(value.get("expires_in")) is not int or not 1<=value["expires_in"]<=28800):
                self.error = "invalid_token_response"
                return False
            self._token, self._token_deadline = token, self.clock()+value["expires_in"]
            # Refresh tokens are deliberately discarded; no persistence or refresh-secret broker.
            return False

    def api(self, path, object_only=True):
        if not re.fullmatch(r"/[A-Za-z0-9_./?=&-]{1,512}",path) or path.startswith("//") or ".." in path:
            raise AppError("unsupported_destination")
        with self.lock:
            generation, token = self.generation, self._token
            if not token or self.clock()>=self._token_deadline: raise AppError("github_authentication_required")
        try:
            value = self.transport.request("GET",f"https://api.github.com{path}",token=token)
        except AppError as error:
            if str(error)=="github_authentication_required":
                with self.lock:
                    if generation==self.generation and token==self._token:
                        self.generation+=1
                        self._token=self._identity=None
                        self.error="github_authentication_required"
            raise
        if object_only and not isinstance(value,dict): raise AppError("invalid_provider_response")
        with self.lock:
            if generation!=self.generation or token!=self._token: raise AppError("github_authentication_required")
        return value

    def access(self, slug):
        if not isinstance(slug,str) or not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+",slug): raise AppError("repository_unavailable")
        started, requests = self.clock(), 0
        def bounded_api(path):
            nonlocal requests
            requests += 1
            if requests>20 or self.clock()-started>30: raise AppError("access_check_limit_exceeded")
            return self.api(path)
        # Fresh app installation and viewer intersection on every call, never a UI cache grant.
        with self.lock: identity, generation = self._identity, self.generation
        if not identity:
            user = bounded_api("/user")
            if type(user.get("id")) is not int or user["id"]<=0 or not isinstance(user.get("login"),str) or not re.fullmatch(r"[A-Za-z0-9-]{1,39}",user["login"]):
                raise AppError("invalid_viewer_identity")
            identity = {"principal":f"github-user:{user['id']}","login":user["login"]}
            with self.lock:
                if generation!=self.generation: raise AppError("github_authentication_required")
                self._identity = identity
        for page in range(1,11):
            entries = bounded_api(f"/user/installations?per_page=100&page={page}").get("installations")
            if not isinstance(entries,list) or len(entries)>100: raise AppError("invalid_installation_response")
            for installation in entries:
                if not isinstance(installation,dict): raise AppError("invalid_installation_response")
                if type(installation.get("app_id")) is not int or "suspended_at" not in installation: raise AppError("invalid_installation_response")
                if installation["app_id"]!=self.settings["app_id"] or installation.get("suspended_at") is not None: continue
                permissions = installation.get("permissions",{})
                if not isinstance(permissions,dict): raise AppError("invalid_installation_response")
                if not all(isinstance(permissions.get(key),str) and permissions[key] in {"read","write"} for key in ("contents","pull_requests","metadata")): continue
                installation_id = installation.get("id")
                if type(installation_id) is not int or installation_id<=0: continue
                for repo_page in range(1,11):
                    repos = bounded_api(f"/user/installations/{installation_id}/repositories?per_page=100&page={repo_page}").get("repositories")
                    if not isinstance(repos,list) or len(repos)>100: raise AppError("invalid_repository_response")
                    for repo in repos:
                        if not isinstance(repo,dict): raise AppError("invalid_repository_response")
                        if not isinstance(repo.get("full_name"),str): raise AppError("invalid_repository_response")
                        if repo["full_name"].lower()!=slug.lower(): continue
                        user_permissions = repo.get("permissions",{})
                        if not isinstance(user_permissions,dict) or user_permissions.get("pull") is not True or type(repo.get("id")) is not int or repo["id"]<=0: raise AppError("repository_unavailable")
                        with self.lock:
                            if generation!=self.generation or not self._token or self.clock()>=self._token_deadline:
                                raise AppError("github_authentication_required")
                        return {**identity,"repository":slug,"repository_id":repo["id"],"installation_id":installation_id,
                                "maintainer":any(user_permissions.get(key) is True for key in ("push","maintain","admin"))}
                    if len(repos)<100: break
            if len(entries)<100: break
        raise AppError("repository_unavailable")

    def _verify_pull_revision(self, slug, pr, baseline, proposal, access):
        """A fork is inert metadata: fetch only its PR ref on the installed base."""
        value = self.api(f"/repos/{slug}/pulls/{pr}")
        base, head = value.get("base"), value.get("head")
        if (type(value.get("number")) is not int or value["number"] != pr
            or not isinstance(base,dict) or not isinstance(head,dict)
            or not isinstance(base.get("repo"),dict)):
            raise AppError("invalid_pr_response")
        repository = base["repo"]
        if (type(repository.get("id")) is not int or repository["id"] != access["repository_id"]
            or not isinstance(repository.get("full_name"),str)
            or repository["full_name"].lower() != slug.lower()):
            raise AppError("repository_unavailable")
        if not all(isinstance(side.get("sha"),str) and re.fullmatch(r"[0-9a-f]{40}",side["sha"]) for side in (base,head)):
            raise AppError("invalid_revision")
        if base["sha"] != baseline or head["sha"] != proposal:
            raise AppError("superseded_revision")

    def fetch_commits(self, root, slug, pr, baseline, proposal):
        access = self.access(slug)
        if type(pr) is not int or pr<1 or not all(isinstance(value,str) and re.fullmatch(r"[0-9a-f]{40}",value) for value in (baseline,proposal)):
            raise AppError("invalid_revision")
        with self.lock:
            if not self._token or self.clock()>=self._token_deadline: raise AppError("github_authentication_required")
            token, generation = self._token, self.generation
        self._verify_pull_revision(slug,pr,baseline,proposal,access)
        root = Path(root)
        # Reusing a checkout can inherit repository-local hooks/configuration.
        # The broker creates only an empty, isolated bare object store.
        if root.is_symlink() or (root.exists() and (not root.is_dir() or any(root.iterdir()))):
            raise AppError("invalid_fetch_workspace")
        env = {key:os.environ[key] for key in ("PATH","TMPDIR","LANG") if key in os.environ}
        env.update({"GIT_TERMINAL_PROMPT":"0","GIT_CONFIG_NOSYSTEM":"1","GIT_CONFIG_GLOBAL":os.devnull,
            "GIT_CONFIG_COUNT":"4","GIT_CONFIG_KEY_0":"credential.helper","GIT_CONFIG_VALUE_0":"",
            "GIT_CONFIG_KEY_1":"http.followRedirects","GIT_CONFIG_VALUE_1":"false",
            "GIT_CONFIG_KEY_2":"http.sslVerify","GIT_CONFIG_VALUE_2":"true",
            "GIT_CONFIG_KEY_3":"http.proxy","GIT_CONFIG_VALUE_3":""})
        fetch_env = {**env,"GIT_CONFIG_COUNT":"5",
            "GIT_CONFIG_KEY_4":f"http.https://github.com/{slug}.git.extraHeader",
            "GIT_CONFIG_VALUE_4":"AUTHORIZATION: basic "+base64.b64encode(f"x-access-token:{token}".encode()).decode()}
        try:
            subprocess.run(["git","init","--bare","--template=",str(root)],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=10,check=True)
            child=subprocess.Popen(["git","-C",str(root),"-c","core.hooksPath=/dev/null","-c","protocol.ext.allow=never","-c","protocol.file.allow=never",
                "fetch","--depth=128","--no-tags","--no-recurse-submodules","--no-auto-maintenance",f"https://github.com/{slug}.git",
                f"{baseline}:refs/csb/base",f"pull/{pr}/head:refs/csb/proposal"],env=fetch_env,
                stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,start_new_session=True)
            started=time.monotonic()
            try:
                while child.poll() is None:
                    with self.lock:
                        if generation!=self.generation or token!=self._token or self.clock()>=self._token_deadline:
                            raise AppError("github_authentication_required")
                    check_fetch_budget(root)
                    if time.monotonic()-started>60: raise AppError("fetch_timeout")
                    time.sleep(.05)
                if child.returncode: raise AppError("github_fetch_unavailable")
            finally:
                if child.poll() is None:
                    # Git can have a HTTPS helper child holding its credential.
                    # Cancel the isolated process group, not only the parent.
                    try: os.killpg(child.pid,signal.SIGKILL)
                    except OSError: child.kill()
                child.wait()
            for reference, expected in [("refs/csb/base",baseline),("refs/csb/proposal",proposal)]:
                actual = subprocess.check_output(["git","-C",str(root),"rev-parse","--verify",reference+"^{commit}"],env=env,timeout=5,text=True).strip()
                if actual!=expected: raise AppError("superseded_revision")
            check_fetch_budget(root)
        except (OSError,subprocess.SubprocessError):
            raise AppError("github_fetch_unavailable") from None
        current = self.access(slug)
        if any(current[key] != access[key] for key in ("principal","repository_id","installation_id")):
            raise AppError("repository_unavailable")
        self._verify_pull_revision(slug,pr,baseline,proposal,current)
        with self.lock:
            if generation!=self.generation or token!=self._token or self.clock()>=self._token_deadline:
                raise AppError("github_authentication_required")


def check_fetch_budget(root):
    size=0
    for count,item in enumerate(Path(root).rglob("*"),1):
        if count>20000: raise AppError("fetch_budget_exceeded")
        if item.is_file(): size+=item.stat().st_size
        if size>256*1024*1024: raise AppError("fetch_budget_exceeded")
