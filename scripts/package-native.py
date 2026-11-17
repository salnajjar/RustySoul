#!/usr/bin/env python3
"""Package the Linux executable, native installer, service and documentation."""
import argparse
import hashlib
from pathlib import Path
import tarfile
root=Path(__file__).resolve().parent.parent
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary',type=Path)
args=parser.parse_args()
output=root/'artifacts/rustysoul-linux-x86_64.tar.gz'
output.parent.mkdir(parents=True,exist_ok=True)
with tarfile.open(output,'w:gz') as tar:
    for source,name in [(args.binary,'rustysoul'),(root/'deploy/install-native.py','install-native.py'),(root/'deploy/rustysoul-native.service','rustysoul-native.service'),(root/'LICENSE','LICENSE'),(root/'THIRD_PARTY_NOTICES.md','THIRD_PARTY_NOTICES.md'),(root/'docs/NATIVE.md','README.md'),*( (root/'docs'/name,name) for name in ['SESSION_AND_ANALYTICS.md','FEATURES.md','SECURITY.md','API.md','TESTING.md','MONITORING.md','STORAGE_PROTECTION.md','TRANSFER_RELIABILITY.md','TRANSFER_QUEUE.md','CONTAINERS.md','INTEROPERABILITY.md','SOULFIND_SLSKD.md','BACKUPS.md','PRIVILEGES.md','MAINTENANCE.md','API_ACCESS.md','USER_DISCOVERY.md'])]:
        info=tar.gettarinfo(source,arcname='rustysoul/'+name)
        info.mode=0o755 if name in ('rustysoul','install-native.py') else 0o644
        info.uid=info.gid=0;info.uname=info.gname='root'
        with source.open('rb') as stream:tar.addfile(info,stream)
checksum=hashlib.sha256(output.read_bytes()).hexdigest()
output.with_suffix(output.suffix+'.sha256').write_text(checksum+'  '+output.name+'\n')
print(output)
