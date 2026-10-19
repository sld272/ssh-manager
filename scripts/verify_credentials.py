#!/usr/bin/env python3
"""Check native credentials and reuse them from a differently signed CLI.
Fixture secrets are disposable; real host credentials remain inside the broker.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import uuid

p = argparse.ArgumentParser()
p.add_argument('--binary', required=True, type=Path)
p.add_argument('--host', default='vm')
p.add_argument('--report', default=Path('.local/verification/credentials.json'), type=Path)
args = p.parse_args()
binary = args.binary.resolve()
helper = binary.with_name('ssh-manager-credentials')
reference = uuid.uuid4().hex
fixture_secret = 'ssh-manager-disposable-credential-fixture'
checks = []
start = time.monotonic()


def native(action, input=None, selected=reference):
    env = os.environ.copy()
    env['SSH_MANAGER_CREDENTIAL_HELPER'] = selected
    env['SSH_MANAGER_CREDENTIAL_ACTION'] = action
    return subprocess.run([str(helper)], input=input, text=True, capture_output=True,
                          env=env, timeout=6)


def check(name, condition):
    assert condition, name
    checks.append({'case': name, 'passed': True})
    print('PASS ' + name, flush=True)


def connect(exe):
    result = subprocess.run([str(exe), '--json', 'check', args.host],
                            capture_output=True, text=True, timeout=20)
    value = json.loads(result.stdout)
    assert value.get('status') == 'succeeded', value


helper_hash = hashlib.sha256(helper.read_bytes()).hexdigest()
helper_mtime = helper.stat().st_mtime_ns
try:
    check('native_save_without_dialog', native('set', fixture_secret).returncode == 0)
    for _ in range(5):
        result = native('get')
        assert result.returncode == 0 and result.stdout == fixture_secret
    check('native_repeated_reads', True)
    result = native('get', selected=uuid.uuid4().hex)
    check('missing_credential_is_explicit_error', result.returncode != 0
          and json.loads(result.stderr)['code'] == 'credential_unavailable')
    # A different ad-hoc CLI identity reproduces a development rebuild without changing the credential helper.
    renamed_cli = binary.parent / ('.ssh-manager-verify-' + uuid.uuid4().hex)
    try:
        shutil.copy2(binary, renamed_cli)
        subprocess.run(['codesign', '--force', '--sign', '-', '--identifier',
                        'dev.ssh-manager.caller-verification', str(renamed_cli)],
                       check=True, capture_output=True)
        for _ in range(3):
            connect(renamed_cli)
        check('changed_cli_identity_reuses_credentials', True)
    finally:
        renamed_cli.unlink(missing_ok=True)
    connect(binary)
    check('installed_cli_still_connects', True)
    check('credential_helper_was_not_replaced', helper_hash == hashlib.sha256(helper.read_bytes()).hexdigest()
          and helper_mtime == helper.stat().st_mtime_ns)
finally:
    result = native('delete')
    assert result.returncode == 0, 'fixture credential cleanup failed'
check('fixture_credential_removed', native('get').returncode != 0)
args.report.parent.mkdir(parents=True, exist_ok=True)
args.report.write_text(json.dumps({'elapsed_seconds': round(time.monotonic()-start, 2),
                                 'cases': checks}, ensure_ascii=False, indent=2) + '\n')
print(f'{len(checks)} checks passed; report: {args.report}')
