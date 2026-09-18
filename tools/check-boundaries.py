#!/usr/bin/env python3
"""Check direct Cargo dependencies, including build/dev dependencies."""
import json
import subprocess
import sys

metadata = json.loads(subprocess.check_output([
    'cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1', '--all-features'
], text=True))
local_names = {p['name'] for p in metadata['packages']}
allowed = {
    'gsm-ctrlc-helper': {'gsm-infra'},
    'gsm-domain': set(),
    'gsm-application': {'gsm-domain'},
    'gsm-infra': {'gsm-domain'},
    'gsm-mock': {'gsm-domain'},
}
errors = []
for package in metadata['packages']:
    name = package['name']
    if name == 'manager-gui':
        continue  # Composition root owns registration of optional game modules.
    permit = {'gsm-domain'} if name.startswith('game-') else allowed.get(name)
    if permit is None:
        errors.append(f'{name}: define a boundary rule before adding this crate')
        continue
    for dep in package['dependencies']:
        target = dep['name']
        if name == 'gsm-application' and dep['kind'] == 'dev' and target == 'gsm-mock':
            continue
        if name == 'gsm-ctrlc-helper' and dep['kind'] == 'dev' and target == 'gsm-domain':
            continue
        if target in local_names and target not in permit:
            errors.append(f'{name} -> {target}: prohibited {dep["kind"] or "normal"} dependency')
        if name in allowed and (target.startswith('slint') or target.startswith('game-')):
            errors.append(f'{name} -> {target}: shared layers must not depend on GUI/game modules')
if errors:
    print('\n'.join(errors), file=sys.stderr)
    sys.exit(1)
print(f'Dependency boundaries OK ({len(local_names)} crates)')
