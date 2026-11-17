#!/usr/bin/env python3
"""Install/upgrade a native Linux systemd service without touching a container installation."""
import argparse
import json
import os
from pathlib import Path
import pwd
import shutil
import subprocess
import sys
from datetime import datetime, timezone

def run(*args):
    return subprocess.run(args,check=True,capture_output=True,text=True).stdout.strip()

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=Path(__file__).resolve().parent/'rustysoul')
    parser.add_argument('--port',type=int,default=8484)
    parser.add_argument('--downloads',type=Path,default=Path('/srv/rustysoul-native/downloads'))
    parser.add_argument('--library',type=Path,action='append',default=[])
    args=parser.parse_args()
    if sys.platform!='linux' or os.geteuid()!=0:parser.error('Run with sudo on a Linux host using systemd')
    if not 1024<=args.port<=65535:parser.error('HTTP port must be 1024..65535')
    if not args.binary.is_file():parser.error('Supply the compiled Linux executable with --binary')
    for path in [args.downloads,*args.library]:
        if not path.is_absolute() or any(c in str(path) for c in '\n\r\0%'):parser.error('Use absolute paths without control characters or percent signs')
        if path.resolve()==Path('/') or str(path.resolve()).startswith(('/etc/','/usr/','/proc/','/sys/','/dev/','/var/lib/rustysoul')):parser.error('Choose dedicated media folders outside system/private data directories')
    # --version checks the architecture and runtime libraries before installing anything.
    run(str(args.binary.resolve()),'--version')
    user='rustysoul'
    try:account=pwd.getpwnam(user)
    except KeyError:
        try:pwd.getpwuid(10001)
        except KeyError:pass
        else:parser.error('UID 10001 belongs to another account; create a dedicated rustysoul service account first')
        run('useradd','--system','--uid','10001','--user-group','--no-create-home','--home-dir','/var/lib/rustysoul-native','--shell','/usr/sbin/nologin',user)
        account=pwd.getpwnam(user)
    if account.pw_uid==0:parser.error('The service account cannot be root')
    config=Path('/etc/rustysoul-native');state=Path('/var/lib/rustysoul-native')
    config.mkdir(mode=0o750,exist_ok=True)
    state.mkdir(mode=0o700,exist_ok=True)
    os.chown(state,account.pw_uid,account.pw_gid);state.chmod(0o700)
    fresh=not (config/'rustysoul.toml').exists()
    if fresh:
        if not args.library:
            library=Path('/srv/rustysoul-native/library');library.mkdir(parents=True,exist_ok=True);os.chown(library,account.pw_uid,account.pw_gid);args.library=[library]
        if not args.downloads.exists():args.downloads.mkdir(parents=True);os.chown(args.downloads,account.pw_uid,account.pw_gid)
        for path in [args.downloads,*args.library]:
            if not path.is_dir():parser.error(f'Folder must exist: {path}')
        # Inspect access as the service account; never recursively chown existing media.
        run('runuser','-u',user,'--','test','-w',str(args.downloads))
        for path in args.library:run('runuser','-u',user,'--','test','-r',str(path))
        q=lambda s:json.dumps(str(s))
        roots=[str(args.downloads.resolve()),*[str(p.resolve()) for p in args.library]]
        text=f'bind = "127.0.0.1:{args.port}"\nallow_remote = true\ndata_dir = "{state}"\ndownload_dir = {q(args.downloads.resolve())}\nlocation_roots = {json.dumps(roots)}\n\n[soulseek]\nshares = {json.dumps(roots[1:])}\nauto_connect = true\n'
        (config/'rustysoul.toml').write_text(text)
        (config/'daemon.env').write_text('RUSTYSOUL_DEFAULT_USERNAME=admin\nRUSTYSOUL_DEFAULT_PASSWORD=change-me-now\n')
        (config/'daemon.env').chmod(0o600)
    else:
        print('Existing native configuration and account data will be retained; install flags do not override them.')
    # Unit write access matches the saved config, including upgrades.
    import tomllib
    saved=tomllib.loads((config/'rustysoul.toml').read_text())
    def unit_path(value):return '"'+str(value).replace('\\','\\\\').replace('"','\\"').replace('%','%%')+'"'
    unit=(Path(__file__).resolve().parent/'rustysoul-native.service').read_text()
    unit=unit.replace('@DOWNLOADS@',unit_path(saved['download_dir']))
    unit=unit.replace('@GROUP@',str(account.pw_gid))
    notices=Path(__file__).resolve().parent/'THIRD_PARTY_NOTICES.md'
    target=Path('/usr/local/bin/rustysoul-native');service=Path('/etc/systemd/system/rustysoul-native.service')
    stamp=datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    for existing in [target,service]:
        if existing.exists():shutil.copy2(existing,existing.with_name(existing.name+'.backup-'+stamp))
    temporary=target.with_name(target.name+'.new');shutil.copyfile(args.binary,temporary);temporary.chmod(0o755);os.replace(temporary,target)
    os.chown(config/'rustysoul.toml',0,account.pw_gid);(config/'rustysoul.toml').chmod(0o640)
    os.chown(config,0,account.pw_gid);config.chmod(0o750)
    if notices.exists():
        destination=Path('/usr/local/share/doc/rustysoul');destination.mkdir(parents=True,exist_ok=True);shutil.copyfile(notices,destination/notices.name)
    service.write_text(unit);service.chmod(0o644)
    run(str(target),'--config',str(config/'rustysoul.toml'),'--check')
    run('systemctl','daemon-reload')
    print('Native service installed. Existing service state is unchanged.')
    print('Start: sudo systemctl enable --now rustysoul-native.service')
    print('Config:',config/'rustysoul.toml')
    print('Keep the container stopped if it uses the same HTTP/transfer ports or Soulseek account.')

if __name__=='__main__':main()
