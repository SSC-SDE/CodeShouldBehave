#!/usr/bin/env python3
"""Run a pinned pr-analysis binary without credentials/network. Never enable a repository."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

LIMIT = 16 * 1024 * 1024
OUTPUT_LIMIT = 16 * 1024 * 1024

def command(engine: Path):
    if sys.platform != 'linux' or not shutil.which('bwrap') or not shutil.which('prlimit'):
        raise ValueError('Networkless Linux pr-analysis worker unavailable')
    return ['prlimit', '--as=1073741824', '--cpu=60', '--fsize=67108864', '--nofile=64',
            '--', 'bwrap', '--unshare-all', '--new-session', '--die-with-parent', '--clearenv',
            '--ro-bind', '/usr', '/usr', '--ro-bind-try', '/lib', '/lib',
            '--ro-bind-try', '/lib64', '/lib64', '--proc', '/proc', '--dev', '/dev',
            '--tmpfs', '/tmp', '--dir', '/engine', '--ro-bind', str(engine), '/engine/pr-analysis',
            '--setenv', 'PATH', '/usr/bin:/bin', '--setenv', 'HOME', '/tmp',
            '--setenv', 'GIT_CONFIG_NOSYSTEM', '1', '--setenv', 'GIT_CONFIG_GLOBAL', '/dev/null',
            '--chdir', '/tmp', '/engine/pr-analysis']

def run(engine: Path, expected: str, data: bytes):
    if not re.fullmatch('[a-f0-9]{64}', expected) or len(data) > LIMIT:
        raise ValueError('Invalid pr-analysis input boundary')
    if engine.is_symlink() or not engine.is_file() or engine.stat().st_size > 64 * 1024 * 1024:
        raise ValueError('Invalid pr-analysis engine')
    binary = engine.read_bytes()
    if hashlib.sha256(binary).hexdigest() != expected:
        raise ValueError('PR analysis engine digest mismatch')
    # Bind an immutable copy of the verified bytes, not a replaceable source path.
    with tempfile.TemporaryDirectory(prefix='csb-app-pr-') as folder:
        root = Path(folder)
        pinned = root / 'pr-analysis'
        pinned.write_bytes(binary)
        pinned.chmod(0o500)
        invocation = command(pinned)
        with (root / 'out').open('w+b') as output:
            result = subprocess.run(invocation, input=data, stdout=output, stderr=subprocess.DEVNULL,
                                    timeout=90, env={'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8'}, check=False)
            if result.returncode != 0 or output.tell() > OUTPUT_LIMIT:
                raise ValueError('PR analysis worker failed or exceeded limits')
            output.seek(0)
            body = output.read(OUTPUT_LIMIT + 1)
            json.loads(body)
            return body

def main():
    try:
        if len(sys.argv) != 3:
            raise ValueError('Expected operator engine path and SHA-256')
        data = sys.stdin.buffer.read(LIMIT + 1)
        sys.stdout.buffer.write(run(Path(sys.argv[1]), sys.argv[2], data))
    except (ValueError, OSError, subprocess.SubprocessError):
        print('Isolated pr-analysis unavailable; no report produced', file=sys.stderr)
        return 1
    return 0

if __name__ == '__main__':
    raise SystemExit(main())
