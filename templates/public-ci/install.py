#!/usr/bin/env python3
"""Prepare a local, reviewable public-ci installation. Never push or publish."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
from datetime import datetime, timezone


def install(config_path, repository_root, package_root):
    package_root, repository_root = Path(package_root).resolve(), Path(repository_root).resolve()
    spec = importlib.util.spec_from_file_location("csb_installer_ci", package_root / "scripts/public_ci.py")
    ci = importlib.util.module_from_spec(spec); spec.loader.exec_module(ci)
    budget = ci.budget_module()
    if sum(budget.LIMITS.values()) != 100:
        raise ci.Refusal("Trusted aggregate REST budget helper is missing or incompatible")
    config = ci.load_config(ci.read_json(Path(config_path), ci.MAX_CONFIG_BYTES))
    if config["operations"] != {"analysis": True, "publication": True, "checks": True}:
        raise ci.Refusal("Installation requires explicit analysis/publication/check activation")
    if not {"package_url", "package_sha256", "package_signer_workflow", "package_attestation_url", "package_attestation_sha256"} <= config["engine"].keys():
        raise ci.Refusal("Attested Linux release pins are required; local candidates cannot activate CI")
    templates = package_root / "templates/public-ci"
    controller = (templates / "controller.yml").read_bytes()
    config["workflow_sha256"] = hashlib.sha256(controller).hexdigest()
    ci.load_config(config, runtime=True)  # Detect all remaining placeholders before creating files.
    policy = ci.read_json(repository_root / config["policy_path"], ci.MAX_CONFIG_BYTES)
    if policy.get("repository") != config["repository"] or policy.get("public") is not True or policy.get("publication_consent") is not True:
        raise ci.Refusal("Base-owned explicit public scope and consent are required")
    for path_key, hash_key in (("policy_path", "policy_sha256"), ("context_path", "context_sha256")):
        path = repository_root / config[path_key]
        if path.is_symlink() or not path.is_file() or path.stat().st_size > 4 * ci.MIB or hashlib.sha256(path.read_bytes()).hexdigest() != config[hash_key]:
            raise ci.Refusal("Base-owned policy/context digest mismatch")
    ci.verify_operator_budget(config, repository_root, datetime.now(timezone.utc))
    # The caller must supply a package already verified by bootstrap. Recheck its
    # exact native source/build identities, not arbitrary executables on PATH.
    build = ci.read_json(package_root / "native-build.json", ci.MAX_CONFIG_BYTES)
    if build.get("source_commit") != config["engine"]["commit"] or build.get("target") not in ("x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl"):
        raise ci.Refusal("Only the pinned hosted Linux runtime can be installed")
    for name, key in (("flowdiff", "flowdiff_sha256"), ("csb-public-report", "exporter_sha256")):
        path = package_root / "bin" / name
        if path.is_symlink() or hashlib.sha256(path.read_bytes()).hexdigest() != config["engine"][key] or path.read_bytes()[:4] != b"\x7fELF":
            raise ci.Refusal("Pinned Linux runtime digest mismatch")
    files = {config["workflow_path"]: controller,
             ".github/workflows/csb-public-nudge.yml": (templates / "nudge.yml").read_bytes(),
             ".codeshouldbehave/bootstrap.py": (templates / "bootstrap.py").read_bytes(),
             ".codeshouldbehave/public-ci.json": ci.canonical(config) + b"\n"}
    for relative in files:
        destination = repository_root / relative
        if destination.exists() or destination.is_symlink() or any(parent.is_symlink() for parent in destination.parents):
            raise ci.Refusal("An installation path already exists or is linked; no file was overwritten")
    # Four create-only files remain normal uncommitted repository changes. The
    # operator can review their exact diff; this command performs no Git writes.
    written = []
    try:
        for relative, data in files.items():
            path = repository_root / relative; path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("xb") as output: output.write(data)
            written.append(path)
    except Exception:
        for path in written: path.unlink()
        raise
    return {"prepared_files": sorted(files), "remote_writes": False, "workflow_sha256": config["workflow_sha256"],
            "next": "Review the diff; separately authorize the public canary/default-branch installation and Pages settings"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--repository-root", required=True, type=Path)
    parser.add_argument("--package-root", required=True, type=Path)
    args = parser.parse_args()
    try:
        result = install(args.config, args.repository_root, args.package_root)
    except Exception as exc:
        parser.exit(2, "Installation not prepared: " + type(exc).__name__ + "\n")
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
