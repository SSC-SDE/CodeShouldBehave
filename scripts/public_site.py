#!/usr/bin/env python3
"""Build/verify a bounded Pages site from explicit public bundles, never private IR.

This command is local-only. It makes no network request or GitHub write. Existing
sites require an independently trusted manifest hash before they can be restored.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
SITE_SCHEMA = "csb-public-site-v1"
INDEX_SCHEMA = "csb-public-index-v1"
MAX_SITE_BYTES = 64 * 1024 * 1024
MAX_REPORT_BYTES = 8 * 1024 * 1024
MAX_FILES = 20_000
MAX_SITE_MANIFEST_BYTES = 4 * 1024 * 1024
MAX_REPORTS = 100
HEX = re.compile(r"[a-f0-9]{64}\Z")


class SiteError(ValueError):
    """Invalid/untrusted/incomplete input: do not deploy it."""


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def parse_json(data):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise SiteError("Duplicate JSON key")
            result[key] = value
        return result
    try:
        return json.loads(data, object_pairs_hook=unique, parse_constant=lambda _: (_ for _ in ()).throw(SiteError("Non-finite JSON")))
    except (UnicodeError, json.JSONDecodeError) as error:
        raise SiteError("Invalid UTF-8 JSON") from error


def relative_path(value):
    if not isinstance(value, str) or not value or len(value) > 512:
        raise SiteError("Invalid manifest path")
    parts = PurePosixPath(value).parts
    if value.startswith("/") or any(c in value for c in "\\:%?#") or any(ord(c) < 32 for c in value):
        raise SiteError("Unsafe manifest path")
    if any(part in ("", ".", "..", ".git") for part in value.split("/")) or str(PurePosixPath(value)) != value or not parts:
        raise SiteError("Unsafe manifest path")
    return value


def read_file(path, limit):
    path = Path(path)
    if path.is_symlink():
        raise SiteError("Symlink input rejected")
    try:
        descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        with os.fdopen(descriptor, "rb") as stream:
            info = os.fstat(stream.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_size > limit:
                raise SiteError("Input is not a bounded regular file")
            data = stream.read(limit + 1)
    except OSError as error:
        raise SiteError("Input file unavailable") from error
    if len(data) > limit:
        raise SiteError("Input byte limit exceeded")
    return data


def tree_files(root):
    root = Path(root)
    if root.is_symlink() or not root.is_dir():
        raise SiteError("Input tree must be a real directory")
    result = []
    for directory, directories, files in os.walk(root, followlinks=False):
        if any((Path(directory) / name).is_symlink() for name in directories):
            raise SiteError("Symlink directory rejected")
        for name in files:
            path = Path(directory) / name
            if path.is_symlink():
                raise SiteError("Symlink file rejected")
            result.append(relative_path(path.relative_to(root).as_posix()))
            if len(result) > MAX_FILES:
                raise SiteError("Site file cap exceeded")
    return sorted(result)


def utc(value):
    if not isinstance(value, str) or not value.endswith("Z"):
        raise SiteError("Explicit UTC observation timestamp required")
    try:
        parsed = datetime.fromisoformat(value[:-1] + "+00:00")
    except ValueError as error:
        raise SiteError("Invalid UTC observation timestamp") from error
    if parsed.tzinfo != timezone.utc:
        raise SiteError("UTC timestamp required")
    return value


def bundle(root):
    root = Path(root)
    data = read_file(root / "manifest.json", MAX_REPORT_BYTES)
    manifest = parse_json(data)
    required = {"schema_version", "review_id", "public_report_id", "analysis_review_id", "repository", "pr_number", "selected_baseline", "baseline", "proposal", "scope_version", "policy_digest", "engine", "files", "routes", "coverage", "capabilities"}
    if not isinstance(manifest, dict) or set(manifest) != required or manifest["schema_version"] != "csb-public-report-v1":
        raise SiteError("Unsupported public manifest")
    identity = manifest["public_report_id"]
    without_ids = {key: value for key, value in manifest.items() if key not in ("review_id", "public_report_id")}
    if not isinstance(identity, str) or not HEX.fullmatch(identity) or manifest["review_id"] != identity or digest(canonical(without_ids)) != identity:
        raise SiteError("Public report identity mismatch")
    if not isinstance(manifest["files"], list) or len(manifest["files"]) > 1000:
        raise SiteError("Public report file cap exceeded")
    inventory, total, previous = {"manifest.json"}, len(data), ""
    for item in manifest["files"]:
        if not isinstance(item, dict) or set(item) != {"path", "sha256", "bytes", "kind"}:
            raise SiteError("Invalid public file table")
        path = relative_path(item["path"])
        if path <= previous or path in inventory or Path(path).suffix not in (".json", ".md"):
            raise SiteError("Public file table must be sorted, unique inert data")
        previous = path
        if type(item["bytes"]) is not int or not 0 <= item["bytes"] <= MAX_REPORT_BYTES:
            raise SiteError("Invalid public file length")
        value = read_file(root / path, item["bytes"])
        if len(value) != item["bytes"] or digest(value) != item["sha256"]:
            raise SiteError("Public chunk hash/length mismatch")
        total += len(value)
        if total > MAX_REPORT_BYTES:
            raise SiteError("Public report byte cap exceeded")
        inventory.add(path)
    if set(tree_files(root)) != inventory:
        raise SiteError("Unexpected or missing public report file")
    return manifest, data


def verify_producer(root, verifier, verifier_sha256):
    """Require the pinned Rust semantic verifier, not only self-consistent hashes.

    This is still not provider authorization: the CI caller separately checks
    trusted producer/run, requested policy and current public eligibility.
    """
    if verifier is None or not isinstance(verifier_sha256, str) or not HEX.fullmatch(verifier_sha256):
        raise SiteError("Pinned Rust semantic verifier required before admission")
    verifier = Path(verifier)
    if digest(read_file(verifier, 64 * 1024 * 1024)) != verifier_sha256:
        raise SiteError("Semantic verifier digest mismatch")
    manifest, _ = bundle(root)
    try:
        result = subprocess.run([str(verifier.resolve()), "verify", "--bundle", str(Path(root).resolve())],
                                capture_output=True, timeout=30, check=False,
                                env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8"})
    except (OSError, subprocess.SubprocessError) as error:
        raise SiteError("Semantic verifier unavailable") from error
    if result.returncode or len(result.stdout) > MAX_REPORT_BYTES or parse_json(result.stdout) != manifest:
        raise SiteError("Public semantic verification failed")
    return manifest


def verify_site(root, expected_sha256=None):
    root = Path(root)
    data = read_file(root / "site-manifest.json", MAX_SITE_MANIFEST_BYTES)
    if expected_sha256 is not None and (not HEX.fullmatch(expected_sha256) or digest(data) != expected_sha256):
        raise SiteError("Site receipt hash mismatch")
    manifest = parse_json(data)
    if not isinstance(manifest, dict) or set(manifest) != {"schema_version", "observed_at", "files"} or manifest["schema_version"] != SITE_SCHEMA:
        raise SiteError("Unsupported site manifest")
    utc(manifest["observed_at"])
    if not isinstance(manifest["files"], list) or len(manifest["files"]) > MAX_FILES:
        raise SiteError("Site inventory cap exceeded")
    inventory, total, previous = {"site-manifest.json"}, len(data), ""
    for item in manifest["files"]:
        if not isinstance(item, dict) or set(item) != {"path", "bytes", "sha256"}:
            raise SiteError("Invalid site inventory")
        path = relative_path(item["path"])
        if path <= previous or path in inventory or type(item["bytes"]) is not int or item["bytes"] < 0:
            raise SiteError("Invalid site inventory order/length")
        previous = path
        value = read_file(root / path, min(item["bytes"], MAX_SITE_BYTES))
        if len(value) != item["bytes"] or digest(value) != item["sha256"]:
            raise SiteError("Site file hash/length mismatch")
        total += len(value)
        if total > MAX_SITE_BYTES:
            raise SiteError("Site byte cap exceeded")
        inventory.add(path)
    if set(tree_files(root)) != inventory:
        raise SiteError("Unexpected/missing site file")
    return {"site_sha256": digest(data), "bytes": total, "index_sha256": digest(read_file(root / "index.json", MAX_SITE_BYTES))}


def copy_public_bundle(source, destination):
    manifest, data = bundle(source)
    destination.mkdir(parents=True, exist_ok=False)
    (destination / "manifest.json").write_bytes(data)
    for item in manifest["files"]:
        path = destination / item["path"]
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(read_file(Path(source) / item["path"], item["bytes"]))
    return manifest, data


def index_entry(manifest, data):
    return {"public_report_id": manifest["public_report_id"],
            "analysis_review_id": manifest["analysis_review_id"], "manifest": f"reviews/{manifest['public_report_id']}/manifest.json",
            "manifest_sha256": digest(data), "manifest_bytes": len(data), "repository": manifest["repository"], "pr_number": manifest["pr_number"],
            "selected_baseline": manifest["selected_baseline"], "baseline": manifest["baseline"], "proposal": manifest["proposal"],
            "scope_version": manifest["scope_version"]}


def assemble_site(bundles, output, observed_at, previous=None, previous_sha256=None, withdrawn=(), project_root=ROOT, observations=(), verifier=None, verifier_sha256=None):
    """Create a new local site. Never overwrite an output or trust an unsigned restore."""
    observed_at = utc(observed_at)
    output, project_root = Path(output), Path(project_root)
    if output.exists() or output.is_symlink() or not output.parent.is_dir() or output.parent.is_symlink():
        raise SiteError("Output must be a new directory under an existing real parent")
    withdrawn = set(withdrawn)
    if len(withdrawn) > 10_000 or any(not isinstance(item, str) or not HEX.fullmatch(item) for item in withdrawn):
        raise SiteError("Invalid withdrawal ledger")
    if (previous is None) != (previous_sha256 is None):
        raise SiteError("Restore requires the independent trusted site hash")
    metadata = {}
    for observation in observations:
        required = {"repository_id", "pr_number", "state", "head", "base", "observed_at"}
        if not isinstance(observation, dict) or set(observation) != required:
            raise SiteError("Invalid PR observation")
        if any(type(observation[key]) is not int or observation[key] < 1 for key in ("repository_id", "pr_number")) or observation["state"] not in ("open", "draft", "closed", "merged", "unknown"):
            raise SiteError("Invalid PR observation identity/state")
        for key in ("head", "base"):
            if not isinstance(observation[key], str) or not re.fullmatch(r"(?:[a-f0-9]{40}|[a-f0-9]{64})", observation[key]):
                raise SiteError("Invalid PR observed commit")
        utc(observation["observed_at"])
        key = (observation["repository_id"], observation["pr_number"])
        if key in metadata:
            raise SiteError("Duplicate PR observation")
        metadata[key] = observation
        if len(metadata) > 100:
            raise SiteError("PR observation cap exceeded")
    sources, expired, previous_metadata = {}, {}, {}
    if previous is not None:
        previous = Path(previous)
        verify_site(previous, previous_sha256)
        old = parse_json(read_file(previous / "index.json", MAX_SITE_BYTES))
        if old.get("schema_version") != INDEX_SCHEMA or not isinstance(old.get("reports"), list):
            raise SiteError("Invalid previous index")
        for record in old["reports"]:
            identity = record.get("public_report_id")
            if not isinstance(identity, str) or not HEX.fullmatch(identity):
                raise SiteError("Invalid previous report identity")
            sources[identity] = previous / "reviews" / identity
            previous_metadata[identity] = {key: record[key] for key in ("state", "historical", "observed_at", "published_at") if key in record}
        for path in tree_files(previous):
            if path.startswith("expired/"):
                identity = Path(path).stem
                if not HEX.fullmatch(identity):
                    raise SiteError("Invalid expiry record")
                expired[identity] = parse_json(read_file(previous / path, 4096))
    for source in bundles:
        manifest, data = bundle(source)
        verify_producer(source, verifier, verifier_sha256)
        identity = manifest["public_report_id"]
        if identity in sources:
            _, old_data = bundle(sources[identity])
            if old_data != data:
                raise SiteError("Immutable report ID collision")
        sources[identity] = Path(source)
    for identity in withdrawn:
        sources.pop(identity, None)
        expired[identity] = {"schema_version": "csb-public-expiry-v1", "public_report_id": identity, "reason": "withdrawn", "observed_at": observed_at}
    # Withdrawals recovered from a newer authoritative ledger always win; they
    # must also be supplied when restoring a pre-withdrawal historical snapshot.
    for identity, record in expired.items():
        if record.get("reason") == "withdrawn":
            sources.pop(identity, None)
    if len(sources) > MAX_REPORTS:
        raise SiteError("Protected reports exceed admission cap; defer instead of deleting")
    temporary = Path(tempfile.mkdtemp(prefix=".csb-site-", dir=output.parent))
    try:
        (temporary / "workspace").mkdir()
        (temporary / "assets").mkdir()
        (temporary / "reviews").mkdir()
        html = read_file(project_root / "src/workspace.html", 512 * 1024).decode()
        html = html.replace('href="/assets/', 'href="../assets/').replace('src="/assets/', 'src="../assets/')
        html = html.replace('href="/workspace"', 'href="./"')
        marker = '<meta name="csb-public-static" content="../index.json">'
        html = html.replace('<meta name="color-scheme" content="dark">', '<meta name="color-scheme" content="dark">\n  ' + marker)
        html = html.replace('<script src="../assets/workspace.js"></script>', '<script src="../assets/public-workspace.js"></script>\n  <script src="../assets/workspace.js"></script>')
        if marker not in html or 'src="../assets/public-workspace.js"' not in html:
            raise SiteError("Approved workspace static hooks unavailable")
        (temporary / "workspace/index.html").write_text(html)
        for name, source in {"workspace.js": "src/workspace.js", "workspace.css": "src/workspace.css", "public-workspace.js": "src/public-workspace.js", "mermaid.js": "docs/prototypes/vendor/mermaid.tiny-12.1.0.js", "mermaid-LICENSE.txt": "docs/prototypes/vendor/mermaid-LICENSE.txt"}.items():
            (temporary / "assets" / name).write_bytes(read_file(project_root / source, 4 * 1024 * 1024))
        for name in ("LICENSE", "THIRD_PARTY_NOTICES.md"):
            (temporary / name).write_bytes(read_file(project_root / name, 2 * 1024 * 1024))
        for group in ("javascript", "native"):
            license_root = project_root / "licenses" / group
            if license_root.is_dir():
                for path in tree_files(license_root):
                    target = temporary / "licenses" / group / path
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(read_file(license_root / path, 256 * 1024))
        # Preserve the renderer's existing source notice path for the root
        # notice document, as well as its copy beside the runtime asset.
        notice = temporary / "docs/prototypes/vendor/mermaid-LICENSE.txt"
        notice.parent.mkdir(parents=True)
        notice.write_bytes(read_file(project_root / "docs/prototypes/vendor/mermaid-LICENSE.txt", 64 * 1024))
        (temporary / ".nojekyll").write_text("")
        (temporary / "index.html").write_text('<!doctype html><meta charset="utf-8"><meta http-equiv="refresh" content="0;url=workspace/"><title>CodeShouldBehave</title><a href="workspace/">Open review workspace</a>\n')
        (temporary / "404.html").write_text('<!doctype html><meta charset="utf-8"><title>Report unavailable · CodeShouldBehave</title><h1>Report unavailable</h1><p>This path is not a published report. Open the workspace from its exact Architecture check link.</p>\n')
        reports = []
        for identity, source in sorted(sources.items()):
            verify_producer(source, verifier, verifier_sha256)
            manifest, data = copy_public_bundle(source, temporary / "reviews" / identity)
            record = index_entry(manifest, data)
            record.update(previous_metadata.get(identity, {}))
            observation = metadata.get((manifest["repository"]["id"], manifest["pr_number"]))
            if observation is not None:
                record.update(state=observation["state"], observed_at=observation["observed_at"],
                              historical=manifest["proposal"] != observation["head"] or manifest["selected_baseline"] != observation["base"])
            reports.append(record)
        index = {"schema_version": INDEX_SCHEMA, "generated_at": observed_at, "reports": reports,
                 "expired": [{"public_report_id": identity, "path": f"expired/{identity}.json"} for identity in sorted(expired)]}
        (temporary / "index.json").write_bytes(canonical(index))
        for identity, record in sorted(expired.items()):
            destination = temporary / "expired" / (identity + ".json")
            destination.parent.mkdir(exist_ok=True)
            destination.write_bytes(canonical(record))
        files, total = [], 0
        for path in tree_files(temporary):
            data = read_file(temporary / path, MAX_SITE_BYTES)
            total += len(data)
            if total > MAX_SITE_BYTES:
                raise SiteError("Site cap exceeded; preserve previous generation")
            files.append({"path": path, "bytes": len(data), "sha256": digest(data)})
        (temporary / "site-manifest.json").write_bytes(canonical({"schema_version": SITE_SCHEMA, "observed_at": observed_at, "files": files}))
        result = verify_site(temporary)
        result["report_count"] = len(reports)
        if output.exists() or output.is_symlink():
            raise SiteError("Output appeared while staging; refusing overwrite")
        temporary.rename(output)
        return result
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    build = commands.add_parser("build")
    build.add_argument("--bundle", action="append", default=[])
    build.add_argument("--output", required=True)
    build.add_argument("--observed-at", required=True)
    build.add_argument("--previous")
    build.add_argument("--previous-sha256")
    build.add_argument("--withdraw", action="append", default=[])
    build.add_argument("--observations", type=Path)
    build.add_argument("--verifier", type=Path, required=True)
    build.add_argument("--verifier-sha256", required=True)
    verify = commands.add_parser("verify")
    verify.add_argument("--site", required=True)
    verify.add_argument("--expected-sha256")
    args = parser.parse_args()
    try:
        observations = parse_json(read_file(args.observations, 256 * 1024)) if args.command == "build" and args.observations else []
        if not isinstance(observations, list):
            raise SiteError("PR observations must be a bounded array")
        result = assemble_site(args.bundle, args.output, args.observed_at, args.previous, args.previous_sha256, args.withdraw, observations=observations, verifier=args.verifier, verifier_sha256=args.verifier_sha256) if args.command == "build" else verify_site(args.site, args.expected_sha256)
    except (SiteError, KeyError, TypeError, ValueError, OSError) as error:
        parser.exit(1, f"Public site unavailable: {error}\n")
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
