#!/usr/bin/env python3
"""Prepare an auditable local source/runtime release; never upload or publish.

Requires Python 3.11+, Git and an already populated Cargo registry cache. Source
comes from an exact committed tree. Private history, review state, screenshots
and operator evidence are deliberately outside this distribution boundary.
"""
import argparse
from datetime import datetime, timezone
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
try:
    import tomllib
except ImportError:
    raise SystemExit("Release packaging requires Python 3.11+ (tomllib); no package was created.")

ROOT = Path(__file__).resolve().parents[1]
MAX_PACKAGE_BYTES = 128 * 1024 * 1024
PREFIXES = ("src/", "scripts/", "tests/", "templates/", "config/", "examples/", "licenses/", "migrations/", "fixtures/")
EXACT = {".gitignore", "Cargo.toml", "Cargo.lock", "LICENSE", "THIRD_PARTY_NOTICES.md", "CONTRIBUTING.md", "SECURITY.md", "CHANGELOG.md", "docs/PUBLIC_MVP_USAGE.md", "docs/prototypes/vendor/mermaid.tiny-12.1.0.js", "docs/prototypes/vendor/mermaid-LICENSE.txt"}
SECRET_RULES = {
    "private-key": re.compile(rb"-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----"),
    "github-token": re.compile(rb"(?<![A-Za-z0-9])(?:gh[pousr]_[A-Za-z0-9]{36,255}|github_pat_[A-Za-z0-9_]{60,255})(?![A-Za-z0-9])"),
    "aws-access-key": re.compile(rb"(?<![A-Z0-9])(?:AKIA|ASIA)[A-Z0-9]{16}(?![A-Z0-9])"),
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def encode(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def git(*args):
    result = subprocess.run(["git", "-c", "core.hooksPath=" + os.devnull, "-C", str(ROOT), *args], check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
    return result.stdout


def distribution_files(revision):
    if not re.fullmatch(r"[a-f0-9]{40}", revision):
        raise ValueError("Exact SHA-1 source commit required")
    result = {}
    for record in git("ls-tree", "-r", "-z", revision).split(b"\0"):
        if not record:
            continue
        metadata, raw_path = record.split(b"\t", 1)
        mode, kind, object_id = metadata.decode().split()
        path = raw_path.decode("utf-8")
        if path not in EXACT and not path.startswith(PREFIXES):
            continue
        if kind != "blob" or mode not in ("100644", "100755") or any(part in ("", ".", "..") for part in path.split("/")):
            raise ValueError("Unsafe distribution entry")
        content = git("cat-file", "blob", object_id)
        if len(content) > 8 * 1024 * 1024:
            raise ValueError("Distribution source file cap exceeded")
        result[path] = (content, int(mode[-3:], 8))
    required = {"Cargo.toml", "Cargo.lock", "LICENSE", "THIRD_PARTY_NOTICES.md", "CONTRIBUTING.md", "SECURITY.md", "docs/PUBLIC_MVP_USAGE.md", "src/workspace.html", "src/workspace.js", "src/workspace-preferences.js", "src/workspace-registry.json", "src/fonts/inter.ttf", "src/fonts/jetbrains-mono.ttf", "src/fonts/inter-OFL.txt", "src/fonts/jetbrains-mono-OFL.txt", "src/public-workspace.js", "scripts/public_ci.py", "scripts/public_site.py", "migrations/0001_foundation.sql", "migrations/0013_lld_defaults.sql", "migrations/0014_workspace_preferences.sql", "src/workspace_preferences.rs", "fixtures/demo/checkout.before.rs", "fixtures/demo/checkout.after.rs", "licenses/native/sources.json", "licenses/javascript/sources.json"}
    missing = required - result.keys()
    if missing:
        raise ValueError("Release source is incomplete: " + ", ".join(sorted(missing)))
    result["README.md"] = result["docs/PUBLIC_MVP_USAGE.md"]
    if sum(len(value[0]) for value in result.values()) > MAX_PACKAGE_BYTES:
        raise ValueError("Distribution source byte cap exceeded")
    return result


def license_inventory(lock, registry):
    records, texts = [], {}
    for package in tomllib.loads(lock.decode())["package"]:
        if not package.get("source", "").startswith("registry+"):
            continue
        name, version = package["name"], package["version"]
        if not re.fullmatch(r"[A-Za-z0-9_.+-]+", name + version):
            raise ValueError("Unsafe lockfile package identity")
        matches = list((registry.parent / "cache").glob(f"*/{name}-{version}.crate"))
        if len(matches) != 1 or matches[0].is_symlink() or matches[0].stat().st_size > 16 * 1024 * 1024:
            raise ValueError(f"Exact cached crate missing/ambiguous: {name}@{version}; run cargo fetch --locked")
        archive_bytes = matches[0].read_bytes()
        if sha(archive_bytes) != package["checksum"]:
            raise ValueError("Cached crate archive differs from Cargo.lock checksum")
        # Read original notices directly from the checksum-verified registry
        # archive, never assume the extracted working cache is unchanged.
        with gzip.GzipFile(fileobj=io.BytesIO(archive_bytes)) as decompressor:
            expanded = decompressor.read(128 * 1024 * 1024 + 1)
        if len(expanded) > 128 * 1024 * 1024:
            raise ValueError("Crate expansion limit exceeded")
        names = []
        with tarfile.open(fileobj=io.BytesIO(expanded), mode="r:") as source:
            members = source.getmembers()
            if len(members) > 20_000:
                raise ValueError("Crate entry cap exceeded")
            metadata_file = source.extractfile(f"{name}-{version}/Cargo.toml")
            if metadata_file is None:
                raise ValueError("Registry Cargo manifest missing")
            metadata = tomllib.loads(metadata_file.read(1024 * 1024).decode())["package"]
            if metadata["name"] != name or metadata["version"] != version:
                raise ValueError("Cached package identity mismatch")
            license_expression = metadata.get("license")
            if not isinstance(license_expression, str) or not license_expression:
                raise ValueError(f"Unresolved package license: {name}@{version}")
            for member in members:
                prefix = f"{name}-{version}/"
                if not member.name.startswith(prefix) or not member.isfile():
                    continue
                relative = member.name[len(prefix):]
                if any(part in ("", ".", "..") for part in relative.split("/")) or "\\" in relative:
                    raise ValueError("Unsafe crate member")
                if not re.match(r"^(?:LICENSE|COPYING|NOTICE|COPYRIGHT)(?:[._-].*)?$", Path(relative).name, re.I):
                    continue
                if member.size > 256 * 1024:
                    raise ValueError("Dependency notice byte cap exceeded")
                text_file = source.extractfile(member)
                if text_file is None:
                    raise ValueError("Dependency notice unavailable")
                data = text_file.read(256 * 1024 + 1)
                target = f"licenses/rust/{name}-{version}/{relative}"
                if target in texts:
                    raise ValueError("Duplicate crate notice")
                texts[target] = (data, 0o644)
                names.append({"path": target, "sha256": sha(data)})
        if not names:
            raise ValueError(f"No original license/notice text available: {name}@{version}")
        records.append({"name": name, "version": version, "purl": f"pkg:cargo/{name}@{version}", "declared_license": license_expression, "registry_checksum_sha256": package["checksum"], "notices": names})
    return records, texts


def verify_notice_sources(files, packages):
    """Retain upstream notices omitted from registry license-file inventories."""
    expected = {item["name"]: item for item in packages}
    covered = set()
    for manifest_path in ("licenses/javascript/sources.json", "licenses/native/sources.json"):
        records = json.loads(files[manifest_path][0])
        if not isinstance(records, list) or len(records) > 100:
            raise ValueError("Invalid supplemental notice inventory")
        for record in records:
            path = record["path"]
            if not isinstance(path, str) or not path.startswith("licenses/") or path not in files:
                raise ValueError("Supplemental notice missing")
            data = files[path][0]
            if sha(data) != record["sha256"] or len(data) != record["bytes"]:
                raise ValueError("Supplemental notice hash/length differs")
            if "package" in record:
                package = expected.get(record["package"])
                if not package or package["version"] != record["package_version"] or package["registry_checksum_sha256"] != record["registry_checksum_sha256"]:
                    raise ValueError("Supplemental notice does not match the locked dependency")
                covered.add(record["package"])
    if ({"tree-sitter", "libsqlite3-sys"} & expected.keys()) - covered:
        raise ValueError("Original Tree-sitter/SQLite supplemental notice is missing")


def scan(files):
    findings = []
    for path, (data, _) in sorted(files.items()):
        for rule, expression in SECRET_RULES.items():
            for match in expression.finditer(data):
                findings.append({"path": path, "line": data[:match.start()].count(b"\n") + 1, "rule": rule, "fingerprint": sha(match.group())})
    return findings


def archive(path, files):
    with path.open("xb") as output:
        with gzip.GzipFile(filename="", mode="wb", fileobj=output, mtime=0) as compressor:
            with tarfile.open(fileobj=compressor, mode="w", format=tarfile.USTAR_FORMAT) as tar:
                for name, (data, mode) in sorted(files.items()):
                    info = tarfile.TarInfo(name)
                    info.size, info.mode, info.mtime = len(data), mode, 0
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    tar.addfile(info, io.BytesIO(data))


def build_native(files, source_commit, directory, requested_target=None):
    """Build our pinned analyzer source in a clean tree; never relabel input bins."""
    source = directory / "native-source"
    source.mkdir()
    for name, (data, mode) in files.items():
        path = source / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        path.chmod(mode)
    environment = {key: value for key, value in os.environ.items() if key in ("PATH", "HOME", "CARGO_HOME", "RUSTUP_HOME", "TMPDIR", "SDKROOT", "DEVELOPER_DIR")}
    environment["LANG"] = "C.UTF-8"
    compiler = subprocess.run(["rustc", "+1.87.0", "-vV"], check=True, capture_output=True, timeout=30, env=environment).stdout.decode()
    target = next((line[len("host: "):] for line in compiler.splitlines() if line.startswith("host: ")), None)
    if not target or (requested_target and requested_target != target):
        raise ValueError("Runtime packaging supports only the actual compiler host target; build Linux on its trusted runner")
    target_directory = directory / "native-target"
    command = ["cargo", "+1.87.0", "build", "--release", "--locked", "--offline", "--all-features", "--bins", "--target", target, "--target-dir", str(target_directory.resolve())]
    with (directory / "native-build.log").open("wb") as log:
        subprocess.run(command, cwd=source, check=True, stdout=log, stderr=subprocess.STDOUT, timeout=600, env=environment)
    binaries, identities = {}, []
    for name in ("flowdiff", "csb-public-report", "csb-onboarding"):
        path = target_directory / target / "release" / name
        if path.is_symlink() or not path.is_file() or path.stat().st_size > 64 * 1024 * 1024:
            raise ValueError("Built runtime executable missing or oversized")
        data = path.read_bytes()
        binaries["bin/" + name] = (data, 0o755)
        identities.append({"name": name, "sha256": sha(data), "bytes": len(data)})
    record = {"schema_version": "csb-local-native-build-v1", "source_commit": source_commit, "target": target,
              "compiler": compiler, "command": command[:command.index("--target-dir")], "binaries": identities,
              "provenance": "local_clean_source_build; hosted release attestation required before CI activation"}
    return binaries, record


def build(output, revision, build_runtime=False, target=None, registry=None):
    output = Path(output)
    if output.exists() or output.is_symlink() or not output.parent.is_dir():
        raise ValueError("Output must be a new directory")
    files = distribution_files(revision)
    registry = Path(registry) if registry else Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))) / "registry/src"
    packages, notices = license_inventory(files["Cargo.lock"][0], registry)
    files.update(notices)
    verify_notice_sources(files, packages)
    inventory = {"schema_version": "csb-license-inventory-v1", "source_commit": revision, "scope": "all_registry_packages_in_Cargo.lock_including_target_and_test_dependencies", "packages": packages}
    files["licenses/rust/inventory.json"] = (encode(inventory), 0o644)
    # A lockfile SBOM is explicitly broader than the linked executable dependency
    # closure. Do not relabel it a measured target-specific runtime SBOM.
    sbom = {"bomFormat": "CycloneDX", "specVersion": "1.6", "version": 1, "metadata": {"component": {"type": "application", "name": "CodeShouldBehave", "version": revision}, "properties": [{"name": "csb:scope", "value": inventory["scope"]}]}, "components": [{"type": "library", "name": p["name"], "version": p["version"], "purl": p["purl"], "hashes": [{"alg": "SHA-256", "content": p["registry_checksum_sha256"]}], "licenses": [{"expression": p["declared_license"].replace("/", " OR ")}]} for p in packages]}
    files["sbom.cdx.json"] = (encode(sbom), 0o644)
    findings = scan(files)
    if findings:
        # Metadata only: never echo credential bytes. Classify deliberate test
        # patterns in a separately reviewed waiver, not by a hidden path ignore.
        raise ValueError("Potential sensitive strings require review: " + json.dumps(findings, sort_keys=True))
    temporary = Path(tempfile.mkdtemp(prefix=".csb-release-", dir=output.parent))
    try:
        build_record = None
        source_archive = temporary / f"codeshouldbehave-source-{revision[:12]}.tar.gz"
        archive(source_archive, files)
        assets = [{"path": source_archive.name, "sha256": sha(source_archive.read_bytes()), "bytes": source_archive.stat().st_size, "kind": "source"}]
        if build_runtime:
            executables, build_record = build_native(files, revision, temporary, target)
            runtime_names = {"README.md", "LICENSE", "THIRD_PARTY_NOTICES.md", "SECURITY.md", "CHANGELOG.md", "sbom.cdx.json", "src/workspace.html", "src/workspace.css", "src/workspace.js", "src/workspace-preferences.js", "src/workspace-registry.json", "src/fonts/inter.ttf", "src/fonts/jetbrains-mono.ttf", "src/fonts/inter-OFL.txt", "src/fonts/jetbrains-mono-OFL.txt", "src/public-workspace.js", "scripts/public_ci.py", "scripts/public_ci_budget.py", "scripts/public_site.py", "docs/prototypes/vendor/mermaid.tiny-12.1.0.js", "docs/prototypes/vendor/mermaid-LICENSE.txt"}
            runtime = {name: value for name, value in files.items() if name in runtime_names or name.startswith(("licenses/", "templates/public-ci/", "config/"))}
            runtime.update(executables)
            runtime["native-build.json"] = (encode(build_record), 0o644)
            target = build_record["target"]
            if sum(len(data) for data, _ in runtime.values()) > MAX_PACKAGE_BYTES:
                raise ValueError("Runtime package byte cap exceeded")
            runtime_archive = temporary / f"codeshouldbehave-{target}-{revision[:12]}.tar.gz"
            archive(runtime_archive, runtime)
            assets.append({"path": runtime_archive.name, "sha256": sha(runtime_archive.read_bytes()), "bytes": runtime_archive.stat().st_size, "kind": "runtime", "target": target})
            # Only bounded release assets leave staging; compiler caches and
            # the copied build tree are not an undocumented distribution.
            shutil.rmtree(temporary / "native-source")
            shutil.rmtree(temporary / "native-target")
            (temporary / "native-build.log").unlink()
        manifest = {"schema_version": "csb-release-candidate-v1", "source_commit": revision, "assets": assets,
                    "source_files": [{"path": path, "sha256": sha(data), "bytes": len(data)} for path, (data, _) in sorted(files.items())],
                    "excluded": ["Git history/refs", "private review/SQLite/IR state", "docs/evidence", "screenshots/media", "credentials", "operator budget observations"],
                    "license_packages": len(packages), "disclosure_scan": {"rules": sorted(SECRET_RULES), "findings": [], "limitation": "Pattern scan plus explicit distribution boundary; not exhaustive secret detection"},
                    "native_build": build_record,
                    "publication": "not_uploaded_or_authorized_by_packaging"}
        (temporary / "release-manifest.json").write_bytes(encode(manifest))
        (temporary / "SHA256SUMS").write_text("".join(f"{asset['sha256']}  {asset['path']}\n" for asset in assets))
        if output.exists() or output.is_symlink():
            raise ValueError("Output appeared while staging")
        temporary.rename(output)
        return manifest
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--revision", required=True, help="Full committed source SHA")
    parser.add_argument("--build-runtime", action="store_true", help="Build this exact source in an isolated clean tree; never package arbitrary prebuilt executables")
    parser.add_argument("--target", help="Must equal the actual compiler host when building runtime")
    parser.add_argument("--registry", type=Path)
    args = parser.parse_args()
    try:
        result = build(args.output, args.revision, args.build_runtime, args.target, args.registry)
    except (ValueError, OSError, subprocess.SubprocessError, KeyError, tomllib.TOMLDecodeError) as error:
        parser.exit(1, f"Release unavailable: {error}\n")
    print(json.dumps({"source_commit": result["source_commit"], "assets": result["assets"], "license_packages": result["license_packages"]}, sort_keys=True))


if __name__ == "__main__":
    main()
