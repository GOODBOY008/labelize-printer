#!/usr/bin/env bash
set -euo pipefail
[[ -f /.dockerenv ]]
version="$(python3 /build/scripts/package-version.py)"
package="/build/dist/labelize-printer_${version}_amd64.deb"
[[ "$(dpkg-deb --field "$package" Version)" == "$version" ]]
dpkg -i "$package"
mkdir -p /run/dbus
[ -e /run/dbus/system_bus_socket ] || dbus-daemon --system --fork
/usr/sbin/cupsd
export LABELIZE_PRINTER_HOME=/tmp/labelize-system-smoke
/opt/labelize-printer/labelize-printer-daemon serve >/build/.cache/linux-system-daemon.log 2>&1 &
python3 - "$version" <<'PY'
import json,pathlib,subprocess,sys,time
cli='/opt/labelize-printer/labelize-printer-daemon'
def call(*args):
    r=subprocess.run([cli,*args],text=True,capture_output=True,check=True)
    return json.loads(r.stdout)
for _ in range(150):
    try: s=call('status'); break
    except subprocess.CalledProcessError: time.sleep(.2)
else: raise RuntimeError('Daemon unavailable')
assert s['version'] == sys.argv[1], s['version']
settings=s['settings'];settings.update(autostart=False,output_dir='/tmp/labelize-system-output',format='pdf')
p=pathlib.Path('/tmp/labelize-settings.json');p.write_text(json.dumps(settings))
call('settings','--file',str(p));call('install')
source=call('test-print','--wait')
subprocess.run(['lp','-d','LabelizePrinter','-n','2','-o','media=na_index-4x6_4x6in',source['output_files'][0]],check=True)
for _ in range(300):
    jobs=[j for j in call('status')['jobs'] if j['source'].startswith('ipp:')]
    if jobs and jobs[0]['state'] in ('failed','saved'): break
    time.sleep(.2)
else: raise RuntimeError('System print did not finish')
j=jobs[0];assert j['state']=='saved',j
assert j['page_count']==2,j
assert len(j['output_files'])==1 and pathlib.Path(j['output_files'][0]).is_file(),j
for page in j['input']['pages']:
    assert abs(page['width_mm']-101.6)<.02 and abs(page['height_mm']-152.4)<.02,page
pathlib.Path('/build/.cache/linux-system-job.json').write_text(json.dumps(j,indent=2))
print('PASS installed DEB: CUPS queue -> real PDF filter -> PAPPL -> 2-page PDF, 101.6 x 152.4 mm',flush=True)
PY
/opt/labelize-printer/labelize-printer-daemon uninstall >/dev/null
/opt/labelize-printer/labelize-printer-daemon stop >/dev/null
if lpstat -p LabelizePrinter >/dev/null 2>&1; then echo 'Queue was not removed' >&2; exit 1; fi
dpkg -r labelize-printer
echo 'PASS installed package queue removal, daemon stop and DEB removal'
