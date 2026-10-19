#!/usr/bin/env python3
"""Exercise a managed Linux VM using an isolated catalog and disposable remote directory.
Credentials remain in the native credential store; this script never reads their values.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import select
import shlex
import shutil
import signal
import sqlite3
import subprocess
import tempfile
import time
import uuid

parser = argparse.ArgumentParser()
parser.add_argument('--binary', type=Path, default=Path('target/debug/ssh-manager'))
parser.add_argument('--host', default='vm')
parser.add_argument('--source-data', type=Path)
parser.add_argument('--report', type=Path, default=Path('.local/verification/vm.json'))
args = parser.parse_args()
binary = str(args.binary.resolve())
started = time.monotonic()
results = []


def invoke(*arguments, data=None):
    command = [binary, '--json']
    if data:
        command += ['--data-dir', str(data)]
    p = subprocess.run(command + list(arguments), capture_output=True, text=True, timeout=95)
    try:
        return json.loads(p.stdout)
    except json.JSONDecodeError:
        raise AssertionError(f'CLI did not return JSON: exit={p.returncode}, stderr={p.stderr[:500]}')


def expect(name, condition, detail=None):
    assert condition, f'{name}: {detail}'
    results.append({'case': name, 'passed': True})
    print(f'PASS {name}', flush=True)


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def rpc(process, request, timeout=10):
    process.stdin.write(json.dumps(request) + '\n')
    process.stdin.flush()
    ready, _, _ = select.select([process.stdout], [], [], timeout)
    assert ready, 'MCP response timed out'
    return json.loads(process.stdout.readline())

source = args.source_data or Path(invoke('doctor')['data_directory'])
with tempfile.TemporaryDirectory(prefix='sshm-verify-') as directory:
    local = Path(directory)
    data = local / 'catalog'
    data.mkdir()
    with sqlite3.connect(source / 'catalog.sqlite3') as origin, sqlite3.connect(data / 'catalog.sqlite3') as copy:
        origin.backup(copy)
    if (source / 'known_hosts').exists():
        shutil.copy(source / 'known_hosts', data / 'known_hosts')
    host = args.host
    remote = f'/tmp/ssh-manager-verify-{uuid.uuid4().hex}'

    def call(*arguments):
        return invoke(*arguments, data=data)

    def execute(command, *options):
        return call('exec', host, *options, '--', command)

    def success(result):
        assert result.get('status') == 'succeeded', result
        return result

    try:
        check = success(call('check', host))
        expect('password_connection', check['exit_code'] == 0 and 'Linux' in check['stdout'])
        r = execute("printf 'out'; printf 'err' >&2; exit 255")
        expect('remote_exit_255', r['status'] == 'command_failed' and r['exit_code'] == 255 and r['stdout'] == 'out' and r['stderr'] == 'err', r)
        success(execute(f'mkdir -p {shlex.quote(remote)}'))
        path = remote + "/working directory 'quoted'"
        success(execute(f'mkdir -p {shlex.quote(path)}'))
        success_context = call('context', 'project', host, 'verify', '--path', path)
        expect('project_context', success_context['projects'][0]['path'] == path)
        r = success(execute('pwd', '--project', 'verify'))
        expect('quoted_working_directory', r['stdout'].strip() == path, r)
        r = execute('sleep 3; printf finished', '--timeout', '1')
        expect('timeout_unknown_result', r['status'] == 'result_unknown' and r['termination'] == 'timeout' and r['exit_code'] is None, r)
        r = success(execute("head -c 200000 /dev/zero | tr '\\0' x; head -c 180000 /dev/zero | tr '\\0' y >&2", '--output-limit', '1024'))
        expect('bounded_output_and_completion', len(r['stdout']) == 1024 and len(r['stderr']) == 1024 and r['stdout_truncated'] and r['stderr_truncated'] and r['stdout_bytes'] == 200000 and r['stderr_bytes'] == 180000, r)

        payload = local / "input ' [test] $ file.bin"
        payload.write_bytes(os.urandom(262144))
        remote_file = remote + "/payload ' [test] $ file.bin"
        success(call('upload', host, str(payload), remote_file))
        downloaded = local / "download ' [test].bin"
        success(call('download', host, remote_file, str(downloaded)))
        expect('binary_round_trip_sha256', digest(payload) == digest(downloaded))
        tree = local / 'dist'
        (tree / 'nested folder').mkdir(parents=True)
        (tree / 'index.txt').write_text('version one\n')
        (tree / 'nested folder' / '中文 [1].txt').write_text('目录传输\n')
        target = remote + '/site'
        success(call('upload', host, str(tree), target, '--mode', 'contents'))
        (tree / 'index.txt').write_text('version two\n')
        success(call('upload', host, str(tree), target, '--mode', 'contents'))
        r = success(execute(f'test ! -e {shlex.quote(target + "/dist")}; cat {shlex.quote(target + "/index.txt")}'))
        expect('repeat_directory_upload_no_extra_level', r['stdout'] == 'version two\n')
        nested = local / 'nested.txt'
        success(call('download', host, target + '/nested folder/中文 [1].txt', str(nested)))
        expect('nested_unicode_filename', nested.read_text() == '目录传输\n')
        before = call('host', 'show', host)
        call('context', 'note', host, 'Disposable VM verification completed')
        call('host', 'edit', host, '--name', 'renamed-vm')
        after = call('host', 'show', 'renamed-vm')
        expect('stable_identity_across_rename', before['id'] == after['id'] and bool(after['context']['agent_notes']))
        call('host', 'edit', 'renamed-vm', '--name', host)

        cancel_marker = remote + '/cancel-started'
        process = subprocess.Popen([binary, '--json', '--data-dir', str(data), 'exec', host, '--', f'printf started > {shlex.quote(cancel_marker)}; sleep 15'], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            for _ in range(12):
                if execute(f'test -f {shlex.quote(cancel_marker)}')['status'] == 'succeeded':
                    break
                time.sleep(0.1)
            else:
                raise AssertionError('cancel fixture never started')
            process.send_signal(signal.SIGINT)
            stdout, _ = process.communicate(timeout=5)
            r = json.loads(stdout)
            expect('cancel_unknown_result', r['status'] == 'result_unknown' and r['termination'] == 'cancelled' and r['phase'] == 'started', r)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()

        with sqlite3.connect(data / 'catalog.sqlite3') as db:
            body = json.loads(db.execute('SELECT body FROM hosts WHERE name=?', (host,)).fetchone()[0])
            original_auth = body['connections'][0]['authentication']
            address, user = body['connections'][0]['address'], body['connections'][0]['user']
        call('host', 'add', 'verify-jump', '--address', address, '--user', user, '--password')
        call('trust', 'import', 'verify-jump')
        with sqlite3.connect(data / 'catalog.sqlite3') as db:
            body = json.loads(db.execute('SELECT body FROM hosts WHERE name=?', ('verify-jump',)).fetchone()[0])
            body['connections'][0]['authentication'] = original_auth
            db.execute('UPDATE hosts SET body=? WHERE name=?', (json.dumps(body), 'verify-jump'))
        call('host', 'edit', host, '--jump', 'verify-jump')
        r = success(call('check', host))
        expect('password_proxyjump', r['exit_code'] == 0)
        call('host', 'edit', host, '--no-jump')

        mcp = subprocess.Popen([binary, '--data-dir', str(data), 'mcp'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            r = rpc(mcp, {'jsonrpc':'2.0', 'id':1, 'method':'initialize', 'params':{'protocolVersion':'2025-11-25', 'capabilities':{}, 'clientInfo':{'name':'vm-verifier', 'version':'1'}}})
            expect('mcp_initialize', r['result']['protocolVersion'] == '2025-11-25')
            mcp.stdin.write(json.dumps({'jsonrpc':'2.0','method':'notifications/initialized'}) + '\n')
            mcp.stdin.flush()
            r = rpc(mcp, {'jsonrpc':'2.0','id':2,'method':'tools/list'})
            expect('mcp_tool_catalog', len(r['result']['tools']) == 7)
            r = rpc(mcp, {'jsonrpc':'2.0','id':3,'method':'tools/call','params':{'name':'exec','arguments':{'host':host,'command':'printf mcp-ready'}}}, timeout=20)
            expect('mcp_remote_execution', r['result']['structuredContent']['stdout'] == 'mcp-ready' and not r['result']['isError'])
            mcp.stdin.write(json.dumps({'jsonrpc':'2.0','id':4,'method':'tools/call','params':{'name':'exec','arguments':{'host':host,'command':'sleep 10'}}}) + '\n')
            mcp.stdin.flush()
            time.sleep(1)
            r = rpc(mcp, {'jsonrpc':'2.0','method':'notifications/cancelled','params':{'requestId':4}}, timeout=5)
            expect('mcp_cancel', r['id'] == 4 and r['result']['structuredContent']['termination'] == 'cancelled')
        finally:
            mcp.stdin.close()
            mcp.wait(timeout=5)

        history = call('history', '--host', host, '--limit', '100')
        expect('audit_omits_commands_and_output', len(history) > 10 and all('stdout' not in r and 'stderr' not in r and 'command' not in r for r in history))
        export = json.dumps(call('host', 'export'))
        expect('export_omits_credential_bindings', original_auth['credential_ref'] not in export)
        trust_file = data / 'known_hosts'
        original_trust = trust_file.read_text()
        lines = original_trust.splitlines()
        # Give the target a real, different key without altering the VM itself.
        key = local / 'other-host-key'
        subprocess.run(['ssh-keygen','-q','-t','ed25519','-N','','-f',str(key)], check=True)
        public = key.with_suffix('.pub').read_text().split()
        target_id = before['default_connection']
        trust_file.write_text('\n'.join([line for line in lines if not line.startswith('sshm-' + target_id + ' ')] + [f'sshm-{target_id} {public[0]} {public[1]}']) + '\n')
        r = call('check', host)
        expect('host_key_change_stops_connection', r['status'] == 'host_key_changed' and r['exit_code'] is None, r)
        trust_file.write_text('')
        r = call('check', host)
        expect('unknown_host_requires_trust', r['status'] == 'host_key_required', r)
        trust_file.write_text(original_trust)
    finally:
        cleanup = execute(f'rm -rf -- {shlex.quote(remote)}')
        if cleanup.get('status') != 'succeeded':
            print('VM fixture cleanup needs attention', flush=True)

report = {'host':args.host, 'elapsed_seconds':round(time.monotonic() - started, 2), 'cases':results}
args.report.parent.mkdir(parents=True, exist_ok=True)
args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
print(f'{len(results)} cases passed in {report["elapsed_seconds"]}s; report: {args.report}')
