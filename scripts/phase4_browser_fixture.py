#!/usr/bin/env python3
"""Generate private offline HLD reviewer fixtures; execute no reviewed code.
Build: cargo +1.87.0 build --locked --all-features --bins
Run: python3 scripts/phase4_browser_fixture.py NEW_DIRECTORY
This is synthetic technical QA, not live GitHub or human acceptance.
"""
import argparse
import copy
import json
import os
from pathlib import Path
import subprocess

PROJECT = Path(__file__).resolve().parents[1]


def build(root):
    root = root.resolve()
    root.mkdir(mode=0o700)
    repo, output, commands = root / "repo", root / "output", root / "bin"
    state = output / ".onboarding"
    for directory in (repo, output, commands, state):
        directory.mkdir(mode=0o700)
    env = {**os.environ, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull, "GIT_TERMINAL_PROMPT": "0"}

    def run(args, **kwargs):
        result = subprocess.run(args, env=env, capture_output=True, text=True, timeout=30, **kwargs)
        if result.returncode:
            raise RuntimeError("Fixture command failed: " + result.stderr[:500])
        return result.stdout.strip()

    def git(*args):
        return run(["git", "-c", "core.hooksPath=" + os.devnull, "-C", str(repo), *args])

    def write(path, text):
        target = repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)

    def commit(name):
        git("add", ".")
        git("-c", "user.name=HLD fixture", "-c", "user.email=fixture@example.invalid", "commit", "--no-gpg-sign", "-qm", name)
        return git("rev-parse", "HEAD")

    git("init", "-q")
    git("remote", "add", "origin", "https://github.com/fixture/hld-review.git")
    write("Cargo.toml", '[workspace]\nmembers=["api","domain"]\nresolver="2"\n')
    write("api/Cargo.toml", '[package]\nname="payment-api"\nversion="0.1.0"\n[dependencies]\npayment-domain={path="../domain"}\n')
    write("api/src/lib.rs", "use payment_domain::charge;\npub fn handle() -> u32 { charge() }\nfn private_helper() -> u32 { 1 }\n")
    write("api/vercel.json", '{"version":2,"routes":[{"src":"/payments","dest":"api.js"}]}\n')
    write("domain/Cargo.toml", '[package]\nname="payment-domain"\nversion="0.1.0"\n')
    write("domain/src/lib.rs", "pub fn charge() -> u32 { 1 }\n")
    write("docs/architecture.md", "# Payment design intent\n\nIntent: Payments API calls the payment domain.\nThis document is not proof of runtime behavior.\n")
    write(".env", "PRIVATE_HLD_SENTINEL=never-retain\n")
    write("excluded/private.rs", "pub fn PRIVATE_EXCLUDED_SENTINEL() {}\n")
    baseline = commit("Pinned HLD baseline")
    slug = "fixture/hld-review"
    identity = {"security_domain": "local", "provider": "github", "owner": "fixture", "name": "hld-review"}
    session_id = "hld-browser-session"

    def invoke(action, request=None):
        payload = {"principal": "github:fixture-user", "maintainer": True, "action": action, "id": session_id, "request": request or {}}
        value = json.loads(run([str(PROJECT / "target/debug/csb-onboarding"), str(repo), str(state), slug], input=json.dumps(payload)))
        assert value["status"] == 200, (action, value)
        return value["value"]

    session = invoke("create", {"id": session_id, "reference": baseline, "scope_version": "hld-fixture-boundary", "scope": {
        "primary_repository": identity, "primary_include": ["."], "primary_exclude": ["excluded"], "contexts": [], "contracts": [], "export_allowed": True, "model_destinations": []}})

    def mutate(action, **fields):
        nonlocal session
        session = invoke(action, {"security_domain": "local", "expected_revision": session["revision"], **fields})

    for _ in range(32):
        mutate("scan")
        if session["inventory"]["complete"]:
            break
    assert session["inventory"]["complete"]
    mutate("detect")
    mutate("inspect")
    for original in session["components"]:
        component = copy.deepcopy(original)
        component_root = component["roots"][0]
        component["name"] = {".": "Payment workspace", "api": "Payments API", "domain": "Payment domain library"}[component_root]
        component["role"] = {".": "workspace", "api": "backend", "domain": "library"}[component_root]
        component["decision"] = "corrected"
        mutate("decisions", component=component)
    for index, original in enumerate(session["providers"]):
        provider = copy.deepcopy(original)
        provider["decision"] = "confirmed"
        mutate("provider-decisions", provider_index=index, provider=provider)
    for original in session["integrations"]:
        mutate("integration-decisions", integration_id=original["id"], decision="include")
    mutate("confirm", proposal_fingerprint=session["proposal_fingerprint"], known_gaps_accepted=True, activate_defaults=True)
    context = invoke("defaults", {"security_domain": "local", "expected_revision": 0, "repository": identity})
    context_path = root / "accepted.json"
    context_path.write_text(json.dumps(context))
    context_path.chmod(0o600)
    write("domain/src/lib.rs", "pub fn charge() -> Result<u32, &'static str> { Ok(2) }\n")
    write("api/src/lib.rs", "use payment_domain::charge;\npub fn handle() -> Result<u32, &'static str> { charge() }\nfn private_helper() -> u32 { 1 }\n")
    proposal = commit("Change public payment contract")
    git("checkout", "--detach", baseline)
    write("api/src/lib.rs", "use payment_domain::charge;\npub fn handle() -> u32 { charge() }\nfn private_helper() -> u32 { 2 }\n")
    helper = commit("Private helper change only")
    git("checkout", "-b", "observed-merge", baseline)
    write("README.md", "Synthetic observed target branch, no deployment.\n")
    write("docs/architecture.md", "# Payment design intent\n\nIntent: review the new Result contract before reuse.\nThis is unverified design intent, not runtime proof.\n")
    commit("Target branch documentation changed")
    git("-c", "user.name=HLD fixture", "-c", "user.email=fixture@example.invalid", "merge", "--no-ff", "--no-gpg-sign", "-qm", "Actual merge candidate", proposal)
    merged = git("rev-parse", "HEAD")
    assert merged != proposal
    reviews = {}
    for number, head in [(99, proposal), (100, helper), (101, merged), (102, proposal)]:
        artifact, html = output / f"pr-{number}.json", output / f"pr-{number}.html"
        run([str(PROJECT / "target/debug/flowdiff"), "review", "--repo", str(repo), "--github-repository", slug,
             "--onboarding-context", str(context_path), "--disable-lld", "--base", baseline, "--head", head,
             "--cache-dir", str(root / "cache"), "--output", str(html), "--artifact", str(artifact)] + (["--internal-artifact"] if number != 99 else []) + (["--disable-hld"] if number == 102 else []) +
            (["--pr-number", "99", "--dashboard-origin", "https://fixture.example.com", "--github-check-draft", str(root / "check-draft.json")] if number == 99 else []))
        artifact.chmod(0o600)
        html.chmod(0o600)
        value = json.loads(artifact.read_text())
        assert value["lld_status"] == "unavailable"
        if number == 102:
            assert not value.get("hld") and value["hld_status"] == "unavailable"
            reviews[str(number)] = {"review_id": f"pr-{number}-{value['review_id']}", "head": head, "hld_outcome": value["hld_status"]}
            continue
        assert value["hld"]
        assert all("PRIVATE_" not in json.dumps(value[key]) for key in ("hld", "onboarding_context"))
        reviews[str(number)] = {"review_id": f"pr-{number}-{value['review_id']}", "head": head, "hld_outcome": value["hld_status"],
                               "changes": len(value["hld"]["delta"]["changes"]), "baseline_design": value["hld"]["baseline"]["id"], "proposal_design": value["hld"]["proposal"]["id"]}
    assert reviews["99"]["hld_outcome"] == "change_detected", reviews
    assert reviews["100"]["hld_outcome"] == "incomplete" and reviews["100"]["changes"] == 0, reviews
    assert reviews["101"]["hld_outcome"] == "change_detected", reviews
    assert reviews["99"]["review_id"].split("-", 2)[2] != reviews["102"]["review_id"].split("-", 2)[2]
    # Static code facts stay equal, but unverified hosting/route context prevents
    # an unconditional no-change outcome. Pure-Rust coverage is tested separately.
    metadata = {"root": str(root), "repo": str(repo), "output": str(output), "bin": str(commands), "baseline": baseline,
                "proposal": proposal, "helper": helper, "merged": merged, "slug": slug, "reviews": reviews, "context_session": session_id}
    (root / "fixture.json").write_text(json.dumps(metadata))
    stub = '''#!/usr/bin/env python3
import json,sys
from pathlib import Path
f=json.loads(Path(FIXTURE).read_text());a=sys.argv[1:]
if a[:2]==["auth","status"]: print("fixture authenticated")
elif a[:2]==["api","user"]: print("fixture-user")
elif a[:2]==["api","repos/"+f["slug"]]: print(json.dumps({"pull":True,"push":True}))
elif len(a)>=2 and a[0]=="api" and a[1].startswith("repos/"+f["slug"]+"/pulls/"):
 n=a[1].rsplit("/",1)[1];print(json.dumps({"base":{"sha":f["baseline"]},"head":{"sha":f["reviews"][n]["head"]}}))
elif a[:2]==["pr","list"]:
 print(json.dumps([{"number":int(n),"title":"HLD fixture "+n,"state":"OPEN","isDraft":False,"updatedAt":"2026-10-12T00:00:00Z","headRefName":"fixture","baseRefName":"main","url":"https://github.com/fixture/hld-review/pull/"+n} for n in f["reviews"]]))
else: sys.exit(1)
'''
    (commands / "gh").write_text(stub.replace("FIXTURE", repr(str(root / "fixture.json"))))
    (commands / "gh").chmod(0o700)
    return metadata


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    print(json.dumps(build(args.output), indent=2))
