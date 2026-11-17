#!/usr/bin/env python3
"""Build only the public project site and its three selected screenshots."""
import argparse
from pathlib import Path
import shutil

root = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, default=root / 'artifacts/site')
args = parser.parse_args()
output = args.output.resolve()
if output == root or output in (root / 'site', root / 'screens'):
    parser.error('Choose a separate build-output directory')
output.mkdir(parents=True, exist_ok=True)
# Explicit paths keep private docs, credentials and unrelated artifacts out.
for relative in ['index.html', 'styles.css', 'site.js', '.nojekyll', 'assets/mark.svg']:
    target = output / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(root / 'site' / relative, target)
for name in ['search.png', 'chat.png', 'settings.png']:
    target = output / 'assets/screens' / name
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(root / 'screens' / name, target)
print(output)
