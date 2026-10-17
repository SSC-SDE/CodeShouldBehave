#!/usr/bin/env python3
"""Fetch one pinned, attested Linux engine package. Never load reviewed code."""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tarfile
import tempfile
import urllib.parse
import urllib.request

CAP = 128 * 1024 * 1024
NATIVE_PREDICATE = "https://github.com/SSC-SDE/codeshouldbehave/attestations/native-build/v1"
HEX = re.compile(r"[0-9a-f]{64}\Z")
URL = re.compile(r"https://github\.com/([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)/releases/download/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+\.tar\.gz\Z")


class Refusal(ValueError):
    pass


class ReleaseRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        target = urllib.parse.urlsplit(new_url)
        if target.scheme != "https" or target.username or target.password or target.hostname not in ("release-assets.githubusercontent.com", "objects.githubusercontent.com", "github.com"):
            raise Refusal("Release asset redirect escaped the approved hosts")
        return super().redirect_request(request, fp, code, message, headers, new_url)


def fetch(config, output, *, opener=None, verifier=subprocess.run):
    engine = config.get("engine", {})
    match = URL.fullmatch(engine.get("package_url", ""))
    sha = engine.get("package_sha256", "")
    commit = engine.get("commit", "")
    signer = engine.get("package_signer_workflow", "")
    if not match or not HEX.fullmatch(sha) or sha == "0" * 64 or not re.fullmatch(r"[0-9a-f]{40}", commit) or commit == "0" * 40:
        raise Refusal("A real fixed release URL, source commit and digest are required")
    upstream = match[1] + "/" + match[2]
    if not re.fullmatch(re.escape(upstream) + r"/\.github/workflows/[A-Za-z0-9_.-]+\.ya?ml", signer):
        raise Refusal("The exact hosted release signer workflow must be pinned")
    attestation_url = engine.get("package_attestation_url", "")
    attestation_sha = engine.get("package_attestation_sha256", "")
    expected_prefix = "https://github.com/" + upstream + "/releases/download/"
    if not attestation_url.startswith(expected_prefix) or not re.fullmatch(r"https://github\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/releases/download/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+\.sigstore\.json", attestation_url) or not HEX.fullmatch(attestation_sha) or attestation_sha == "0" * 64:
        raise Refusal("A digest-pinned release attestation bundle is required")
    output = Path(output)
    if output.exists() or output.is_symlink():
        raise Refusal("Package output must be new")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="csb-release-", dir=output.parent) as directory:
        temporary = Path(directory)
        archive = temporary / "engine.tar.gz"
        client = opener or urllib.request.build_opener(ReleaseRedirect)
        with client.open(engine["package_url"], timeout=30) as response, archive.open("xb") as target:
            count, digest = 0, hashlib.sha256()
            while True:
                chunk = response.read(65536)
                if not chunk:
                    break
                count += len(chunk)
                if count > CAP:
                    raise Refusal("Package download exceeds its byte budget")
                digest.update(chunk); target.write(chunk)
        if digest.hexdigest() != sha:
            raise Refusal("Release archive digest mismatch")
        proof = temporary / "release.sigstore.json"
        with client.open(attestation_url, timeout=15) as response:
            proof_bytes = response.read(256 * 1024 + 1)
        if len(proof_bytes) > 256 * 1024 or hashlib.sha256(proof_bytes).hexdigest() != attestation_sha:
            raise Refusal("Release attestation digest mismatch")
        proof.write_bytes(proof_bytes)
        verified = verifier(["gh", "attestation", "verify", str(archive), "--repo", upstream, "--bundle", str(proof),
                             "--signer-workflow", signer, "--signer-digest", commit,
                             "--source-digest", commit, "--deny-self-hosted-runners", "--predicate-type", NATIVE_PREDICATE, "--format", "json"],
                            check=False, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60)
        if verified.returncode or len(verified.stdout) > 2 * 1024 * 1024:
            raise Refusal("Hosted release provenance verification failed")
        # gh performs cryptographic verification; additionally reject a valid but
        # private-source/self-hosted release that cannot be this public profile.
        try:
            results = json.loads(verified.stdout)
            certificates = [value["verificationResult"]["signature"]["certificate"] for value in results]
        except (KeyError, TypeError, ValueError) as exc:
            raise Refusal("Release verifier returned no certificate") from exc
        if not any(cert.get("sourceRepositoryVisibilityAtSigning") == "public" and cert.get("runnerEnvironment") == "github-hosted" and cert.get("sourceRepositoryDigest") == commit and cert.get("buildSignerDigest") == commit for cert in certificates):
            raise Refusal("Release source visibility or hosted builder differs")
        # Bound decompression before tar header parsing, including PAX headers.
        with gzip.GzipFile(fileobj=io.BytesIO(archive.read_bytes())) as compressed:
            expanded = compressed.read(CAP + 1)
        if len(expanded) > CAP:
            raise Refusal("Release expansion exceeds its byte budget")
        staging = temporary / "package"; staging.mkdir()
        seen, total = set(), 0
        with tarfile.open(fileobj=io.BytesIO(expanded), mode="r:") as source:
            for item in source:
                path = PurePosixPath(item.name)
                if path.is_absolute() or any(part in ("", ".", "..", ".git") for part in item.name.split("/")) or "\\" in item.name or item.name in seen or not item.isfile() or item.pax_headers or item.size < 0:
                    raise Refusal("Unsafe release archive entry")
                seen.add(item.name); total += item.size
                if len(seen) > 20_000 or total > CAP:
                    raise Refusal("Release file inventory exceeds its budget")
                destination = staging / item.name; destination.parent.mkdir(parents=True, exist_ok=True)
                data = source.extractfile(item)
                if data is None:
                    raise Refusal("Release entry missing")
                with destination.open("xb") as target:
                    shutil.copyfileobj(data, target, 65536)
                destination.chmod(0o755 if item.name.startswith("bin/") else 0o644)
        for name, key in (("flowdiff", "flowdiff_sha256"), ("csb-public-report", "exporter_sha256")):
            path = staging / "bin" / name
            if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != engine.get(key) or path.read_bytes()[:4] != b"\x7fELF":
                raise Refusal("Pinned Linux native executable mismatch")
        build = json.loads((staging / "native-build.json").read_bytes())
        if build.get("source_commit") != commit or not re.fullmatch(r"x86_64-unknown-linux-(?:gnu|musl)", build.get("target", "")):
            raise Refusal("Linux build source or target differs")
        for relative in ("scripts/public_ci.py", "scripts/public_ci_budget.py", "scripts/public_site.py", "src/workspace.html", "src/workspace.js", "src/public-workspace.js"):
            if relative not in seen:
                raise Refusal("Release helper or dashboard asset missing")
        staging.rename(output)
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        if args.config.is_symlink() or args.config.stat().st_size > 128 * 1024:
            raise Refusal("Unsafe base-owned configuration")
        fetch(json.loads(args.config.read_bytes()), args.output)
    except (Refusal, OSError, ValueError, subprocess.SubprocessError, tarfile.TarError) as exc:
        # Exception classes only: asset redirects can contain signed query URLs.
        parser.exit(2, "Release bootstrap unavailable: " + type(exc).__name__ + "\n")
    print("Attested pinned Linux engine ready")


if __name__ == "__main__":
    main()
