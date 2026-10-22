#!/usr/bin/env python3
"""Loopback-only GitHub onboarding and review launcher for the local MVP."""

from __future__ import annotations

import argparse
import fcntl
import html
import hashlib
import json
import os
import pty
import re
import secrets
import select
import stat
import subprocess
import tempfile
import threading
import time
import weakref
from concurrent.futures import ThreadPoolExecutor
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse
from github_app_auth import AppError, DeviceAuth, config as app_config

ROOT = Path(__file__).resolve().parent.parent
LANDING = (ROOT / "src" / "github_mvp.html").read_bytes()
LANDING_JS = (ROOT / "src" / "github_mvp.js").read_bytes()
WORKSPACE = (ROOT / "src" / "workspace.html").read_bytes()
PREFERENCES_JS = (ROOT / "src" / "workspace-preferences.js").read_bytes()
PREFERENCES_REGISTRY = (ROOT / "src" / "workspace-registry.json").read_bytes()
WORKSPACE_JS = (ROOT / "src" / "workspace.js").read_bytes()
WORKSPACE_CSS = (ROOT / "src" / "workspace.css").read_bytes()
ONBOARDING = (ROOT / "src" / "onboarding.html").read_bytes()
ONBOARDING_CSS = (ROOT / "src" / "onboarding.css").read_bytes()
ONBOARDING_JS = (ROOT / "src" / "onboarding.js").read_bytes()
MERMAID = (ROOT / "docs" / "prototypes" / "vendor" / "mermaid.tiny-12.1.0.js").read_bytes()


def architecture_workspace_url(review_id: str, hld_available: bool = False) -> str:
    """Immutable dashboard handoff; moving PR-only identifiers are not links."""
    if not isinstance(review_id, str) or not re.fullmatch(r"pr-[1-9][0-9]*-[0-9a-f]{64}", review_id):
        raise ValueError("invalid_review_identity")
    return f"/workspace#review={review_id}&layout=architecture&side=diff"+("&design=hld" if hld_available else "")


def native_binary(profile: str, name: str) -> Path:
    """Only operator-selected build profiles and fixed project binaries are valid."""
    if profile not in ("debug", "release"):
        raise ValueError("Native profile must be debug or release")
    if name not in ("csb-onboarding", "flowdiff"):
        raise ValueError("Unknown trusted native binary")
    return ROOT / "target" / profile / name


def ensure_native_binaries(profile: str) -> None:
    binaries = [native_binary(profile, name) for name in ("csb-onboarding", "flowdiff")]
    if any(binary.is_symlink() for binary in binaries):
        raise ValueError("Native binary cannot be a symlink")
    if all(binary.is_file() for binary in binaries):
        return
    command = ["cargo", "build", "--locked", "--features", "server", "--bin", "csb-onboarding",
               "--bin", "flowdiff", "--manifest-path", str(ROOT / "Cargo.toml")]
    if profile == "release": command.append("--release")
    subprocess.run(command, cwd=ROOT, check=True)
    if not all(binary.is_file() and not binary.is_symlink() for binary in binaries):
        raise ValueError("Selected trusted native binaries were not built")


class State:
    def __init__(self, repository: Path, output: Path, app_settings=None, onboarding_enabled=True, hld_enabled=True, lld_enabled=True, native_profile="debug"):
        native_binary(native_profile, "csb-onboarding")
        self.native_profile = native_profile
        self.repository = repository.resolve()
        self.output = output.resolve()
        self.output.mkdir(parents=True, mode=0o700, exist_ok=True)
        if self.output.stat().st_uid != os.getuid(): raise ValueError("Output must be owned by the service operator")
        os.chmod(self.output,0o700)
        # One process owns this private output for its entire State lifetime.
        # context_lock then serializes native state operations inside that host.
        # Keep the file object open; finalization/exit releases the advisory lock.
        lock_path=self.output / ".host.lock"
        descriptor=os.open(lock_path,os.O_RDWR|os.O_CREAT|os.O_NOFOLLOW|os.O_CLOEXEC,0o600)
        try:
            metadata=os.fstat(descriptor)
            if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid!=os.getuid() or metadata.st_nlink!=1:
                raise ValueError("Invalid local host lock")
            try: fcntl.flock(descriptor,fcntl.LOCK_EX|fcntl.LOCK_NB)
            except BlockingIOError:
                raise ValueError("Output is already in use by another CodeShouldBehave host") from None
            current=lock_path.lstat()
            if (current.st_dev,current.st_ino)!=(metadata.st_dev,metadata.st_ino):
                raise ValueError("Local host lock changed")
            os.fchmod(descriptor,0o600)
            self._host_lock=os.fdopen(descriptor,"r+b",buffering=0)
            weakref.finalize(self,self._host_lock.close)
            descriptor=None
        finally:
            if descriptor is not None:os.close(descriptor)
        self.session = secrets.token_urlsafe(24)
        self.lock = threading.Lock()
        self.context_lock = threading.RLock()
        self.auth_running = False
        self.auth_code = None
        self.auth_error = None
        self.review_running = False
        self.review_error = None
        self.review_url = None
        self.review_generation = 0
        self.review_pr = None
        self.provider_status_cache: dict = {}
        self.provider_status_until = 0.0
        self.provider_status_lock = threading.Lock()
        self.provider_error: str | None = None
        self.annotations: dict[str, list[dict]] = {}
        self.app_auth = DeviceAuth(app_settings) if app_settings else None
        self.onboarding_enabled = onboarding_enabled
        self.hld_enabled = hld_enabled
        self.lld_enabled = lld_enabled
        # Existence hint only. Native reads still authorize every immutable
        # parent through current accepted context and grants; this is no grant.
        self._lld_parent_registered = set()

    def report(self, review_id: str, artifact_directory: Path | None = None, *, _defer_lld_origin=False) -> dict | None:
        match = re.fullmatch(r"pr-([1-9][0-9]*)(?:-([0-9a-f]{64}))?", review_id)
        if not match:
            return None
        path = (artifact_directory or self.output) / f"pr-{match.group(1)}.json"
        if artifact_directory is None and match.group(2):
            snapshots = self.output / "reviews"
            snapshot = snapshots / review_id
            if snapshots.is_symlink() or snapshot.is_symlink(): return None
            immutable = snapshot / "review.json"
            if immutable.exists(): path = immutable
        if not path.is_file() or path.is_symlink() or path.stat().st_size > 16 * 1024 * 1024:
            return None
        try:
            artifact = json.loads(path.read_text(encoding="utf-8"))
            if artifact.get("artifact_version") != "csb-review-artifact-v1":
                return None
            if not all(re.fullmatch(r"(?:[0-9a-f]{40}|[0-9a-f]{64})", artifact.get(key, "")) for key in ("baseline", "proposal", "selected_baseline")):
                return None
            digest=artifact.get("review_id","")
            if not re.fullmatch(r"[0-9a-f]{64}",digest) or (match.group(2) and match.group(2)!=digest): return None
            identity=artifact["scope_manifest"]["primary_repository"]
            # Only CLI LLD GET routing may defer this duplicate discovery. Its
            # lld_request checks the exact origin under context_lock before any
            # provider/native work. App and ordinary report callers retain it.
            defer_origin=_defer_lld_origin and self.app_auth is None
            if identity.get("provider")!="github" or (not defer_origin and f"{identity.get('owner')}/{identity.get('name')}"!=self.repository_slug()): return None
            files, architecture = artifact["files"], artifact["architecture"]
            if not isinstance(files, list) or len(files)>150 or not isinstance(architecture.get("nodes"),list): return None
            observation = {"baseline": artifact["baseline"], "proposal": artifact["proposal"],
                           "files": [item["path"] for item in files], "warnings": artifact["warnings"]}
            return {"id": f"pr-{match.group(1)}-{digest}", "number": int(match.group(1)), "observation": observation,
                    "architecture": architecture, "files": files, "artifact": artifact, "artifact_path":path}
        except (ValueError, KeyError, TypeError, AttributeError, OSError):
            return None

    def preserve_report(self, report: dict, typed: Path) -> None:
        """Keep exact revision links immutable; latest PR pointers are separate."""
        architecture_workspace_url(report["id"])
        snapshots = self.output / "reviews"
        if snapshots.is_symlink(): raise OSError("Invalid artifact store")
        snapshots.mkdir(mode=0o700, exist_ok=True)
        snapshot = snapshots / report["id"]
        if snapshot.is_symlink(): raise OSError("Invalid artifact snapshot")
        snapshot.mkdir(mode=0o700, exist_ok=True)
        target = snapshot / "review.json"
        content = typed.read_bytes()
        if len(content) > 16 * 1024 * 1024: raise OSError("Artifact exceeds limit")
        if target.exists() or target.is_symlink():
            if target.is_symlink() or not target.is_file() or target.read_bytes() != content:
                raise OSError("Immutable artifact identity conflict")
            return
        with tempfile.NamedTemporaryFile(dir=snapshot, prefix=".review-", delete=False) as temporary:
            staged = Path(temporary.name)
            try:
                temporary.write(content)
                temporary.flush()
                os.fsync(temporary.fileno())
                os.link(staged, target)  # Fails rather than replacing an existing identity.
            finally:
                staged.unlink(missing_ok=True)

    def active_context(self, *, access=None, slug=None) -> dict:
        # An LLD request may carry its freshly verified origin through this
        # bounded read. Its delivery fence rereads origin and authority; no
        # grant/context is cached, and other callers keep the discovery path.
        if slug is None: slug = self.repository_slug()
        if not isinstance(slug,str) or not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+",slug):
            raise AppError("repository_unavailable")
        owner, name = slug.split("/")
        request={
            "security_domain":"local", "expected_revision":0,
            "repository":{"security_domain":"local", "provider":"github", "owner":owner, "name":name}}
        # HLD already obtained fresh provider authority for this request. Reuse
        # that actor only inside the bounded native read, then recheck actor and
        # context before returning/publishing the HLD. This avoids a second CLI
        # login/permissions round trip without caching an authorization grant.
        if access is None:
            status,context=self.onboarding_request("defaults","active",request)
        else:
            status,context=self.native_onboarding_request("defaults","active",request,slug,access["principal"],access["maintainer"])
        if status != 200:
            raise AppError("confirmed_repository_context_required" if status == 404 else "onboarding_context_unavailable")
        if context.get("version") != "csb-accepted-context-v1" or context.get("usable") is not True:
            raise AppError("confirmed_repository_context_required")
        if context.get("contexts"):
            raise AppError("related_repository_adapter_unavailable")
        return context

    def reports(self) -> list[dict]:
        return [report for path in sorted(self.output.glob("pr-*.json"), reverse=True)
                if (report := self.report(path.stem))]

    def routed_report(self, path, *, lld_get=False):
        if path=="/v1/prs":
            reports=self.reports()
            return reports[0] if reports else None
        revision=re.fullmatch(r"/v1/prs/PR-([1-9][0-9]*)/revisions",path)
        if revision: return self.report(f"pr-{revision.group(1)}")
        review=re.match(r"/v1/reviews/(pr-[1-9][0-9]*-[0-9a-f]{64})(?:/|$)",path)
        if (review and lld_get and self.app_auth is None
            and re.fullmatch(r"/v1/reviews/pr-[1-9][0-9]*-[0-9a-f]{64}/lld/(?:catalog|detail|source|history|export)",path)):
            return self.report(review.group(1),_defer_lld_origin=True)
        return self.report(review.group(1)) if review else None

    def command(self, args: list[str], *, input_text: str | None = None, env_extra: dict | None = None, clean_environment=False, timeout=600) -> subprocess.CompletedProcess:
        environment={key:os.environ[key] for key in ("PATH","TMPDIR","LANG") if key in os.environ} if clean_environment else dict(os.environ)
        return subprocess.run(
            args,
            cwd=self.repository,
            input=input_text,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=timeout,
            check=False,
            env={**environment, "GH_PAGER": "cat", "GH_PROMPT_DISABLED": "0", **(env_extra or {})},
        )

    def authenticated(self) -> bool:
        if self.app_auth: return self.app_auth.public_status()["authenticated"]
        result=self.command(["gh", "api", "user", "--jq", ".login"])
        self.provider_error = "GitHub API rate limit reached. Wait for the reset; reconnecting will not fix this." if "rate limit" in result.stdout.lower() else None
        return result.returncode==0

    def repository_slug(self) -> str | None:
        result = self.command(["git", "remote", "get-url", "origin"])
        match = re.fullmatch(r"(?:https://github\.com/|git@github\.com:)([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+?)(?:\.git)?",result.stdout.strip())
        return match.group(1) if result.returncode==0 and match else None

    def onboarding_request(self, action: str, session_id: str | None, request: dict) -> tuple[int,dict]:
        if not self.authenticated(): return (503, {"error":"github_rate_limited"}) if self.provider_error else (401, {"error":"github_authentication_required"})
        if not self.onboarding_enabled and action not in ("read","defaults","activate","cancel"):
            return 503,{"error":"new_onboarding_disabled"}
        slug=self.repository_slug()
        if not slug: return 404, {"error":"repository_unavailable"}
        # Recheck current account/repo permissions on every operation. No cached authority.
        if self.app_auth:
            try:
                access = self.app_auth.access(slug)
                principal, maintainer = access["principal"], access["maintainer"]
            except AppError as error: return 503 if str(error)=="github_rate_limited" else 404,{"error":str(error)}
        else:
            return self.cli_onboarding_request(action, session_id, request, slug)
        return self.native_onboarding_request(action, session_id, request, slug, principal, maintainer)

    def cli_onboarding_request(self, action, session_id, request, slug):
        account=self.command(["gh","api","user","--jq",".login"])
        access=self.command(["gh","api",f"repos/{slug}","--jq",".permissions"])
        try:
            permissions=json.loads(access.stdout)
            if account.returncode!=0 or access.returncode!=0 or not permissions.get("pull"): return 404,{"error":"repository_unavailable"}
            principal=account.stdout.strip()
            if not re.fullmatch(r"[A-Za-z0-9-]{1,39}",principal): return 401,{"error":"github_authentication_required"}
        except (ValueError,AttributeError): return 503,{"error":"github_access_unavailable"}
        return self.native_onboarding_request(action, session_id, request, slug, f"github:{principal}",bool(permissions.get("push") or permissions.get("maintain") or permissions.get("admin")))

    def native_onboarding_request(self, action, session_id, request, slug, principal, maintainer, *, source_repository=None, publication_fence=None):
        with self.context_lock:
            return self._native_onboarding_request(action, session_id, request, slug, principal, maintainer, source_repository=source_repository, publication_fence=publication_fence)

    def _native_onboarding_request(self, action, session_id, request, slug, principal, maintainer, *, source_repository=None, publication_fence=None):
        binary=native_binary(self.native_profile, "csb-onboarding")
        if not binary.is_file() or binary.is_symlink(): return 503,{"error":"onboarding_adapter_not_built"}
        directory=self.output / ".onboarding"
        payload={"principal":principal,"maintainer":maintainer,
                 "action":action,"id":session_id,"request":request}
        staged_action=action in ("confirm","activate","hld-view","hld-preview","hld-accept","lld-preview","lld-accept")
        requires_maintainer=action in ("confirm","activate","hld-preview","hld-accept","lld-preview","lld-accept")
        # Only the host supplies an isolated, already authorized PR mirror. This
        # path is never accepted from browser JSON and is used for read-only drift.
        if source_repository is not None and action != "drift": return 400,{"error":"invalid_source_override"}

        def private_database(path,*,missing_allowed=False):
            # Both the live state and stage are service-owned. Never follow an
            # attacker-selected SQLite/WAL path or accept unbounded state.
            total=0;files=[]
            for name in (path,Path(str(path)+"-wal"),Path(str(path)+"-shm")):
                try: metadata=name.lstat()
                except FileNotFoundError:
                    if name==path and not missing_allowed: raise OSError("Onboarding database missing")
                    continue
                if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid!=os.getuid() or metadata.st_nlink!=1:
                    raise OSError("Invalid onboarding database")
                total+=metadata.st_size
                files.append((name,metadata))
            if total>64*1024*1024: raise OSError("Onboarding database exceeds limit")
            identity=None
            for name,expected in files:
                descriptor=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_CLOEXEC)
                try:
                    current=os.fstat(descriptor)
                    if (not stat.S_ISREG(current.st_mode) or current.st_uid!=os.getuid() or current.st_nlink!=1
                        or (current.st_dev,current.st_ino)!=(expected.st_dev,expected.st_ino)):
                        raise OSError("Onboarding database changed")
                    os.fchmod(descriptor,0o600)
                    if name==path:identity=(current.st_dev,current.st_ino)
                finally:os.close(descriptor)
            return identity

        def backup_database(source_path,target_path):
            # Both copies use the same bundled SQLite runtime as Storage. The
            # host's Python runtime can be below the supported security version
            # and must never open application databases itself.
            copy_payload={**payload,"action":"database-copy","request":{
                "source":str(source_path),"destination":str(target_path)}}
            result=self.command([str(binary),str(self.repository),str(target_path.parent),slug],input_text=json.dumps(copy_payload),clean_environment=True,timeout=10)
            if result.returncode!=0: raise ValueError("Native database copy failed")
            response=json.loads(result.stdout)
            if response.get("status")!=200 or response.get("value",{}).get("copied") is not True:
                raise ValueError("Invalid database copy response")

        def invoke(state_directory):
            result=self.command([str(binary),str(source_repository or self.repository),str(state_directory),slug],input_text=json.dumps(payload),clean_environment=True)
            if result.returncode!=0: raise ValueError("Native onboarding failed")
            response=json.loads(result.stdout)
            status=response["status"]
            if type(status) is not int or not 100<=status<=599 or not isinstance(response["value"],dict):
                raise ValueError("Invalid onboarding response")
            return status,response["value"]

        def current_authority():
            if self.app_auth:
                current=self.app_auth.access(slug)
                if current["principal"]!=principal or (requires_maintainer and not current["maintainer"]):
                    raise AppError("repository_unavailable")
            elif staged_action:
                account=self.command(["gh","api","user","--jq",".login"])
                access=self.command(["gh","api",f"repos/{slug}","--jq",".permissions"])
                permissions=json.loads(access.stdout)
                login=account.stdout.strip()
                if (account.returncode!=0 or access.returncode!=0 or not re.fullmatch(r"[A-Za-z0-9-]{1,39}",login)
                    or principal!=f"github:{login}" or not isinstance(permissions,dict) or permissions.get("pull") is not True
                    or (requires_maintainer and not any(permissions.get(role) is True for role in ("push","maintain","admin")))):
                    raise AppError("repository_unavailable")
            if publication_fence is not None: publication_fence()

        try:
            if directory.is_symlink(): raise OSError("Invalid onboarding directory")
            directory.mkdir(mode=0o700,exist_ok=True)
            if directory.stat().st_uid!=os.getuid(): raise OSError("Invalid onboarding owner")
            os.chmod(directory,0o700)
            live=directory / "onboarding.sqlite"
            if not staged_action:
                private_database(live,missing_allowed=True)
                if action=="preferences-write" and self.app_auth:
                    current_authority()
                    with self.app_auth.lock:
                        if (not self.app_auth._token or self.app_auth.clock()>=self.app_auth._token_deadline or (self.app_auth._identity or {}).get("principal")!=principal):
                            raise AppError("github_authentication_required")
                        status,value=invoke(directory)
                else: status,value=invoke(directory)
                current_authority()
                return status,value
            if requires_maintainer and not maintainer: return 404,{"error":"repository_unavailable"}
            if self.app_auth:
                with self.app_auth.lock:
                    generation,token=self.app_auth.generation,self.app_auth._token
                    if not token or self.app_auth.clock()>=self.app_auth._token_deadline:
                        raise AppError("github_authentication_required")
            identity=private_database(live)
            # Confirmation/default activation must remain invisible until the
            # provider fence passes. SQLite backup includes committed WAL data;
            # the native process exits and closes staged connections first.
            with tempfile.TemporaryDirectory(prefix=".authority-",dir=directory) as temporary:
                stage=Path(temporary)
                staged=stage / "onboarding.sqlite"
                staged.touch(mode=0o600)
                backup_database(live,staged)
                status,value=invoke(stage)
                current_authority()
                if status!=200: return status,value
                private_database(staged)
                if private_database(live)!=identity: raise OSError("Onboarding database changed")
                if self.app_auth:
                    # Serialize logout/reconnect with the final bounded backup.
                    # The destination transaction commits all accepted defaults
                    # together; interruption rolls it back without touching live.
                    with self.app_auth.lock:
                        if self.app_auth.generation!=generation or self.app_auth._token!=token or self.app_auth.clock()>=self.app_auth._token_deadline:
                            raise AppError("github_authentication_required")
                        backup_database(staged,live)
                else: backup_database(staged,live)
                return status,value
        except AppError: return 404,{"error":"repository_unavailable"}
        except (OSError,subprocess.TimeoutExpired,ValueError,KeyError,TypeError,AttributeError):
            return 503,{"error":"onboarding_adapter_unavailable"}

    def publish_report(self, pr, typed, candidate, validated, authentication_generation):
        # Serialize default activation with the final fence and publication.
        with self.context_lock:
            os.chmod(typed,0o600)
            os.chmod(candidate,0o600)
            provenance=validated.get("artifact",{}).get("onboarding_context")
            if self.app_auth:
                if not isinstance(provenance,dict): raise AppError("confirmed_repository_context_required")
                current=self.active_context()
                if any(provenance.get(key)!=current.get(key) for key in
                       ("session_id","session_revision","input_fingerprint","proposal_fingerprint","accepted_scope_fingerprint")):
                    raise AppError("superseded_context")
                scope=validated["artifact"].get("scope_manifest",{})
                if any(scope.get(key)!=current.get(key) for key in ("include","exclude","contexts","export_allowed")):
                    raise AppError("superseded_context")
                slug=self.repository_slug()
                access=self.app_auth.access(slug)
                self.app_auth._verify_pull_revision(slug,pr,validated["artifact"]["selected_baseline"],validated["artifact"]["proposal"],access)
            if self.app_auth:
                with self.app_auth.lock:
                    if self.app_auth.generation!=authentication_generation or not self.app_auth._token or self.app_auth.clock()>=self.app_auth._token_deadline:
                        raise AppError("github_authentication_required")
                    self.preserve_report(validated, typed)
                    os.replace(typed,self.output/f"pr-{pr}.json")
                    os.replace(candidate,self.output/f"pr-{pr}.html")
            else:
                self.preserve_report(validated, typed)
                os.replace(typed,self.output/f"pr-{pr}.json")
                os.replace(candidate,self.output/f"pr-{pr}.html")

    def prs(self) -> list[dict]:
        if self.app_auth:
            slug=self.repository_slug()
            self.app_auth.access(slug)
            value=self.app_auth.api(f"/repos/{slug}/pulls?state=all&per_page=50",object_only=False)
            if not isinstance(value,list): raise AppError("invalid_pr_response")
            return [{"number":item["number"],"title":item["title"],"state":"MERGED" if item.get("merged_at") else item["state"].upper(),"isDraft":bool(item.get("draft")),"updatedAt":item["updated_at"],"headRefName":item["head"]["ref"],"baseRefName":item["base"]["ref"]} for item in value]
        result = self.command([
            "gh", "pr", "list", "--state", "all", "--limit", "50",
            "--json", "number,title,state,isDraft,updatedAt,headRefName,baseRefName,url",
        ])
        if result.returncode != 0:
            return []
        value = json.loads(result.stdout)
        return value if isinstance(value, list) else []

    def provider_status(self) -> dict:
        # Polling review progress must not issue GitHub requests every 1.5 seconds.
        # This UI observation is never used as a mutation/access grant.
        with self.provider_status_lock:
            if time.monotonic()<self.provider_status_until: return dict(self.provider_status_cache)
            authenticated=self.authenticated()
            value={"authenticated":authenticated,"provider_error":self.provider_error,"observed_at_ms":int(time.time()*1000)}
            if self.app_auth: value.update(self.app_auth.public_status())
            if authenticated:
                try: value.update({"repository":self.repository_slug(),"prs":self.prs()})
                except (AppError,KeyError,TypeError,AttributeError) as error: value["provider_error"]="GitHub API rate limit reached. Wait for the reset." if str(error)=="github_rate_limited" else "Repository access is unavailable. Check the App installation and your permissions."
            self.provider_status_cache=value; self.provider_status_until=time.monotonic()+(60 if self.provider_error else 30)
            return dict(value)

    def start_auth(self) -> None:
        if self.app_auth:
            with self.lock:
                if self.auth_running or self.app_auth.public_status()["authenticated"]: return
                self.auth_running=True
                self.provider_status_until=0; self.provider_status_cache={}
            def connect_app():
                try:
                    self.app_auth.begin()
                    while self.app_auth.poll_once(): time.sleep(1)
                except AppError as error:
                    self.app_auth.logout(); self.app_auth.error=str(error)
                finally:
                    with self.lock: self.auth_running=False
                    self.provider_status_until=0
            threading.Thread(target=connect_app,daemon=True).start()
            return
        with self.lock:
            if self.auth_running or self.authenticated():
                return
            self.auth_running, self.auth_code, self.auth_error = True, None, None

        def run() -> None:
            master = None
            try:
                master, slave = pty.openpty()
                process = subprocess.Popen([
                    "gh", "auth", "login", "--hostname", "github.com", "--web",
                    "--git-protocol", "https",
                ], cwd=self.repository, stdin=slave, stdout=slave, stderr=slave, close_fds=True,
                    env={**os.environ, "GH_PAGER": "cat"})
                os.close(slave)
                output, answered, continued = "", False, False
                deadline = time.monotonic() + 600
                while process.poll() is None and time.monotonic() < deadline:
                    readable, _, _ = select.select([master], [], [], 0.25)
                    if readable:
                        try: data = os.read(master, 1024).decode(errors="replace")
                        except OSError: break
                        output = (output + data)[-8192:]
                        if "Authenticate Git with your GitHub credentials?" in output and not answered:
                            os.write(master, b"y\n"); answered = True
                        code = re.search(r"\b[A-Z0-9]{4}-[A-Z0-9]{4}\b", output)
                        if code:
                            with self.lock: self.auth_code = code.group(0)
                            if not continued: os.write(master, b"\n"); continued = True
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=5)
                with self.lock:
                    if process.returncode != 0 or not self.authenticated():
                        self.auth_error = "GitHub authorization did not complete. Start it again."
            except (OSError, subprocess.TimeoutExpired):
                with self.lock:
                    self.auth_error = "GitHub CLI authentication could not be started."
            finally:
                if master is not None:
                    try: os.close(master)
                    except OSError: pass
                with self.lock:
                    self.auth_running = False
                    self.provider_status_until = 0

        threading.Thread(target=run, daemon=True).start()

    def require_app_access(self, report=None):
        if not self.app_auth: return
        access=self.app_auth.access(self.repository_slug())
        if report:
            if not isinstance(report["artifact"].get("onboarding_context"),dict):
                raise AppError("confirmed_repository_context_required")
            current=self.active_context()
            def allowed(path):
                def within(prefix):return prefix=="." or path==prefix or path.startswith(prefix+"/")
                return isinstance(path,str) and any(within(prefix) for prefix in current["include"]) and not any(within(prefix) for prefix in current["exclude"])
            retained=[item.get("path") for item in report["files"]]+[item.get("path") for item in report["architecture"].get("nodes",[])]
            if not all(allowed(path) for path in retained): raise AppError("context_unavailable")
            contexts=report["artifact"]["scope_manifest"].get("contexts",[])
            if len(contexts)>12: raise AppError("context_limit_exceeded")
            for context in contexts:
                repository=context.get("repository") if isinstance(context,dict) else None
                if not isinstance(repository,dict) or repository.get("provider")!="github": raise AppError("context_unavailable")
                self.app_auth.access(f"{repository['owner']}/{repository['name']}")
        return access

    def fresh_hld_access(self, *, slug=None, parallel_cli=False):
        if slug is None: slug=self.repository_slug()
        if not isinstance(slug,str) or not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+",slug):
            raise AppError("repository_unavailable")
        if self.app_auth: return self.app_auth.access(slug)
        if parallel_cli:
            # These independent provider reads both have to succeed. LLD's
            # context_lock bounds this to two workers; its final fence repeats
            # the reads after native work. There is no cached permission.
            try:
                with ThreadPoolExecutor(max_workers=2,thread_name_prefix="csb-current-access") as workers:
                    login=workers.submit(self.command,["gh","api","user","--jq",".login"],timeout=10)
                    permissions=workers.submit(self.command,["gh","api",f"repos/{slug}","--jq",".permissions"],timeout=10)
                    account,response=login.result(),permissions.result()
            except (OSError,subprocess.TimeoutExpired):
                raise AppError("repository_unavailable") from None
        else:
            account=self.command(["gh","api","user","--jq",".login"])
            response=self.command(["gh","api",f"repos/{slug}","--jq",".permissions"])
        try: permissions=json.loads(response.stdout)
        except (ValueError,TypeError): raise AppError("repository_unavailable") from None
        login=account.stdout.strip()
        if (account.returncode!=0 or response.returncode!=0 or not re.fullmatch(r"[A-Za-z0-9-]{1,39}",login)
            or not isinstance(permissions,dict) or permissions.get("pull") is not True): raise AppError("repository_unavailable")
        return {"principal":f"github:{login}","maintainer":any(permissions.get(role) is True for role in ("push","maintain","admin"))}

    def hld_request(self, report, action="hld-view", request=None):
        """Current authority intersects an immutable review and typed native HLD."""
        with self.context_lock:
            try:
                access=self.fresh_hld_access()
                self.require_app_access(report)
                if not isinstance(report["artifact"].get("hld"),dict): return 404,{"error":"hld_not_generated_regenerate_review"}
                context=self.active_context(access=access)
                original=report["artifact"].get("onboarding_context") or {}
                context_current=all(original.get(key)==context.get(key) for key in
                    ("session_id","session_revision","input_fingerprint","proposal_fingerprint","accepted_scope_fingerprint"))
                mutable=action in ("hld-preview","hld-accept")
                if mutable and not self.hld_enabled: return 503,{"error":"new_hld_generation_disabled"}
                if mutable and (not access["maintainer"] or not context_current): return 404,{"error":"hld_confirmation_unavailable"}
                actor=access["principal"]
                def fence():
                    current=self.fresh_hld_access()
                    if current["principal"]!=actor or (mutable and not current["maintainer"]): raise AppError("repository_unavailable")
                    self.require_app_access(report)
                    if self.active_context(access=current)!=context: raise AppError("superseded_context")
                    if mutable:
                        if self.app_auth:
                            self.app_auth._verify_pull_revision(self.repository_slug(),report["number"],report["artifact"]["selected_baseline"],report["artifact"]["proposal"],current)
                        else:
                            result=self.command(["gh","api",f"repos/{self.repository_slug()}/pulls/{report['number']}"])
                            metadata=json.loads(result.stdout)
                            if result.returncode!=0 or metadata["base"]["sha"]!=report["artifact"]["selected_baseline"] or metadata["head"]["sha"]!=report["artifact"]["proposal"]: raise AppError("superseded_revision")
                payload=dict(request or {})
                payload["artifact_path"]=str(report["artifact_path"])
                payload["comparison_id"]=report["id"]
                prefix="hld-review-"+hashlib.sha256(report["id"].encode()).hexdigest()[:32]+"-"
                if action=="hld-preview" and not payload.get("preview_id"):
                    payload["preview_id"]=prefix+secrets.token_hex(16)
                    payload["create_preview"]=True
                if mutable and (not isinstance(payload.get("preview_id"),str) or not re.fullmatch(re.escape(prefix)+r"[0-9a-f]{32}",payload["preview_id"])):
                    return 404,{"error":"hld_preview_comparison_mismatch"}
                # Authorized immutable candidates need no staged database copy
                # on source/diagram reads. A missing candidate is registered
                # once behind the same publication fence; permissions are
                # still rechecked for every read, never cached with the graph.
                operation="hld-read" if action=="hld-view" else action
                status,value=self.native_onboarding_request(operation,"hld",payload,self.repository_slug(),actor,access["maintainer"],publication_fence=fence)
                if action=="hld-view" and status==404:
                    status,value=self.native_onboarding_request(action,"hld",payload,self.repository_slug(),actor,access["maintainer"],publication_fence=fence)
                if status!=200: return status,value
                sources_complete=True
                if action=="hld-view":
                    try: sources_complete=self.preserve_hld_sources(report)
                    except OSError: sources_complete=False
                # A committed confirmation cannot be reported rejected because
                # optional source retention subsequently failed.
                return 200,{"api_version":"v1","review_id":report["id"],**value,
                            "can_accept":bool(access["maintainer"] and context_current and self.hld_enabled),
                            "can_export":report["artifact"].get("scope_manifest",{}).get("export_allowed") is True and context.get("export_allowed") is True,
                            "generation_enabled":self.hld_enabled,"source_gaps":report["artifact"]["hld"].get("source_gaps",[])+([] if sources_complete else ["Private evidence retention is unavailable or its retention budget was reached; some exact source spans are unavailable"])}
            except AppError: return 404,{"error":"hld_unavailable_in_current_context"}
            except (ValueError,KeyError,TypeError,OSError): return 503,{"error":"hld_adapter_unavailable"}

    def _lld_delivery_access(self, slug):
        # Independent current observations overlap, but both must finish before
        # context checks or delivery. They never survive this request as grants.
        try:
            with ThreadPoolExecutor(max_workers=2,thread_name_prefix="csb-lld-delivery") as workers:
                origin=workers.submit(self.repository_slug)
                access=workers.submit(self.fresh_hld_access,slug=slug,parallel_cli=True)
                current_origin,current_access=origin.result(),access.result()
        except (OSError,subprocess.TimeoutExpired):
            raise AppError("repository_unavailable") from None
        if current_origin!=slug: raise AppError("repository_unavailable")
        return current_access

    def lld_request(self, report, action="lld-catalog", request=None):
        """Fresh authority surrounds native work on private pinned syntax inputs."""
        with self.context_lock:
            try:
                slug=self.repository_slug()
                identity=report["artifact"].get("scope_manifest",{}).get("primary_repository",{})
                if (not isinstance(identity,dict) or identity.get("provider")!="github"
                    or f"{identity.get('owner')}/{identity.get('name')}"!=slug):
                    raise AppError("repository_unavailable")
                access=self.fresh_hld_access(slug=slug,parallel_cli=True)
                self.require_app_access(report)
                if not isinstance(report["artifact"].get("lld"),dict): return 404,{"error":"lld_not_generated_regenerate_review"}
                context=self.active_context(access=access,slug=slug)
                original=report["artifact"].get("onboarding_context") or {}
                context_current=all(original.get(key)==context.get(key) for key in
                    ("session_id","session_revision","input_fingerprint","proposal_fingerprint","accepted_scope_fingerprint"))
                mutable=action in ("lld-preview","lld-accept")
                if (mutable or action in ("lld-detail","lld-source","lld-export")) and not self.lld_enabled: return 503,{"error":"new_lld_generation_disabled"}
                if mutable and (not access["maintainer"] or not context_current): return 404,{"error":"lld_confirmation_unavailable"}
                actor=access["principal"]
                def fence():
                    # Origin is an identity pin, not an authorization cache.
                    # A changed checkout target must not redirect this request
                    # to another repository, even for the same current actor.
                    current=self._lld_delivery_access(slug)
                    if current["principal"]!=actor or (mutable and not current["maintainer"]): raise AppError("repository_unavailable")
                    self.require_app_access(report)
                    if self.active_context(access=current,slug=slug)!=context: raise AppError("superseded_context")
                    if mutable:
                        if self.app_auth:
                            self.app_auth._verify_pull_revision(slug,report["number"],report["artifact"]["selected_baseline"],report["artifact"]["proposal"],current)
                        else:
                            result=self.command(["gh","api",f"repos/{slug}/pulls/{report['number']}"])
                            metadata=json.loads(result.stdout)
                            if result.returncode!=0 or metadata["base"]["sha"]!=report["artifact"]["selected_baseline"] or metadata["head"]["sha"]!=report["artifact"]["proposal"]: raise AppError("superseded_revision")
                payload=dict(request or {})
                payload["artifact_path"]=str(report["artifact_path"])
                payload["comparison_id"]=report["id"]
                parent=payload.get("parent_hld")
                if parent not in [report["artifact"].get("hld",{}).get(side,{}).get("id") for side in ("baseline","proposal")]:
                    return 404,{"error":"lld_parent_version_unavailable"}
                prefix="lld-review-"+hashlib.sha256(report["id"].encode()).hexdigest()[:32]+"-"
                if action=="lld-preview" and not payload.get("preview_id"):
                    payload["preview_id"]=prefix+secrets.token_hex(16)
                    payload["create_preview"]=True
                if mutable and (not isinstance(payload.get("preview_id"),str) or not re.fullmatch(re.escape(prefix)+r"[0-9a-f]{32}",payload["preview_id"])):
                    return 404,{"error":"lld_preview_comparison_mismatch"}
                if report["id"] not in self._lld_parent_registered:
                    status,_=self.hld_request(report)
                    if status!=200: return status,{"error":"lld_hld_parent_unavailable"}
                    self._lld_parent_registered.add(report["id"])
                status,value=self.native_onboarding_request(action,"lld",payload,slug,actor,access["maintainer"],publication_fence=fence)
                if status!=200: return status,value
                # Native never exposes the full syntax bundle. Only bounded
                # catalog pages, selected graphs or authorized spans cross HTTP.
                identity=value.pop("identity",{})
                return 200,{"api_version":"v1","review_id":report["id"],**value,**identity,
                    "can_accept":bool(access["maintainer"] and context_current and self.lld_enabled),
                    "can_export":report["artifact"].get("scope_manifest",{}).get("export_allowed") is True and context.get("export_allowed") is True,
                    "generation_enabled":self.lld_enabled}
            except AppError: return 404,{"error":"lld_unavailable_in_current_context"}
            except (ValueError,KeyError,TypeError,OSError): return 503,{"error":"lld_adapter_unavailable"}

    def preserve_hld_sources(self, report):
        # Exact evidence spans stay private and are retrieved only after native
        # design authorization. Missing spans never fall back to the worktree.
        directory=self.output/".design-evidence"
        if directory.is_symlink(): raise OSError("Invalid evidence store")
        directory.mkdir(mode=0o700,exist_ok=True); os.chmod(directory,0o700)
        entries=list(directory.iterdir())
        if len(entries)>20000: return False
        retained=sum(item.lstat().st_size for item in entries if item.is_file() and not item.is_symlink())
        complete=True
        for source in report["artifact"]["hld"].get("sources",[]):
            identity=source.get("evidence_id","")
            if not re.fullmatch(r"evidence-[0-9a-f]{64}",identity): raise OSError("Invalid evidence identity")
            encoded=json.dumps(source,separators=(",",":"),sort_keys=True).encode()
            body=json.dumps({"version":"csb-hld-source-v1","sha256":hashlib.sha256(encoded).hexdigest(),"source":source},separators=(",",":"),sort_keys=True).encode()
            if len(body)>300000: raise OSError("Evidence span exceeds limit")
            target=directory/(identity+".json")
            if target.exists() or target.is_symlink():
                if self.read_private_hld_source(target)!=body: raise OSError("Evidence identity conflict")
                continue
            if retained+len(body)>64*1024*1024 or len(entries)>=20000: complete=False; continue
            with tempfile.NamedTemporaryFile(dir=directory,prefix=".evidence-",delete=False) as temporary:
                staged=Path(temporary.name); os.chmod(staged,0o600); temporary.write(body); temporary.flush(); os.fsync(temporary.fileno())
            try: os.link(staged,target)
            finally: staged.unlink(missing_ok=True)
            retained+=len(body); entries.append(target)
        return complete

    @staticmethod
    def read_private_hld_source(target):
        parent=target.parent.lstat()
        if not stat.S_ISDIR(parent.st_mode) or parent.st_uid!=os.getuid() or parent.st_mode&0o077:
            raise OSError("Invalid evidence store")
        descriptor=os.open(target,os.O_RDONLY|os.O_NOFOLLOW|os.O_CLOEXEC)
        try:
            metadata=os.fstat(descriptor)
            if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid!=os.getuid() or metadata.st_nlink!=1
                or metadata.st_mode&0o077 or metadata.st_size>300000): raise OSError("Invalid private evidence")
            with os.fdopen(descriptor,"rb",closefd=False) as handle:
                body=handle.read(300001)
            if len(body)>300000: raise OSError("Evidence span exceeds limit")
            return body
        finally: os.close(descriptor)

    def hld_source(self, report, payload, design_id, evidence_id):
        versions=[payload.get(key) for key in ("baseline","proposal","accepted","accepted_intended")]
        design=next((item for item in versions if item and item.get("id")==design_id),None)
        if not design: return None
        evidence=next((item for item in design.get("evidence",[]) if item.get("id")==evidence_id),None)
        if not evidence or not re.fullmatch(r"evidence-[0-9a-f]{64}",evidence_id): return None
        target=self.output/".design-evidence"/(evidence_id+".json")
        try:
            retained=json.loads(self.read_private_hld_source(target))
            source=retained.get("source")
            if not isinstance(source,dict) or retained.get("version")!="csb-hld-source-v1" or retained.get("sha256")!=hashlib.sha256(json.dumps(source,separators=(",",":"),sort_keys=True).encode()).hexdigest(): return None
            current=next((item for item in report["artifact"]["hld"].get("sources",[]) if item.get("evidence_id")==evidence_id),None)
            if current is not None and current!=source: return None
            if (source.get("evidence_id")!=evidence_id or source.get("commit")!=evidence.get("commit")
                or source.get("path")!=evidence.get("path") or source.get("blob_id")!=evidence.get("blob")
                or source.get("start_line")!=evidence.get("start_line") or type(source.get("end_line")) is not int
                or source["end_line"]<source["start_line"] or source["end_line"]>evidence.get("end_line",0)
                or not isinstance(source.get("text"),str) or len(source["text"].encode())>65536): return None
            return {"api_version":"v1","review_id":report["id"],**source}
        except (ValueError,TypeError,OSError,KeyError): return None

    def app_review(self, pr, pending):
        slug=self.repository_slug()
        actor=self.app_auth.access(slug)["principal"]
        context=self.active_context()
        metadata=self.app_auth.api(f"/repos/{slug}/pulls/{pr}")
        baseline, proposal=metadata["base"]["sha"],metadata["head"]["sha"]
        mirror=pending/"mirror.git"
        self.app_auth.fetch_commits(mirror,slug,pr,baseline,proposal)
        access=self.app_auth.access(slug)
        if access["principal"]!=actor: raise AppError("github_authentication_required")
        drift={"version":"csb-onboarding-drift-v1","session_id":context["session_id"],
               "session_revision":context["session_revision"],"source_commit":context["commit"],
               "target_commit":proposal,"input_fingerprint":context["input_fingerprint"],
               "proposal_fingerprint":context["proposal_fingerprint"],
               "accepted_scope_fingerprint":context["accepted_scope_fingerprint"],
               "status":"incomplete","reuse_allowed":False,"reconfirmation_required":True,
               "affected_components":[],"gaps":["New onboarding/drift checks are disabled; accepted scope remains unchanged."],
               "defaults_changed":False,"scope_expanded":False}
        if self.onboarding_enabled:
            owner,name=slug.split("/")
            status,drift=self.native_onboarding_request("drift",context["session_id"],
                {"security_domain":"local","expected_revision":context["session_revision"],"reference":proposal,
                 "repository":{"security_domain":"local","provider":"github","owner":owner,"name":name}},
                slug,actor,access["maintainer"],source_repository=mirror)
            if status != 200: raise AppError("onboarding_context_unavailable")
        context_path=pending/"accepted-context.json"
        with context_path.open("x", encoding="utf-8") as stream:
            os.chmod(context_path,0o600)
            json.dump({**context,"drift":drift},stream)
        binary=native_binary(self.native_profile, "flowdiff")
        if not binary.is_file() or binary.is_symlink(): raise ValueError("Selected trusted flowdiff binary is unavailable")
        env={key:os.environ[key] for key in ("PATH","TMPDIR","LANG") if key in os.environ}
        env.update({"GIT_CONFIG_NOSYSTEM":"1","GIT_CONFIG_GLOBAL":os.devnull,"GIT_TERMINAL_PROMPT":"0"})
        result=subprocess.run([str(binary),"review",*(["--disable-hld"] if not self.hld_enabled else []),*(["--disable-lld"] if not self.lld_enabled else []),"--repo",str(mirror),"--github-repository",slug,"--onboarding-context",str(context_path),"--internal-artifact","--base",baseline,"--head",proposal,"--merge-base","--cache-dir",str(self.output/".cache"),"--output",str(pending/f"pr-{pr}.html"),"--artifact",str(pending/f"pr-{pr}.json")],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=120,check=False)
        if self.app_auth.access(slug)["principal"]!=actor: raise AppError("github_authentication_required")
        latest=self.app_auth.api(f"/repos/{slug}/pulls/{pr}")
        if latest["base"]["sha"]!=baseline or latest["head"]["sha"]!=proposal: raise AppError("superseded_revision")
        if self.active_context() != context: raise AppError("superseded_context")
        return result

    def start_review(self, pr: int) -> int | None:
        with self.lock:
            if self.review_running:
                return None
            self.review_generation += 1
            generation = self.review_generation
            authentication_generation = self.app_auth.generation if self.app_auth else None
            self.review_pr = pr
            self.review_running, self.review_error, self.review_url = True, None, None

        def run() -> None:
            report = self.output / f"pr-{pr}.html"
            published_current = False
            try:
                with tempfile.TemporaryDirectory(prefix=f".pending-pr-{pr}-", dir=self.output) as pending:
                    pending_path = Path(pending)
                    result = self.app_review(pr,pending_path) if self.app_auth else self.command([
                        str(ROOT / "scripts" / "github_mvp.sh"), str(self.repository), str(pr),
                        str(pending_path), "--no-serve",
                    ], env_extra={"CSB_CACHE_DIR": str(self.output / ".cache"),
                                  "CSB_FLOWDIFF_BIN": str(native_binary(self.native_profile, "flowdiff"))})
                    candidate = pending_path / f"pr-{pr}.html"
                    if result.returncode == 0 and candidate.is_file() and not candidate.is_symlink():
                        typed = pending_path / f"pr-{pr}.json"
                        if not typed.is_file() or typed.is_symlink(): raise OSError("Typed artifact missing")
                        validated=self.report(f"pr-{pr}", pending_path)
                        if not validated: raise OSError("Invalid typed artifact")
                        with self.lock:
                            if self.review_generation != generation or self.review_pr != pr: raise OSError("Superseded review")
                            if self.app_auth: self.app_auth.access(self.repository_slug())
                            self.publish_report(pr,typed,candidate,validated,authentication_generation)
                            published_current = True
                with self.lock:
                    if result.returncode == 0 and published_current and report.is_file():
                        published=self.report(f"pr-{pr}")
                        if published: self.review_url=architecture_workspace_url(published['id'],bool(published['artifact'].get('hld')))
                        else: self.review_error="The generated artifact could not be validated."
                    else:
                        self.review_error = "Review generation failed. Verify repository access and the selected PR."
            except AppError as error:
                with self.lock:
                    self.review_error = {"confirmed_repository_context_required":"Open Confirm architecture, resolve the scanned context, and select Use this confirmed context for PR reviews.",
                        "superseded_context":"The confirmed repository context changed. Start the review again.",
                        "superseded_revision":"The PR commits changed. Start the review again.",
                        "related_repository_adapter_unavailable":"Related repository analysis is unavailable in this primary-repository adapter."}.get(str(error),"GitHub access or repository context is unavailable. Reconnect and verify the App installation.")
            except (OSError, subprocess.TimeoutExpired,KeyError,TypeError,AttributeError,ValueError):
                with self.lock:
                    self.review_error = "Review generation could not complete within its local limit."
            finally:
                with self.lock:
                    self.review_running = False

        threading.Thread(target=run, daemon=True).start()
        return generation


def handler(state: State):
    class Handler(BaseHTTPRequestHandler):
        server_version = "CodeShouldBehaveMVP/1"

        def loopback_origin_allowed(self) -> bool:
            hosts = {f"127.0.0.1:{self.server.server_port}",f"localhost:{self.server.server_port}"}
            origin = self.headers.get("Origin")
            return self.headers.get("Host") in hosts and (not origin or origin in {f"http://{host}" for host in hosts})

        def send_body_headers(self, content_type: str, length: int) -> None:
            self.send_response(HTTPStatus.OK)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(length))
            self.send_header("Cache-Control", "no-store")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("Referrer-Policy", "no-referrer")
            self.send_header("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; form-action 'none'")
            self.end_headers()

        def json_status(self, status: int, value: dict) -> None:
            body=json.dumps(value).encode()
            self.send_response(status); self.send_header("Content-Type","application/json"); self.send_header("Content-Length",str(len(body))); self.send_header("Cache-Control","private, no-store"); self.end_headers(); self.wfile.write(body)

        def json(self, value: dict) -> None:
            body = json.dumps(value, separators=(",", ":")).encode()
            self.send_body_headers("application/json", len(body)); self.wfile.write(body)

        def authorized(self) -> bool:
            return secrets.compare_digest(self.headers.get("X-CSB-Session", ""), state.session)

        def api_authorized(self) -> bool:
            value = self.headers.get("Authorization", "")
            return value.startswith("Bearer ") and secrets.compare_digest(value[7:], state.session)

        def workspace_json(self, report: dict, path: str, query: dict) -> dict | None:
            review_id = report["id"]; observation = report["observation"]; architecture = report["architecture"]
            slug = state.repository_slug() or observation.get("repository", "local/repository")
            owner, name = slug.split("/", 1) if "/" in slug else ("local", slug)
            cached = state.provider_status_cache
            provider_pr = next((item for item in cached.get("prs",[]) if item.get("number")==report["number"]), None)
            provider_state = "unknown" if not provider_pr else ("draft" if provider_pr.get("isDraft") else provider_pr.get("state","unknown").lower())
            revision = {"repository_id": 1, "repository": {"security_domain":"local","provider":"github","owner":owner,"name":name},
                        "provider_pr_id": f"PR-{report['number']}", "generation": 1, "review_id": review_id,
                        "base_commit": observation["baseline"], "head_commit": observation["proposal"], "target_branch":"main",
                        "state":provider_state, "priority":50, "observed_at_ms":cached.get("observed_at_ms",0) if provider_pr else 0}
            suffix = path.removeprefix(f"/v1/reviews/{review_id}")
            if path == "/v1/prs":
                items=[]
                for item in state.reports():
                    try: state.require_app_access(item)
                    except AppError: continue
                    current = self.workspace_json(item, f"/v1/reviews/{item['id']}", {})
                    items.append(current["revision"])
                return {"api_version":"v1","items":items,"next_cursor":None}
            if path.startswith("/v1/prs/") and path.endswith("/revisions"):
                return {"api_version":"v1","items":[revision],"next_cursor":None}
            if suffix == "":
                return {"api_version":"v1","review_id":review_id,"selected_baseline":report["artifact"]["selected_baseline"],
                        "analysis_baseline":observation["baseline"],"proposal":observation["proposal"],"comparison_mode":"merge_base" if report["artifact"]["selected_baseline"]!=observation["baseline"] else "exact",
                        "scope_version":report["artifact"]["scope_version"],"stale":False,"capabilities":["graph","source"]+(["export"] if report["artifact"].get("scope_manifest",{}).get("export_allowed",True) else []),"revision":revision,
                        "context_drift":(report["artifact"].get("onboarding_context") or {}).get("drift")}
            if suffix == "/progress":
                return {"api_version":"v1","review_id":review_id,"job_state":"completed","attempt_count":1,
                        "terminal_reason":None,"coverage":[[key,value] for key,value in report["artifact"]["coverage"].items() if key!="enumeration_complete"],"stale":False}
            if suffix == "/files":
                return {"api_version":"v1","review_id":review_id,"items":report["files"],"next_cursor":None}
            if suffix == "/findings":
                findings=[]; finding_id=1
                for item in report["files"]:
                    findings.append({"id":finding_id,"kind":f"structural_path_{item['kind']}","resolution":"resolved",
                                     "confidence":1.0,"source_path":item["path"],"source_start_line":1,"source_end_line":1})
                    finding_id += 1
                for index, edge in enumerate(architecture.get("edges", [])):
                    if edge.get("status") == "unchanged": continue
                    if edge.get("from") not in observation.get("files",[]) and edge.get("to") not in observation.get("files",[]): continue
                    findings.append({"id":finding_id,"kind":f"call_edge_{edge.get('status','changed')}","resolution":"ambiguous",
                                     "confidence":0.5,"source_path":edge.get("from"),"source_start_line":1,"source_end_line":1})
                    finding_id += 1
                    if len(findings) >= 24: break
                return {"api_version":"v1","review_id":review_id,"items":findings,"next_cursor":None}
            if suffix == "/annotations":
                principal=getattr(self,"current_access",None)
                principal=principal["principal"] if principal else "local-reviewer"
                items=[item for item in state.annotations.get(review_id,[]) if item.get("visibility")!="private" or item.get("owner_principal")==principal]
                return {"api_version":"v1","review_id":review_id,"items":items,"delta":None}
            if suffix == "/graph":
                side=(query.get("side") or ["proposal"])[0]; selected=(query.get("path") or [None])[0]; symbol=(query.get("symbol") or [None])[0]
                nodes=architecture.get("nodes",[])
                if symbol:
                    edges=[]
                    for index, edge in enumerate(architecture.get("edges",[])):
                        if edge.get("name") != symbol: continue
                        edges.append({"id":index+1,"stable_key":f"{index}:{edge.get('from')}:{edge.get('to')}","from_symbol":edge.get("name"),
                                      "to_symbol":edge.get("name"),"kind":"call","confidence":0.5,"resolution":"ambiguous","source_path":edge.get("from")})
                    return {"api_version":"v1","kind":"neighborhood","edges":edges,"next_cursor":None,"omitted":0,"coverage":[]}
                if selected:
                    node = next((item for item in nodes if item.get("path") == selected), None)
                    if not node: return {"api_version":"v1","kind":"symbols","symbols":[],"next_cursor":None}
                    names=node.get("definitionsBefore" if side=="baseline" else "definitionsAfter",[])
                    source_text=node.get("sourceBefore" if side=="baseline" else "sourceAfter") or ""
                    symbols=[{"stable_key":f"{side}:{selected}:{name}","correspondence_key":f"{selected}:{name}","path":selected,"kind":"symbol","name":name,
                              "start_line":1,"start_column":1,"end_line":max(1,min(500,len(source_text.splitlines()))),"end_column":1,"blob_id":node.get("blobBefore" if side=="baseline" else "blobAfter")} for name in names[:100]]
                    return {"api_version":"v1","kind":"symbols","symbols":symbols,"next_cursor":None}
                paths=[]
                for node in nodes:
                    status=node.get("status","unchanged")
                    if (side=="baseline" and status=="added") or (side=="proposal" and status=="removed"): continue
                    if status!="unchanged" or node.get("path") in observation.get("files",[]):
                        paths.append({"path":node.get("path"),"kind":"file","revision":side,"content_status":status})
                return {"api_version":"v1","kind":"children","paths":paths[:100],"next_cursor":None,"omitted":max(0,len(paths)-100)}
            if suffix == "/source":
                selected=(query.get("path") or [""])[0]; side=(query.get("side") or ["proposal"])[0]
                node=next((item for item in architecture.get("nodes",[]) if item.get("path")==selected),None)
                if not node: return None
                text=node.get("sourceBefore" if side=="baseline" else "sourceAfter")
                if text is None: return None
                blob=node.get("blobBefore" if side=="baseline" else "blobAfter")
                if (query.get("blob") or [None])[0] != blob: return None
                try:
                    start=int((query.get("start_line") or [1])[0]); end=int((query.get("end_line") or [500])[0])
                    if start<1 or end<start or end-start>=500: return None
                except ValueError: return None
                lines=text.splitlines(); end=min(end,len(lines))
                if start>end: return None
                return {"api_version":"v1","review_id":review_id,"commit":observation["baseline" if side=="baseline" else "proposal"],
                        "blob_id":blob,"path":selected,"start_line":start,"end_line":end,"text":"\n".join(lines[start-1:end]),
                        "truncated":node.get("sourceBeforeTruncated" if side=="baseline" else "sourceAfterTruncated",False)}
            return None

        def hld_json(self,report,path,query):
            status,payload=state.hld_request(report)
            if status!=200: return status,payload
            suffix=path.removeprefix(f"/v1/reviews/{report['id']}/hld")
            if suffix=="" and not query: return 200,payload
            if suffix=="/source" and set(query)=={"design_id","evidence_id"} and all(len(items)==1 for items in query.values()):
                source=state.hld_source(report,payload,query["design_id"][0],query["evidence_id"][0])
                return (200,source) if source else (404,{"error":"exact_hld_source_unavailable"})
            if suffix=="/export" and set(query)=={"view"} and len(query["view"])==1:
                if not payload["can_export"]: return 404,{"error":"hld_export_unavailable"}
                view=query["view"][0]
                permitted=payload.get("export_views",{})
                if (view=="diff" and not (permitted.get("baseline") is True and permitted.get("proposal") is True)) or (view!="diff" and permitted.get(view) is not True): return 404,{"error":"hld_export_unavailable"}
                if view=="diff":
                    delta=payload["delta"]
                    markdown=f"# HLD architecture delta\n\nBaseline: {payload['baseline']['snapshot']}\nProposal: {payload['proposal']['snapshot']}\nOutcome: {delta['outcome']}\n\n```mermaid\n{delta['mermaid']}\n```\n\n"+payload["baseline"]["markdown"]+"\n\n"+payload["proposal"]["markdown"]
                elif view in ("baseline","proposal","accepted") and payload.get(view): markdown=payload[view]["markdown"]
                else: return 404,{"error":"hld_export_unavailable"}
                if len(markdown.encode())>2*1024*1024: return 413,{"error":"hld_export_budget_reached"}
                return 200,{"api_version":"v1","review_id":report["id"],"markdown":markdown,"filename":f"codeshouldbehave-hld-{view}.md"}
            return 404,{"error":"hld_operation_unavailable"}

        def lld_json(self,report,path,query):
            suffix=path.removeprefix(f"/v1/reviews/{report['id']}/lld/")
            allowed={"catalog":{"parent_hld","component","cursor","limit"},
                     "detail":{"parent_hld","component","entry","baseline_entry","correspondence_kind","side"},
                     "history":{"parent_hld","component","entry","baseline_entry","correspondence_kind"},
                     "source":{"parent_hld","component","entry","baseline_entry","correspondence_kind","evidence","start_line","end_line"},
                     "export":{"parent_hld","component","entry","baseline_entry","correspondence_kind","view"}}
            if suffix not in allowed or set(query)-allowed[suffix] or any(len(values)!=1 for values in query.values()):
                return 400,{"error":"invalid_lld_query"}
            request={key:values[0] for key,values in query.items()}
            if any(len(value)>512 or any(ord(char)<32 for char in value) for value in request.values()): return 400,{"error":"invalid_lld_identity"}
            if not request.get("parent_hld") or (suffix!="catalog" and not request.get("entry")): return 400,{"error":"missing_lld_identity"}
            if suffix=="source" and not request.get("evidence"): return 400,{"error":"missing_lld_evidence"}
            if "correspondence_kind" in request and request["correspondence_kind"] not in ("rename","move"): return 400,{"error":"invalid_lld_correspondence"}
            if "side" in request and request["side"] not in ("baseline","proposal","diff"): return 400,{"error":"invalid_lld_side"}
            if "view" in request and request["view"] not in ("baseline","proposal","diff"): return 400,{"error":"invalid_lld_view"}
            for key in ("limit","start_line","end_line"):
                if key in request:
                    if not re.fullmatch(r"[1-9][0-9]{0,6}",request[key]): return 400,{"error":"invalid_lld_bound"}
                    request[key]=int(request[key])
            if request.get("limit",50)>100: return 400,{"error":"lld_page_limit"}
            return state.lld_request(report,"lld-"+suffix,request)

        def preferences(self, write=False):
            if not self.api_authorized(): self.send_error(HTTPStatus.FORBIDDEN); return
            if not state.app_auth: self.json_status(401,{"error":"GitHub App sign-in required"}); return
            try:
                identity=state.app_auth.access(state.repository_slug())
                request={}
                if write:
                    length=int(self.headers.get("Content-Length","0"))
                    if length<2 or length>65536: raise ValueError("Invalid body")
                    request=json.loads(self.rfile.read(length))
                status,value=state.native_onboarding_request("preferences-write" if write else "preferences-read", "", request, state.repository_slug(), identity["principal"], False)
                self.json_status(status,value)
            except AppError:
                self.json_status(401,{"error":"GitHub App sign-in required"})
            except (ValueError,TypeError,KeyError,OSError) as error:
                self.json_status(409 if "Preference conflict" in str(error) else 400,{"error":"Preference conflict" if "Preference conflict" in str(error) else "Preferences unavailable"})

        def do_GET(self) -> None:
            if not self.loopback_origin_allowed(): self.send_error(HTTPStatus.FORBIDDEN); return
            path = urlparse(self.path).path
            if path == "/v1/me/workspace-preferences": self.preferences(); return
            if path == "/":
                body = LANDING.replace(b"__CSB_SESSION__", state.session.encode())
                self.send_body_headers("text/html; charset=utf-8", len(body)); self.wfile.write(body); return
            if path == "/assets/github-mvp.js":
                self.send_body_headers("text/javascript; charset=utf-8", len(LANDING_JS)); self.wfile.write(LANDING_JS); return
            if path == "/onboarding":
                body=ONBOARDING.replace(b"<title>",f'<meta name="csb-local-session" content="{state.session}"><title>'.encode())
                self.send_body_headers("text/html; charset=utf-8",len(body)); self.wfile.write(body); return
            if path == "/workspace":
                body = WORKSPACE.replace(b"<title>", f'<meta name="csb-local-session" content="{state.session}"><title>'.encode())
                self.send_body_headers("text/html; charset=utf-8", len(body)); self.wfile.write(body); return
            assets = {"/assets/workspace-preferences.js":("text/javascript; charset=utf-8",PREFERENCES_JS),"/assets/workspace-registry.json":("application/json",PREFERENCES_REGISTRY),"/assets/onboarding.css":("text/css; charset=utf-8",ONBOARDING_CSS),"/assets/onboarding.js":("text/javascript; charset=utf-8",ONBOARDING_JS),"/assets/workspace.js":("text/javascript; charset=utf-8",WORKSPACE_JS),
                      "/assets/workspace.css":("text/css; charset=utf-8",WORKSPACE_CSS),
                      "/assets/mermaid.js":("text/javascript; charset=utf-8",MERMAID)}
            if path in assets:
                content_type, body = assets[path]; self.send_body_headers(content_type,len(body)); self.wfile.write(body); return
            if path.startswith("/v1/onboarding/"):
                if not self.api_authorized(): self.send_error(HTTPStatus.FORBIDDEN); return
                if parse_qs(urlparse(self.path).query)!={"security_domain":["local"]}: self.send_error(HTTPStatus.BAD_REQUEST); return
                match=re.fullmatch(r"/v1/onboarding/([A-Za-z0-9-]{1,128})",path)
                if not match: self.send_error(HTTPStatus.NOT_FOUND); return
                status,value=state.onboarding_request("read",match.group(1),{})
                self.json_status(status,value); return
            if path == "/api/status":
                provider = state.provider_status()
                app_status=state.app_auth.public_status() if state.app_auth else {}
                with state.lock:
                    payload = {**provider, **app_status,"native_profile": state.native_profile,"auth_running": state.auth_running,
                               "auth_code": app_status.get("auth_code",state.auth_code), "auth_error": app_status.get("auth_error",state.auth_error),
                               "review_running": state.review_running, "review_error": state.review_error,
                               "review_url": state.review_url, "review_generation": state.review_generation,
                               "review_pr": state.review_pr}
                if state.app_auth and not app_status["authenticated"]:
                    payload.pop("prs",None); payload.pop("repository",None); payload["review_url"]=None
                self.json(payload); return
            if path.startswith("/reviews/"):
                name = path.removeprefix("/reviews/")
                if not re.fullmatch(r"pr-[1-9][0-9]*\.html", name):
                    self.send_error(HTTPStatus.NOT_FOUND); return
                target = state.output / name
                if not target.is_file() or target.is_symlink():
                    self.send_error(HTTPStatus.NOT_FOUND); return
                report=state.report(name.removesuffix('.html'))
                if not report: self.send_error(HTTPStatus.NOT_FOUND,"Regenerate the review to create its typed artifact"); return
                try: state.require_app_access(report)
                except AppError: self.send_error(HTTPStatus.FORBIDDEN); return
                self.send_response(HTTPStatus.FOUND); self.send_header("Location",architecture_workspace_url(report['id'],bool(report['artifact'].get('hld')))); self.end_headers(); return
            if path == "/v1/prs" or path.startswith("/v1/prs/") or path.startswith("/v1/reviews/"):
                if not self.api_authorized(): self.send_error(HTTPStatus.UNAUTHORIZED); return
                query=parse_qs(urlparse(self.path).query)
                report=state.routed_report(path,lld_get=True) if "/lld/" in path else state.routed_report(path)
                if not report: self.send_error(HTTPStatus.NOT_FOUND); return
                try: self.current_access=state.require_app_access(None if path=="/v1/prs" else report)
                except AppError: self.send_error(HTTPStatus.FORBIDDEN); return
                if path==f"/v1/reviews/{report['id']}/hld" or path.startswith(f"/v1/reviews/{report['id']}/hld/"):
                    status,value=self.hld_json(report,path,query); self.json_status(status,value); return
                if path.startswith(f"/v1/reviews/{report['id']}/lld/"):
                    status,value=self.lld_json(report,path,query); self.json_status(status,value); return
                value=self.workspace_json(report,path,query)
                if value is None: self.send_error(HTTPStatus.NOT_FOUND); return
                self.json(value); return
            self.send_error(HTTPStatus.NOT_FOUND)

        def do_POST(self) -> None:
            if not self.loopback_origin_allowed(): self.send_error(HTTPStatus.FORBIDDEN); return
            path = urlparse(self.path).path
            lld=re.fullmatch(r"/v1/reviews/(pr-[1-9][0-9]*-[0-9a-f]{64})/lld/(preview|accept)",path)
            if lld:
                if not self.api_authorized(): self.send_error(HTTPStatus.UNAUTHORIZED); return
                report=state.report(lld.group(1))
                if not report: self.send_error(HTTPStatus.NOT_FOUND); return
                try:
                    length=int(self.headers.get("Content-Length","0"))
                    if length<2 or length>65536: raise ValueError
                    value=json.loads(self.rfile.read(length))
                    common={"parent_hld","component","entry","baseline_entry","correspondence_kind"}
                    allowed=common|({"design_id","interpretation","corrections","expected_revision","preview_id"} if lld.group(2)=="preview" else {"preview_id","expected_revision","input_fingerprint","known_gaps_accepted","activate_default"})
                    if not isinstance(value,dict) or set(value)-allowed or type(value.get("expected_revision")) is not int or value["expected_revision"]<0: raise ValueError
                    if not all(isinstance(value.get(key),str) and len(value[key])<=512 for key in ("parent_hld","entry")): raise ValueError
                    for key in ("component","baseline_entry","correspondence_kind"):
                        if key in value and value[key] is not None and (not isinstance(value[key],str) or len(value[key])>512 or any(ord(char)<32 for char in value[key])): raise ValueError
                        if value.get(key) is None: value.pop(key,None)
                    if "correspondence_kind" in value and value["correspondence_kind"] not in ("rename","move"): raise ValueError
                    if lld.group(2)=="preview" and (not isinstance(value.get("corrections"),list) or len(value["corrections"])>200): raise ValueError
                except (ValueError,json.JSONDecodeError): self.send_error(HTTPStatus.BAD_REQUEST); return
                status,value=state.lld_request(report,"lld-"+lld.group(2),value)
                self.json_status(status,value); return
            hld=re.fullmatch(r"/v1/reviews/(pr-[1-9][0-9]*-[0-9a-f]{64})/hld/(preview|accept)",path)
            if hld:
                if not self.api_authorized(): self.send_error(HTTPStatus.UNAUTHORIZED); return
                report=state.report(hld.group(1))
                if not report: self.send_error(HTTPStatus.NOT_FOUND); return
                try:
                    length=int(self.headers.get("Content-Length","0"))
                    if length<2 or length>65536: raise ValueError
                    value=json.loads(self.rfile.read(length))
                    if not isinstance(value,dict): raise ValueError
                    allowed={"design_id","interpretation","corrections","expected_revision","preview_id"} if hld.group(2)=="preview" else {"preview_id","expected_revision","input_fingerprint","known_gaps_accepted","activate_default"}
                    if set(value)-allowed or type(value.get("expected_revision")) is not int or value["expected_revision"]<0: raise ValueError
                    if hld.group(2)=="preview" and (not isinstance(value.get("corrections"),list) or len(value["corrections"])>200): raise ValueError
                except (ValueError,json.JSONDecodeError): self.send_error(HTTPStatus.BAD_REQUEST); return
                status,value=state.hld_request(report,"hld-"+hld.group(2),value)
                self.json_status(status,value); return
            if not self.authorized():
                self.send_error(HTTPStatus.FORBIDDEN); return
            if path == "/v1/onboarding" or path.startswith("/v1/onboarding/"):
                try:
                    length=int(self.headers.get("Content-Length","0"))
                    if length<2 or length>65536: raise ValueError
                    value=json.loads(self.rfile.read(length))
                    if not isinstance(value,dict): raise ValueError
                except (ValueError,json.JSONDecodeError): self.send_error(HTTPStatus.BAD_REQUEST); return
                match=re.fullmatch(r"/v1/onboarding/([A-Za-z0-9-]{1,128})/(scan|detect|inspect|cancel|decisions|provider-decisions|integration-decisions|confirm|activate|defaults|drift)",path)
                if path!="/v1/onboarding" and not match: self.send_error(HTTPStatus.NOT_FOUND); return
                status,value=state.onboarding_request(match.group(2) if match else "create",match.group(1) if match else None,value)
                self.json_status(status,value); return
            if path == "/api/auth/start":
                state.start_auth(); self.json({"accepted": True}); return
            if path == "/api/auth/disconnect" and state.app_auth:
                state.app_auth.logout()
                with state.lock:
                    state.provider_status_cache={}; state.provider_status_until=0; state.review_url=None
                self.json({"disconnected":True}); return
            if path == "/api/reviews":
                try: state.require_app_access()
                except AppError: self.send_error(HTTPStatus.FORBIDDEN); return
                try:
                    length = int(self.headers.get("Content-Length", "0"))
                    if length < 1 or length > 1024: raise ValueError
                    value = json.loads(self.rfile.read(length))
                    pr = value.get("pr")
                    if type(pr) is not int or pr < 1: raise ValueError
                except (ValueError, json.JSONDecodeError, AttributeError):
                    self.send_error(HTTPStatus.BAD_REQUEST); return
                generation = state.start_review(pr)
                if generation is None:
                    self.send_error(HTTPStatus.CONFLICT); return
                self.json({"accepted": True, "generation": generation}); return
            self.send_error(HTTPStatus.NOT_FOUND)

        def do_PUT(self) -> None:
            if not self.loopback_origin_allowed(): self.send_error(HTTPStatus.FORBIDDEN); return
            if urlparse(self.path).path == "/v1/me/workspace-preferences": self.preferences(write=True); return
            path=urlparse(self.path).path; match=re.fullmatch(r"/v1/reviews/(pr-[1-9][0-9]*-[0-9a-f]{64})/annotations",path)
            if not match or not self.api_authorized(): self.send_error(HTTPStatus.FORBIDDEN); return
            report=state.report(match.group(1))
            if not report: self.send_error(HTTPStatus.NOT_FOUND); return
            try: access=state.require_app_access(report)
            except AppError: self.send_error(HTTPStatus.FORBIDDEN); return
            try:
                length=int(self.headers.get("Content-Length","0"))
                if length<2 or length>65536: raise ValueError
                value=json.loads(self.rfile.read(length))
                if not isinstance(value,dict): raise ValueError
                if type(value.get("expected_version")) is not int or value["expected_version"]<0: raise ValueError
                if not isinstance(value.get("id"),str) or len(value["id"])>128: raise ValueError
            except (ValueError,json.JSONDecodeError): self.send_error(HTTPStatus.BAD_REQUEST); return
            principal=access["principal"] if access else "local-reviewer"
            with state.lock:
                items=state.annotations.setdefault(match.group(1),[]); current=next((item for item in items if item.get("id")==value["id"]),None)
                if current and current["owner_principal"]!=principal: self.send_error(HTTPStatus.FORBIDDEN); return
                if value["expected_version"]!=(current["version"] if current else 0): self.send_error(HTTPStatus.CONFLICT); return
                written={**value,"review_id":match.group(1),"proposal_commit":report["observation"]["proposal"],"owner_principal":principal,
                         "version":value["expected_version"]+1,"updated_at_ms":int(time.time()*1000)}
                if current: items[items.index(current)]=written
                else: items.append(written)
            self.json(written)

        def log_message(self, format: str, *args) -> None:
            return

    return Handler


def main() -> None:
    parser = argparse.ArgumentParser(description="CodeShouldBehave GitHub browser MVP")
    parser.add_argument("--repo", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--port", type=int, default=3195)
    parser.add_argument("--native-profile", choices=("debug", "release"), default="release",
                        help="Trusted native build profile; release is optimized for normal use")
    parser.add_argument("--auth-profile",choices=("gh-cli","github-app"),default="github-app")
    parser.add_argument("--github-app-config",type=Path,default=ROOT/"config"/"github-app.json")
    parser.add_argument("--disable-new-onboarding",action="store_true",help="Keep confirmed authorized reviews; disable new scans and onboarding checks")
    parser.add_argument("--disable-new-lld",action="store_true",help="Disable new LLD indexing, detail generation and confirmations; preserve immutable records and source reviews")
    parser.add_argument("--disable-new-hld",action="store_true",help="Keep authorized HLD history and ordinary reviews; disable new HLD generation/confirmation")
    args = parser.parse_args()
    if not (1 <= args.port <= 65535): parser.error("port must be between 1 and 65535")
    ensure_native_binaries(args.native_profile)
    state = State(args.repo, args.output, app_config(args.github_app_config) if args.auth_profile=="github-app" else None,not args.disable_new_onboarding,not args.disable_new_hld,not args.disable_new_lld,args.native_profile)
    server = ThreadingHTTPServer(("127.0.0.1", args.port), handler(state))
    print(f"CodeShouldBehave GitHub MVP: http://127.0.0.1:{args.port}/", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
