#!/usr/bin/env python3
"""Exercise real IPC admission and the headless update helper without installing software."""
import argparse, json, os, pathlib, socket, subprocess, sys, tempfile, time, uuid

p = argparse.ArgumentParser()
p.add_argument('--profile', default='debug')
p.add_argument('--target-dir', type=pathlib.Path, default=pathlib.Path('target'))
a = p.parse_args()
root = pathlib.Path(__file__).resolve().parents[2]
base = (root / a.target_dir / a.profile).resolve()
suffix = '.exe' if os.name == 'nt' else ''
daemon_bin = base / ('labelize-printer-daemon' + suffix)
helper_bin = base / ('labelize-printer-updater' + suffix)

def eventually(fn, timeout=20):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        try:
            value = fn()
            if value: return value
        except (OSError, ValueError, RuntimeError): pass
        time.sleep(.05)
    raise AssertionError('Timed out')

with tempfile.TemporaryDirectory(prefix='labelize-update-') as work:
    work = pathlib.Path(work).resolve(); state = work / 'state'
    log = open(work / 'service.log', 'w+', encoding='utf-8')
    def start(): return subprocess.Popen([str(daemon_bin), '--data-dir', str(state), 'serve'], stdout=log, stderr=log)
    def cli(*args, ok=True):
        result = subprocess.run([str(daemon_bin), '--data-dir', str(state), *args], capture_output=True, text=True, timeout=40)
        if ok:
            if result.returncode: raise RuntimeError(result.stderr)
            return json.loads(result.stdout)
        assert result.returncode != 0, args
        return result.stderr
    def status(): return cli('status')
    daemon = start(); helpers = []; parent = None
    try:
        info = eventually(status)
        settings = info['settings']; settings.update(autostart=False, output_dir=str(work / 'output'))
        config = work / 'settings.json'; config.write_text(json.dumps(settings)); cli('settings', '--file', str(config))
        raw = socket.create_connection(('127.0.0.1', settings['raw_port']))
        raw.sendall(b'^XA^PW400^LL240^FO20,20^FDupgrade safety^FS^XZ')
        eventually(lambda: any(j['state'] == 'receiving' for j in status()['jobs']))
        token = str(uuid.uuid4())
        assert 'active' in cli('prepare-update', token, ok=False).lower()
        raw.close()
        saved = eventually(lambda: next((j for j in status()['jobs'] if j['state'] == 'saved'), None))
        eventually(lambda: not any(j['state'] in ('receiving','queued','processing') for j in status()['jobs']))
        time.sleep(.1)
        cli('prepare-update', token)
        assert not status()['raw_listening']
        cli('test-print', ok=False)
        source = work / 'input.zpl'; source.write_text('^XA^FDnew^FS^XZ')
        cli('submit', str(source), ok=False)
        cli('retry', saved['id'], ok=False)
        cli('begin-raster', 'during-update', 'ipp-update', ok=False)
        try:
            socket.create_connection(('127.0.0.1', settings['raw_port']), timeout=1).close()
            raise AssertionError('RAW listener remained open')
        except (ConnectionRefusedError, OSError): pass
        cli('commit-update', str(uuid.uuid4()), ok=False)
        cli('cancel-update', str(uuid.uuid4()), ok=False)
        cli('cancel-update', token)
        eventually(lambda: status()['raw_listening'])
        assert cli('test-print', '--wait')['state'] == 'saved'
        print('PASS active RAW blocks update; prepared gate rejects RAW/IPP/import/retry; cancel resumes printing')

        def launch(parent_pid):
            identifier = str(uuid.uuid4()); directory = state / 'updates' / identifier; directory.mkdir(parents=True)
            ticket = dict(id=identifier, data_dir=str(state), parent=parent_pid, app=str(base / ('labelize-printer' + suffix)),
                plan=dict(source='Homebrew', version='99.0.0', installer=None, manual=True), phase='preparing', error=None)
            request = directory / 'request.json'; request.write_text(json.dumps(ticket))
            helper = subprocess.Popen([str(helper_bin), '--handoff', str(request)], stdout=log, stderr=log); helpers.append(helper)
            eventually(lambda: (directory / 'ready').exists())
            return helper, directory
        helper, directory = launch(os.getpid())
        assert (state / 'update-pending.json').exists()
        other = start(); assert other.wait(timeout=10) == 0
        (directory / 'cancel').write_text('cancel'); assert helper.wait(timeout=10) != 0
        assert not (state / 'update-pending.json').exists()
        eventually(lambda: status()['raw_listening'])
        print('PASS helper cancellation removes durable gate; duplicate daemon cannot start during handoff')

        parent = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])
        helper, directory = launch(parent.pid)
        parent.terminate(); parent.wait(timeout=10); parent = None
        assert helper.wait(timeout=10) != 0
        assert not (state / 'update-pending.json').exists()
        eventually(lambda: status()['raw_listening'])
        print('PASS closing desktop before authorization cancels handoff and restores printing')

        # A child represents the desktop. Manual Homebrew handoff executes no installer or brew command.
        parent = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])
        helper, directory = launch(parent.pid)
        (directory / 'go').write_text('install')
        time.sleep(.25)
        assert daemon.poll() is None and helper.poll() is None
        parent.terminate(); parent.wait(timeout=10); parent = None
        assert helper.wait(timeout=20) == 0
        assert daemon.wait(timeout=10) == 0
        record = json.loads((state / 'update-pending.json').read_text()); assert record['phase'] == 'verify'
        assert pathlib.Path(saved['output_files'][0]).exists()
        blocked = start(); assert blocked.wait(timeout=10) == 0
        print('PASS helper waits for desktop, stops idle daemon/IPP, retains pending verification and saved output')
        # Test-only acknowledgement: no software was replaced in this smoke run.
        (state / 'update-pending.json').unlink()
        daemon = start(); eventually(status)
        assert cli('test-print', '--wait')['state'] == 'saved'
        assert cli('job', saved['id'])['output_files'] == saved['output_files']
        print('PASS restart preserves history/settings and PDF output after simulated handoff')
        if os.name != 'nt':
            token = str(uuid.uuid4()); cli('prepare-update', token)
            (state / 'update-pending.json').write_text('{"test_only":true}')
            # Lose the commit response deliberately: the service must still exit cleanly.
            hashed = 0xcbf29ce484222325
            for byte in str(state).encode(): hashed = ((hashed ^ byte) * 0x100000001b3) & 0xffffffffffffffff
            endpoint = pathlib.Path(tempfile.gettempdir()) / f'labelize-printer-{hashed:016x}' / 'ipc.sock'
            stream = socket.socket(socket.AF_UNIX); stream.connect(str(endpoint))
            stream.sendall((json.dumps(dict(token=(state/'ipc.token').read_text(), request=dict(command='commit_update', token=token))) + '\n').encode())
            stream.shutdown(socket.SHUT_RDWR); stream.close()
            assert daemon.wait(timeout=10) == 0
            assert (state / 'update-pending.json').exists()
            print('PASS lost commit response still stops service and preserves restart protection')
    except BaseException:
        log.flush(); log.seek(0); print(log.read()); raise
    finally:
        if parent and parent.poll() is None: parent.terminate(); parent.wait(timeout=10)
        for helper in helpers:
            if helper.poll() is None: helper.terminate(); helper.wait(timeout=10)
        if daemon.poll() is None: daemon.terminate(); daemon.wait(timeout=10)
        log.close(); time.sleep(2.2)
