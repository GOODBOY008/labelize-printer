#!/usr/bin/env python3
"""Real daemon/RAW/IPP tests; never registers a system queue or login service."""
import argparse, errno, http.client, json, os, pathlib, signal, socket, struct, subprocess, tempfile, time

parser = argparse.ArgumentParser()
parser.add_argument('--profile', default='debug')
parser.add_argument('--skip-ipp', action='store_true')
parser.add_argument('--disk-full-dir', type=pathlib.Path, help='Dedicated small tmpfs for real ENOSPC testing')
args = parser.parse_args()
root = pathlib.Path(__file__).resolve().parents[1]
binary = root / 'target' / args.profile / ('labelize-printer-daemon.exe' if os.name == 'nt' else 'labelize-printer-daemon')

def eventually(fn, timeout=30):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            last = fn()
            if last: return last
        except (OSError, subprocess.SubprocessError): pass
        time.sleep(.1)
    raise AssertionError(f'Timed out: {last}')

def attribute(tag, name, value):
    name = name.encode()
    if isinstance(value, int): value = struct.pack('>i', value)
    else: value = value.encode()
    return bytes([tag]) + struct.pack('>H', len(name)) + name + struct.pack('>H', len(value)) + value

def ipp(port, title, data, mime, copies=1, per_set=None):
    body = b'\x02\x00\x00\x02\x00\x00\x00\x01\x01'
    for tag, key, value in [(0x47, 'attributes-charset', 'utf-8'), (0x48, 'attributes-natural-language', 'en'),
        (0x45, 'printer-uri', f'ipp://127.0.0.1:{port}/ipp/print'), (0x42, 'requesting-user-name', 'smoke'),
        (0x42, 'job-name', title), (0x49, 'document-format', mime)]: body += attribute(tag,key,value)
    body += b'\x02' + attribute(0x21, 'copies', copies)
    if per_set: body += attribute(0x21, 'job-pages-per-set', per_set)
    body += b'\x03' + data
    connection = http.client.HTTPConnection('127.0.0.1', port, timeout=60)
    connection.request('POST', '/ipp/print', body, {'Content-Type': 'application/ipp'})
    response = connection.getresponse(); payload = response.read(); connection.close()
    assert response.status == 200, (response.status, payload)
    status = struct.unpack('>H', payload[2:4])[0]
    assert status < 0x100, (status, payload)
    return payload

with tempfile.TemporaryDirectory(prefix='labelize-smoke-') as temporary:
    work = pathlib.Path(temporary).resolve()
    state = work / 'state'
    log = open(work / 'daemon.log', 'w+', encoding='utf-8')
    daemon = subprocess.Popen([str(binary), '--data-dir', str(state), 'serve'], stdout=log, stderr=log)
    def cli(*command):
        result = subprocess.run([str(binary), '--data-dir', str(state), *command], capture_output=True, encoding='utf-8', timeout=360)
        if result.returncode: raise RuntimeError(result.stderr)
        return json.loads(result.stdout)
    def status():
        try: return cli('status')
        except RuntimeError: return None
    def finished(title):
        s = status()
        return next((j for j in s['jobs'] if j['title'] == title and j['state'] in ('saved','failed')), None) if s else None
    try:
        initial = eventually(status)
        settings = initial['settings']; settings.update(output_dir=str(work / '中文 output'), autostart=False, format='png')
        config = work / 'settings.json'; config.write_text(json.dumps(settings))
        cli('settings', '--file', str(config))
        job = cli('test-print', '--wait')
        assert job['state'] == 'saved' and job['page_count'] == 1
        assert pathlib.Path(job['output_files'][0]).is_file(), job['output_files']
        print('PASS headless daemon ZPL -> PNG')

        raw = socket.create_connection(('127.0.0.1', settings['raw_port']))
        for chunk in [b'^XA^PW400^LL240', b'^FO0,0^FDfirst^FS^PQ2', b'^XZ']:
            raw.sendall(chunk); time.sleep(.05)
        time.sleep(.25)
        receiving = [j for j in cli('status')['jobs'] if j['source'] == 'raw-tcp']
        assert len(receiving) == 1 and receiving[0]['state'] == 'receiving'
        raw.sendall(b'^XA^PW800^LL1200^FO0,0^FDsecond^FS^PQ1^XZ'); raw.close()
        raw_id = receiving[0]['id']
        complete = eventually(lambda: next((j for j in cli('status')['jobs'] if j['id'] == raw_id and j['state'] == 'saved'), None))
        assert complete['page_count'] == 3 and len(complete['output_files']) == 3
        print('PASS RAW fragmentation, EOF job boundary, mixed media, ^PQ once')

        malformed = work / 'broken.zpl'; malformed.write_bytes(b'^XA^FO0,0^FDpartial^FS')
        broken = cli('submit', str(malformed), '--title', 'incomplete')
        failed = eventually(lambda: finished('incomplete'))
        assert failed['state'] == 'failed' and not failed['output_files']
        assert (state / 'jobs' / broken['id'] / 'input.zpl').exists()
        print('PASS truncated ZPL fails with input retained')

        if args.disk_full_dir:
            limited = args.disk_full_dir.resolve()
            assert limited.is_dir() and not any(limited.iterdir()), 'Use an empty test-only tmpfs'
            ordinary_output = settings['output_dir']
            settings['output_dir'] = str(limited)
            config.write_text(json.dumps(settings)); cli('settings', '--file', str(config))
            filler = limited / 'fill-disk'
            try:
                with filler.open('wb', buffering=0) as f:
                    while True: f.write(b'x' * 4096)
            except OSError as error:
                assert error.errno == errno.ENOSPC, error
            disk_job = cli('test-print')
            def disk_failed():
                job = cli('job', disk_job['id'])
                return job if job['state'] == 'failed' else None
            failed = eventually(disk_failed)
            assert not failed['output_files'] and failed['transport_complete'], failed
            assert 'space' in failed['error'].lower(), failed
            filler.unlink()
            retried = cli('retry', disk_job['id'], '--wait')
            assert retried['id'] == disk_job['id'] and retried['state'] == 'saved'
            assert retried['attempts'] == 2
            for file in limited.iterdir():
                if file.is_dir():
                    import shutil
                    shutil.rmtree(file)
            settings['output_dir'] = ordinary_output
            config.write_text(json.dumps(settings)); cli('settings', '--file', str(config))
            print('PASS actual ENOSPC fails without committed output; same-ID retry saves after space is freed')

        if os.name != 'nt':
            crash_input = work / 'worker-crash.zpl'
            crash_input.write_bytes(b'^XA^PW816^LL1216^FO24,24^A0N,48,48^FDworker isolation^FS^PQ100^XZ')
            crash_job = cli('submit', str(crash_input), '--title', 'worker-crash')
            def worker_pid():
                for line in subprocess.check_output(['ps','-eo','pid,ppid,args'], text=True).splitlines()[1:]:
                    parts = line.split(None,2)
                    if len(parts) == 3 and parts[1] == str(daemon.pid) and 'worker '+crash_job['id'] in parts[2]: return int(parts[0])
                return None
            pid = eventually(worker_pid)
            os.kill(pid, signal.SIGKILL)
            failed = eventually(lambda: finished('worker-crash'))
            assert failed['state'] == 'failed' and failed['transport_complete']
            retried = cli('retry', crash_job['id'], '--wait')
            assert retried['id'] == crash_job['id'] and retried['state'] == 'saved' and retried['page_count'] == 100
            assert retried['attempts'] == 2
            print('PASS worker crash is isolated; retry keeps the job ID and saves all 100 pages')

        # Restart the real service and verify committed output/history survives.
        daemon.terminate(); daemon.wait(timeout=10); time.sleep(2.2)
        daemon = subprocess.Popen([str(binary), '--data-dir', str(state), 'serve'], stdout=log, stderr=log)
        restarted = eventually(status)
        settings = restarted['settings']
        assert next(j for j in restarted['jobs'] if j['id'] == complete['id'])['output_files'] == complete['output_files']
        print('PASS service restart preserves saved output and history')

        if not args.skip_ipp:
            assert initial['platform']['ipp_running'], initial['platform']
            # An adapter whose disk is full must be able to fail a receiving
            # task through IPC even when it cannot produce a manifest.
            begun = subprocess.check_output([str(binary), '--data-dir', str(state),
                'begin-raster', 'manifest-write-failure', 'ipp-manifest-failure'], text=True).strip()
            failed = cli('abort-raster', begun, 'Manifest write failed: No space left on device')
            assert failed['state'] == 'failed' and not failed['transport_complete']
            print('PASS IPP manifest write failure leaves a failed task, not a stuck reception')
            fixture = root / '.cache' / 'raster-fixture'
            subprocess.run(['cc', str(root / 'native/ipp/fixture.c'), '-lcups', '-o', str(fixture)], check=True)
            cases = [
                ('pwg',8,2,None,4), ('pwg',1,1,None,2), ('urf',8,1,None,2),
                ('pwg',24,1,None,2), ('urf',24,1,None,2), ('pwg',8,2,1,2),
            ]
            for mode, bits, copies, per_set, expected in cases:
                title = f'{mode}-{bits}-{copies}-{per_set}'
                raster = work / 'fixture.ras'
                subprocess.run([str(fixture), str(raster), mode, str(bits)], check=True)
                result = ipp(settings['ipp_port'], title, raster.read_bytes(), 'image/urf' if mode == 'urf' else 'image/pwg-raster', copies, per_set)
                job = eventually(lambda: finished(title))
                assert job['state'] == 'saved', job
                assert job['page_count'] == expected, job
                pages = job['input']['pages']
                assert abs(pages[0]['width_mm'] - 101.6) < .001 and abs(pages[1]['height_mm'] - 76.2) < .001, pages
                # Check PNG IHDR and unfiltered top-left pixels via the normalized spool.
                for i, page in enumerate(pages):
                    data = (state / 'jobs' / job['id'] / page['file']).read_bytes()
                    pixels = data.split(b'\n',3)[3]
                    expected_gray = (76, 150)[i % 2] if bits == 24 else (0, 64)[i % 2]
                    assert pixels[0] == (0 if bits == 1 else expected_gray), (title, i, pixels[0])
                print(f'PASS IPP {title}: {expected} pages, dimensions, order, polarity')
            ipp(settings['ipp_port'], 'truncated-raster', raster.read_bytes()[:-50], 'image/pwg-raster')
            job = eventually(lambda: finished('truncated-raster'))
            assert job['state'] == 'failed' and not job['output_files'], job
            print('PASS truncated raster does not become saved output')

        # A busy port does not take over an existing printer service on first run.
        second_state = work / 'second-state'
        second = subprocess.Popen([str(binary), '--data-dir', str(second_state), 'serve'], stdout=log, stderr=log)
        try:
            def second_status():
                r = subprocess.run([str(binary), '--data-dir', str(second_state), 'status'], capture_output=True, encoding='utf-8')
                return json.loads(r.stdout) if r.returncode == 0 else None
            second_info = eventually(second_status)
            assert second_info['settings']['raw_port'] != settings['raw_port']
            if not args.skip_ipp:
                assert second_info['settings']['ipp_port'] != settings['ipp_port']
            print('PASS first-run port conflicts select and persist distinct ports')
        finally:
            second.terminate(); second.wait(timeout=10)
    except Exception:
        log.flush(); log.seek(0); print(log.read())
        ipp_log = state / 'ipp.log'
        if ipp_log.exists(): print(ipp_log.read_text()[-10000:])
        raise
    finally:
        daemon.terminate(); daemon.wait(timeout=10); log.close()
        # Let the adapter's parent-liveness timer release ports before deleting state.
        time.sleep(2.2)
