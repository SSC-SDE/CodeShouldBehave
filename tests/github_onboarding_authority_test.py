"""Atomic confirmation/default publication with offline App and SQLite fixtures."""
import json
import gc
import http.client
import os
from contextlib import contextmanager
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"scripts"))
import github_mvp_server as module


@contextmanager
def fixture_database(path):
    connection=sqlite3.connect(path)
    try:
        with connection:yield connection
    finally:connection.close()


class OnboardingAuthorityTest(unittest.TestCase):
    def test_second_host_using_same_output_is_denied_until_owner_is_finalized(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);first=module.State(root,root)
            self.assertEqual((root/".host.lock").stat().st_mode&0o777,0o600)
            with self.assertRaisesRegex(ValueError,"already in use"):
                module.State(root,root)
            del first
            gc.collect()
            next_host=module.State(root,root)
            self.assertFalse(next_host._host_lock.closed)
            next_host._host_lock.close()

    def test_distinct_outputs_have_independent_lifetime_locks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);first=module.State(root,root/"first");second=module.State(root,root/"second")
            try:
                self.assertNotEqual(first._host_lock.fileno(),second._host_lock.fileno())
                self.assertFalse(first._host_lock.closed)
                self.assertFalse(second._host_lock.closed)
                self.assertEqual((root/"first"/".host.lock").stat().st_mode&0o777,0o600)
                self.assertEqual((root/"second"/".host.lock").stat().st_mode&0o777,0o600)
            finally:
                first._host_lock.close();second._host_lock.close()

    def test_other_process_is_fenced_by_output_lock_and_can_start_after_release(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);first=module.State(root,root)
            code="""import sys
sys.path.insert(0,sys.argv[1])
from pathlib import Path
from github_mvp_server import State
try:
    state=State(Path(sys.argv[2]),Path(sys.argv[2]))
except ValueError:
    sys.exit(42)
"""
            args=[sys.executable,"-c",code,str(module.ROOT/"scripts"),str(root)]
            kwargs={"env":{"PYTHONDONTWRITEBYTECODE":"1"},"capture_output":True,"text":True,"timeout":10}
            try:
                blocked=subprocess.run(args,**kwargs)
                self.assertEqual(blocked.returncode,42,blocked.stderr)
                self.assertEqual(blocked.stdout,"")
            finally:first._host_lock.close()
            allowed=subprocess.run(args,**kwargs)
            self.assertEqual(allowed.returncode,0,allowed.stderr)

    def test_host_lock_never_follows_symlink_or_accepts_nonregular_file(self):
        for kind in ("symlink","directory"):
            with self.subTest(kind=kind),tempfile.TemporaryDirectory() as temporary:
                root=Path(temporary);outside=root/"sentinel";outside.write_text("private fixture")
                mode=outside.stat().st_mode&0o777
                lock=root/".host.lock"
                if kind=="symlink":lock.symlink_to(outside)
                else:lock.mkdir()
                with self.assertRaises((OSError,ValueError)):module.State(root,root)
                self.assertEqual(outside.read_text(),"private fixture")
                self.assertEqual(outside.stat().st_mode&0o777,mode)

    def fixture(self, root):
        root=root.resolve()
        state=module.State(root,root)
        directory=root/".onboarding"
        directory.mkdir(mode=0o700)
        database=directory/"onboarding.sqlite"
        with fixture_database(database) as connection:
            connection.execute("CREATE TABLE accepted (name TEXT PRIMARY KEY, value TEXT)")
            connection.executemany("INSERT INTO accepted VALUES (?,?)",[("revision","old"),("defaults","old")])
        auth=SimpleNamespace(lock=threading.Lock(),generation=7,_token="fixture-token",
                             _token_deadline=100,clock=lambda:0)
        auth.access=lambda slug:{"principal":"github-user:9","maintainer":True}
        state.app_auth=auth
        return state,database

    def values(self, database):
        with fixture_database(database) as connection:
            return dict(connection.execute("SELECT name,value FROM accepted"))

    def database_copy(self,payload,*,interrupt=False):
        # This is an offline fixture implementation of the native transport.
        # Application database copies are performed only by bundled Rust SQLite.
        source=Path(payload["request"]["source"])
        destination=Path(payload["request"]["destination"])
        self.assertTrue(source.is_absolute() and destination.is_absolute())
        self.assertTrue(source.is_file() and destination.is_file())
        try:
            with fixture_database(source) as original,fixture_database(destination) as target:
                def progress(status,remaining,total):
                    if interrupt and status!=sqlite3.SQLITE_DONE:raise RuntimeError("fixture interruption")
                original.backup(target,pages=64,progress=progress)
        except RuntimeError:
            return SimpleNamespace(returncode=1,stdout=json.dumps({"status":503,"value":{"error":"local_database_copy_unavailable"}}))
        return SimpleNamespace(returncode=0,stdout=json.dumps({"status":200,"value":{"copied":True}}))

    def command(self, state, live, *, status=200, after=None, large=False):
        def run(args,**kwargs):
            payload=json.loads(kwargs["input_text"])
            self.assertTrue(kwargs["clean_environment"])
            if payload["action"]=="database-copy":
                self.assertEqual(kwargs["timeout"],10)
                return self.database_copy(payload)
            staged=Path(args[2])/"onboarding.sqlite"
            self.assertNotEqual(staged,live)
            self.assertEqual(staged.parent.parent,live.parent)
            self.assertEqual(staged.parent.stat().st_mode&0o777,0o700)
            self.assertEqual(staged.stat().st_mode&0o777,0o600)
            self.assertEqual(payload["principal"],"github-user:9")
            self.assertIn(payload["action"],("confirm","activate"))
            self.assertEqual(self.values(live),{"revision":"old","defaults":"old"})
            with fixture_database(staged) as connection:
                connection.execute("PRAGMA journal_mode=WAL")
                connection.execute("UPDATE accepted SET value='new'")
                if large:
                    connection.execute("CREATE TABLE pages (data BLOB)")
                    connection.execute("INSERT INTO pages VALUES (zeroblob(1048576))")
            if after:after()
            return SimpleNamespace(returncode=0,stdout=json.dumps({"status":status,"value":{"result":"fixture"}}))
        state.command=run

    def invoke(self,state,action="activate"):
        # Avoid reliance on the operator's built binary. Every command in these
        # tests is the fake native process above; no network or live DB is used.
        with patch.object(module.Path,"is_file",return_value=True):
            return state.native_onboarding_request(action,"confirmed",{},"owner/repo","github-user:9",True)

    def assert_cleaned(self,database):
        self.assertEqual(list(database.parent.glob(".authority-*")),[])

    def test_authorized_confirmation_and_activation_publish_all_rows(self):
        for action in ("confirm","activate"):
            with self.subTest(action=action),tempfile.TemporaryDirectory() as temporary:
                state,database=self.fixture(Path(temporary))
                calls=[]
                def access(slug):
                    calls.append(slug)
                    self.assertEqual(self.values(database),{"revision":"old","defaults":"old"})
                    return {"principal":"github-user:9","maintainer":True}
                state.app_auth.access=access
                self.command(state,database)
                self.assertEqual(self.invoke(state,action),(200,{"result":"fixture"}))
                self.assertEqual(calls,["owner/repo"])
                self.assertEqual(self.values(database),{"revision":"new","defaults":"new"})
                self.assert_cleaned(database)

    def test_revoked_principal_or_maintainer_never_publishes_staged_changes(self):
        denied=({"principal":"github-user:9","maintainer":False},
                {"principal":"github-user:10","maintainer":True},None)
        for current in denied:
            with self.subTest(current=current),tempfile.TemporaryDirectory() as temporary:
                state,database=self.fixture(Path(temporary))
                def access(slug):
                    if current is None:raise module.AppError("installation_revoked")
                    return current
                state.app_auth.access=access
                self.command(state,database)
                self.assertEqual(self.invoke(state)[0],404)
                self.assertEqual(self.values(database),{"revision":"old","defaults":"old"})
                self.assert_cleaned(database)

    def test_cli_profile_rechecks_current_login_and_maintainer_before_publication(self):
        cases=(("fixture-user",{"pull":True,"push":True},200),
               ("fixture-user",{"pull":True,"push":False},404),
               ("other-user",{"pull":True,"push":True},404),
               ("fixture-user",{"pull":False,"admin":True},404),
               ("fixture-user",{"pull":True,"push":"true"},404))
        for login,permissions,expected in cases:
            with self.subTest(login=login,permissions=permissions),tempfile.TemporaryDirectory() as temporary:
                state,database=self.fixture(Path(temporary));state.app_auth=None;calls=[]
                def run(args,**kwargs):
                    if args[0]=="gh":
                        calls.append(args)
                        value=login if args[2]=="user" else json.dumps(permissions)
                        return SimpleNamespace(returncode=0,stdout=value)
                    payload=json.loads(kwargs["input_text"])
                    if payload["action"]=="database-copy":return self.database_copy(payload)
                    staged=Path(args[2])/"onboarding.sqlite"
                    self.assertNotEqual(staged,database)
                    with fixture_database(staged) as connection:
                        connection.execute("UPDATE accepted SET value='new'")
                    return SimpleNamespace(returncode=0,stdout=json.dumps({"status":200,"value":{"accepted":True}}))
                state.command=run
                with patch.object(module.Path,"is_file",return_value=True):
                    status,_=state.native_onboarding_request("activate","accepted",{},"owner/repo","github:fixture-user",True)
                self.assertEqual(status,expected)
                self.assertEqual(calls,[["gh","api","user","--jq",".login"],["gh","api","repos/owner/repo","--jq",".permissions"]])
                value="new" if expected==200 else "old"
                self.assertEqual(self.values(database),{"revision":value,"defaults":value})
                self.assert_cleaned(database)

    def test_logout_reconnect_or_expiry_between_native_work_and_publish_is_denied(self):
        for change in ("generation","token","expiry"):
            with self.subTest(change=change),tempfile.TemporaryDirectory() as temporary:
                state,database=self.fixture(Path(temporary))
                def after():
                    with state.app_auth.lock:
                        if change=="generation":state.app_auth.generation+=1
                        elif change=="token":state.app_auth._token="other-fixture-token"
                        else:state.app_auth._token_deadline=0
                self.command(state,database,after=after)
                self.assertEqual(self.invoke(state)[0],404)
                self.assertEqual(self.values(database),{"revision":"old","defaults":"old"})
                self.assert_cleaned(database)

    def test_native_conflict_or_failure_keeps_live_database(self):
        for result in ("conflict","timeout","bad-json","failed-process"):
            with self.subTest(result=result),tempfile.TemporaryDirectory() as temporary:
                state,database=self.fixture(Path(temporary))
                self.command(state,database,status=409)
                native=state.command
                def run(args,**kwargs):
                    response=native(args,**kwargs)
                    if json.loads(kwargs["input_text"])["action"]=="database-copy":return response
                    if result=="timeout":raise subprocess.TimeoutExpired(args,600)
                    if result=="bad-json":return SimpleNamespace(returncode=0,stdout="invalid")
                    if result=="failed-process":return SimpleNamespace(returncode=1,stdout=response.stdout)
                    return response
                state.command=run
                self.assertEqual(self.invoke(state)[0],409 if result=="conflict" else 503)
                self.assertEqual(self.values(database),{"revision":"old","defaults":"old"})
                self.assert_cleaned(database)

    def test_interrupted_publication_backup_rolls_back_entire_destination(self):
        with tempfile.TemporaryDirectory() as temporary:
            state,database=self.fixture(Path(temporary));self.command(state,database,large=True)
            native=state.command;interrupted=[]
            def command(args,**kwargs):
                payload=json.loads(kwargs["input_text"])
                if payload["action"]=="database-copy" and Path(payload["request"]["destination"])==database:
                    interrupted.append(True)
                    return self.database_copy(payload,interrupt=True)
                return native(args,**kwargs)
            state.command=command
            self.assertEqual(self.invoke(state)[0],503)
            self.assertEqual(interrupted,[True])
            self.assertEqual(self.values(database),{"revision":"old","defaults":"old"})
            with fixture_database(database) as connection:
                self.assertEqual(connection.execute("SELECT name FROM sqlite_master WHERE name='pages'").fetchall(),[])
                self.assertEqual(connection.execute("PRAGMA integrity_check").fetchone()[0],"ok")
            self.assert_cleaned(database)

    def test_both_database_copies_use_native_transport_without_python_sqlite(self):
        with tempfile.TemporaryDirectory() as temporary:
            state,database=self.fixture(Path(temporary));self.command(state,database)
            native=state.command;copies=[]
            def command(args,**kwargs):
                payload=json.loads(kwargs["input_text"])
                if payload["action"]=="database-copy":copies.append(payload["request"])
                return native(args,**kwargs)
            state.command=command
            self.assertFalse(hasattr(module,"sqlite3"))
            self.assertEqual(self.invoke(state)[0],200)
            self.assertEqual(len(copies),2)
            self.assertEqual(copies[0]["source"],str(database))
            self.assertEqual(copies[0]["destination"],copies[1]["source"])
            self.assertEqual(copies[1]["destination"],str(database))
            self.assertEqual(self.values(database),{"revision":"new","defaults":"new"})
            self.assert_cleaned(database)

    @unittest.skipUnless((module.ROOT/"target/debug/csb-onboarding").is_file(),"Build the bundled csb-onboarding adapter for native-copy regression")
    def test_real_native_bridge_copies_cold_wal_after_all_database_connections_close(self):
        with tempfile.TemporaryDirectory() as temporary:
            state,database=self.fixture(Path(temporary))
            connection=sqlite3.connect(database)
            try:
                self.assertEqual(connection.execute("PRAGMA journal_mode=WAL").fetchone()[0],"wal")
                connection.execute("CREATE TABLE cold_wal_evidence (id INTEGER)")
                connection.execute("INSERT INTO cold_wal_evidence VALUES (23)")
                connection.commit()
                self.assertEqual(connection.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()[0],0)
            finally:connection.close()
            # Some Python SQLite builds retain empty WAL metadata. Model the
            # normal native close only after all frames were checkpointed.
            wal=Path(str(database)+"-wal")
            if wal.exists():
                self.assertEqual(wal.stat().st_size,0)
                wal.unlink()
            Path(str(database)+"-shm").unlink(missing_ok=True)
            self.assertFalse(Path(str(database)+"-wal").exists())
            self.assertFalse(Path(str(database)+"-shm").exists())
            real_command=state.command
            self.command(state,database)
            native_operation=state.command;copies=[]
            def command(args,**kwargs):
                payload=json.loads(kwargs["input_text"])
                if payload["action"]=="database-copy":
                    copies.append(payload["request"])
                    return real_command(args,**kwargs)
                with fixture_database(Path(args[2])/"onboarding.sqlite") as stage:
                    self.assertEqual(stage.execute("SELECT id FROM cold_wal_evidence").fetchall(),[(23,)])
                return native_operation(args,**kwargs)
            state.command=command
            self.assertEqual(self.invoke(state),(200,{"result":"fixture"}))
            self.assertEqual(len(copies),2)
            self.assertEqual(self.values(database),{"revision":"new","defaults":"new"})
            self.assert_cleaned(database)

    @unittest.skipUnless((module.ROOT/"target/debug/csb-onboarding").is_file(),"Build the bundled csb-onboarding adapter for HTTP-confirm regression")
    def test_real_native_http_confirmation_activates_context_and_hides_copy_action(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary).resolve();repository=root/"repo";repository.mkdir()
            (repository/"src").mkdir()
            (repository/"Cargo.toml").write_text('[package]\nname="authority_fixture"\nversion="0.1.0"\nedition="2021"\n')
            (repository/"src/main.rs").write_text("fn main() { start(); }\nfn start() {}\n")
            environment={key:os.environ[key] for key in ("PATH","LANG","TMPDIR") if key in os.environ}
            environment.update(GIT_CONFIG_NOSYSTEM="1",GIT_CONFIG_GLOBAL=os.devnull)
            def git(*args):
                result=subprocess.run(["git","-c","core.hooksPath="+os.devnull,"-C",str(repository),*args],
                    env=environment,capture_output=True,text=True,timeout=10,check=True)
                return result.stdout.strip()
            git("init","--template=","-q");git("add",".")
            git("-c","user.name=Authority fixture","-c","user.email=fixture@example.invalid","commit","--no-gpg-sign","-qm","Fixture")
            commit=git("rev-parse","HEAD")
            state=module.State(repository,root/"output")
            state.repository_slug=lambda:"owner/repo"
            state.authenticated=lambda:True
            state.app_auth=SimpleNamespace(lock=threading.Lock(),generation=1,_token="fixture-token",clock=lambda:0,
                _token_deadline=100,access=lambda slug:{"principal":"github-user:9","maintainer":True})
            server=module.ThreadingHTTPServer(("127.0.0.1",0),module.handler(state))
            thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
            def post(path,request):
                client=http.client.HTTPConnection("127.0.0.1",server.server_port,timeout=15)
                try:
                    client.request("POST",path,json.dumps(request),{"X-CSB-Session":state.session,"Content-Type":"application/json"})
                    response=client.getresponse();body=response.read()
                    return response.status,json.loads(body) if response.getheader("Content-Type","").startswith("application/json") else None
                finally:client.close()
            identity={"security_domain":"local","provider":"github","owner":"owner","name":"repo"}
            session_id="native-authority"
            try:
                status,session=post("/v1/onboarding",{"id":session_id,"reference":commit,"scope_version":"native-fixture-v1",
                    "scope":{"primary_repository":identity,"primary_include":["."],"primary_exclude":[],"contexts":[],
                             "contracts":[],"export_allowed":False,"model_destinations":[]}})
                self.assertEqual(status,200,session)
                def mutate(action,**fields):
                    result,value=post(f"/v1/onboarding/{session_id}/{action}",{
                        "security_domain":"local","expected_revision":session["revision"],**fields})
                    self.assertEqual(result,200,value)
                    return value
                for _ in range(16):
                    session=mutate("scan")
                    if session.get("inventory",{}).get("complete"):break
                self.assertTrue(session["inventory"]["complete"])
                session=mutate("detect");session=mutate("inspect")
                self.assertTrue(session["components"])
                for original in session["components"]:
                    session=mutate("decisions",component={**original,"decision":"confirmed"})
                self.assertEqual(session["providers"],[])
                self.assertEqual(session["integrations"],[])
                self.assertTrue(session["confirmation_readiness"]["ready"])
                session=mutate("confirm",proposal_fingerprint=session["proposal_fingerprint"],known_gaps_accepted=True,activate_defaults=True)
                self.assertEqual(session["state"],"confirmed")
                self.assertTrue(session["review_defaults_active"])
                status,accepted=post(f"/v1/onboarding/{session_id}/defaults",{
                    "security_domain":"local","expected_revision":0,"repository":identity})
                self.assertEqual(status,200,accepted)
                self.assertEqual(accepted["commit"],commit)
                self.assertFalse(accepted["export_allowed"])
                self.assertTrue(accepted["usable"])
                self.assertEqual(post(f"/v1/onboarding/{session_id}/database-copy",{
                    "source":"/not-authorized","destination":"/not-authorized"})[0],404)
                database=state.output/".onboarding/onboarding.sqlite"
                self.assertEqual(database.stat().st_mode&0o777,0o600)
                self.assert_cleaned(database)
            finally:
                server.shutdown();server.server_close();thread.join(timeout=2)
                state._host_lock.close()

    def test_native_copy_timeout_and_invalid_success_are_sanitized_without_publication(self):
        for invalid in ("timeout","bad-json","false-success"):
            with self.subTest(invalid=invalid),tempfile.TemporaryDirectory() as temporary:
                state,database=self.fixture(Path(temporary))
                def command(args,**kwargs):
                    self.assertEqual(json.loads(kwargs["input_text"])["action"],"database-copy")
                    self.assertEqual(kwargs["timeout"],10)
                    if invalid=="timeout":raise subprocess.TimeoutExpired(args,10,output="SECRET_SENTINEL")
                    value="invalid" if invalid=="bad-json" else json.dumps({"status":200,"value":{"copied":False}})
                    return SimpleNamespace(returncode=0,stdout=value)
                state.command=command
                self.assertEqual(self.invoke(state),(503,{"error":"onboarding_adapter_unavailable"}))
                self.assertEqual(self.values(database),{"revision":"old","defaults":"old"})
                self.assert_cleaned(database)

    def test_read_and_scan_reject_existing_database_or_wal_symlink_before_native_use(self):
        for action,kind in (("read","database"),("scan","wal")):
            with self.subTest(action=action,kind=kind),tempfile.TemporaryDirectory() as temporary:
                root=Path(temporary);state,database=self.fixture(root);sentinel=root/"outside";sentinel.write_text("private fixture")
                if kind=="database":database.unlink();database.symlink_to(sentinel)
                else:Path(str(database)+"-wal").symlink_to(sentinel)
                state.command=lambda *args,**kwargs:self.fail("native invoked with symlink state")
                with patch.object(module.Path,"is_file",return_value=True):
                    response=state.native_onboarding_request(action,"accepted",{},"owner/repo","github-user:9",True)
                self.assertEqual(response,(503,{"error":"onboarding_adapter_unavailable"}))
                self.assertEqual(sentinel.read_text(),"private fixture")

    def test_committed_wal_is_included_in_staged_copy(self):
        with tempfile.TemporaryDirectory() as temporary:
            state,database=self.fixture(Path(temporary))
            connection=sqlite3.connect(database)
            try:
                connection.execute("PRAGMA journal_mode=WAL")
                connection.execute("CREATE TABLE wal_evidence (id INTEGER)")
                connection.execute("INSERT INTO wal_evidence VALUES (19)")
                connection.commit()
                self.command(state,database)
                native=state.command
                def run(args,**kwargs):
                    if json.loads(kwargs["input_text"])["action"]=="database-copy":return native(args,**kwargs)
                    with fixture_database(Path(args[2])/"onboarding.sqlite") as stage:
                        self.assertEqual(stage.execute("SELECT id FROM wal_evidence").fetchall(),[(19,)])
                    return native(args,**kwargs)
                state.command=run
                self.assertEqual(self.invoke(state)[0],200)
            finally:connection.close()
            self.assert_cleaned(database)

    def test_symlink_directory_database_and_wal_are_not_followed(self):
        for kind in ("directory","database","wal","shm"):
            with self.subTest(kind=kind),tempfile.TemporaryDirectory() as temporary:
                root=Path(temporary);state,database=self.fixture(root)
                outside=root/"outside";outside.mkdir();sentinel=outside/"sentinel";sentinel.write_text("private fixture")
                if kind=="directory":
                    database.unlink();database.parent.rmdir();database.parent.symlink_to(outside)
                elif kind=="database":database.unlink();database.symlink_to(sentinel)
                else:Path(str(database)+( "-wal" if kind=="wal" else "-shm")).symlink_to(sentinel)
                state.command=lambda *args,**kwargs:self.fail("native process invoked for unsafe state")
                self.assertEqual(self.invoke(state)[0],503)
                self.assertEqual(sentinel.read_text(),"private fixture")
                self.assertEqual(list(outside.iterdir()),[sentinel])

    def test_oversized_or_missing_database_does_not_start_native_work(self):
        for oversized in (False,True):
            with self.subTest(oversized=oversized),tempfile.TemporaryDirectory() as temporary:
                state,database=self.fixture(Path(temporary))
                if oversized:
                    with database.open("r+b") as handle:handle.truncate(64*1024*1024+1)
                else:database.unlink()
                state.command=lambda *args,**kwargs:self.fail("native process invoked for invalid state")
                self.assertEqual(self.invoke(state)[0],503)
                self.assert_cleaned(database)

    def test_other_actions_use_live_state_and_drift_override_stays_read_only(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);state,database=self.fixture(root);calls=[]
            def command(args,**kwargs):
                calls.append(args)
                self.assertEqual(Path(args[2]),database.parent)
                return SimpleNamespace(returncode=0,stdout=json.dumps({"status":200,"value":{"unchanged":True}}))
            state.command=command
            with patch.object(module.Path,"is_file",return_value=True):
                self.assertEqual(state.native_onboarding_request("drift","accepted",{},"owner/repo","github-user:9",False,source_repository=root/"mirror.git")[0],200)
                self.assertEqual(state.native_onboarding_request("activate","accepted",{},"owner/repo","github-user:9",True,source_repository=root/"mirror.git")[0],400)
            self.assertEqual(len(calls),1)
            self.assertEqual(calls[0][1],str(root/"mirror.git"))
            self.assertEqual(self.values(database),{"revision":"old","defaults":"old"})
            self.assert_cleaned(database)


if __name__=="__main__":unittest.main()
