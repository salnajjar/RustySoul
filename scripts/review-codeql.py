#!/usr/bin/env python3
"""Retain CodeQL findings except narrowly documented non-secret test/config flows.

The original SARIF and a separate review report must also be retained. This does
not disable queries, skip test files, dismiss GitHub alerts, or alter severity.
"""
import argparse
import json
import math
from pathlib import Path, PurePosixPath
import re
import tomllib
from urllib.parse import unquote

from tree_sitter import Language, Parser
import tree_sitter_rust

HARDCODED = 'rust/hard-coded-cryptographic-value'
HASHING = 'rust/weak-sensitive-data-hashing'


class Sources:
    def __init__(self, root):
        self.root = Path(root).resolve()
        self.parser = Parser(Language(tree_sitter_rust.language()))
        self.files = {}
        self.test_files = set()
        self.scopes = {}
        self.functions = {}
        self.external = {}
        self.uncertain_includes = set()
        self.parse_errors = set()
        for directory in ['src', 'vendor/soulseek-rs-lib/src']:
            for path in (self.root / directory).rglob('*.rs'):
                self.parse(path.relative_to(self.root).as_posix())
        queue = [p for paths in self.external.values() for p, test in paths if test]
        while queue:
            path = queue.pop()
            if path in self.test_files:
                continue
            self.test_files.add(path)
            self.parse(path)
            queue.extend(p for p, _ in self.external.get(path, []))
        self.runtime_files = set()
        self.unresolved_runtime_include = False
        roots = []
        for project in [self.root, self.root / 'vendor/soulseek-rs-lib']:
            manifest = project / 'Cargo.toml'
            metadata = tomllib.loads(manifest.read_text(encoding='utf-8')) if manifest.exists() else {}
            roots.extend(project / p for p in [metadata.get('lib', {}).get('path', 'src/lib.rs'), 'src/main.rs'])
            roots.extend(project / b['path'] for b in metadata.get('bin', []) if 'path' in b)
            for directory in ['src/bin', 'examples']:
                roots.extend((project / directory).glob('*.rs'))
        queue = [p.relative_to(self.root).as_posix() for p in roots if p.is_file()]
        while queue:
            path = queue.pop()
            if path in self.runtime_files:
                continue
            self.runtime_files.add(path)
            self.parse(path)
            if path in self.uncertain_includes or path in self.parse_errors:
                self.unresolved_runtime_include = True
            queue.extend(p for p, test in self.external.get(path, []) if not test)

    def path(self, uri):
        parts = PurePosixPath(unquote(uri.replace('\\', '/')))
        if parts.is_absolute() or '..' in parts.parts or ':' in str(parts):
            raise ValueError('SARIF source must be a repository-relative path')
        path = (self.root / str(parts)).resolve()
        if not path.is_relative_to(self.root):
            raise ValueError('SARIF source escapes the checkout')
        return path

    def text(self, uri):
        if uri not in self.files:
            self.files[uri] = self.path(uri).read_text(encoding='utf-8')
        return self.files[uri]

    def parse(self, uri):
        if uri in self.scopes:
            return
        self.scopes[uri], self.functions[uri], self.external[uri] = [], [], []
        source = self.text(uri).encode('utf-8')
        tree = self.parser.parse(source)
        if tree.root_node.has_error:
            # Unknown syntax cannot establish a test-only exception.
            self.parse_errors.add(uri)
            return

        def walk(parent, inherited=False):
            attributes = []
            for node in parent.named_children:
                if node.type == 'attribute_item':
                    attributes.append(node.text.decode('utf-8'))
                    continue
                if node.type in ['line_comment', 'block_comment']:
                    continue
                attrs = [re.sub(r'\s+', '', a) for a in attributes]
                test = inherited or any(
                    a in ['#[cfg(test)]', '#[test]', '#[tokio::test]']
                    or (a.startswith('#[cfg(all(test,') and a.endswith('))]'))
                    for a in attrs
                )
                if test:
                    self.scopes[uri].append((node.start_byte, node.end_byte))
                if node.type == 'function_item':
                    name = node.child_by_field_name('name')
                    self.functions[uri].append((node.start_byte, node.end_byte, name.text.decode('utf-8')))
                if node.type == 'mod_item':
                    name = node.child_by_field_name('name').text.decode('utf-8')
                    body = node.child_by_field_name('body')
                    if body:
                        walk(body, test)
                    else:
                        file = self.path(uri)
                        parent_dir = file.parent if file.stem in ['mod', 'lib', 'main'] else file.with_suffix('')
                        custom = next((re.fullmatch(r'#\[\s*path\s*=\s*"([^"]+)"\s*\]', a) for a in attributes if re.match(r'#\[\s*path\b', a)), None)
                        if any(re.match(r'#\[\s*path\b', a) for a in attributes) and not custom:
                            self.uncertain_includes.add(uri)
                        candidates = [file.parent / custom[1]] if custom else [parent_dir / (name + '.rs'), parent_dir / name / 'mod.rs']
                        for candidate in candidates:
                            if candidate.is_file() and candidate.resolve().is_relative_to(self.root):
                                self.external[uri].append((candidate.resolve().relative_to(self.root).as_posix(), test))
                                break
                attributes = []
        walk(tree.root_node)
        pending = [tree.root_node]
        while pending:
            node = pending.pop()
            pending.extend(node.named_children)
            if node.type != 'macro_invocation':
                continue
            macro = node.child_by_field_name('macro')
            if not macro or macro.text != b'include':
                continue
            test = any(start <= node.start_byte < end for start, end in self.scopes[uri])
            arguments = next((n for n in node.named_children if n.type == 'token_tree'), None)
            literal = re.fullmatch(r'\(\s*"([^"\\]+)"\s*\)', arguments.text.decode('utf-8')) if arguments else None
            candidate = self.path(uri).parent / literal[1] if literal else None
            if candidate and candidate.is_file() and candidate.resolve().is_relative_to(self.root):
                self.external[uri].append((candidate.resolve().relative_to(self.root).as_posix(), test))
            elif not test:
                self.uncertain_includes.add(uri)

    def offset(self, location):
        physical = location.get('physicalLocation', location)
        uri = physical['artifactLocation']['uri']
        lines = self.text(uri).splitlines(keepends=True)
        region = physical['region']
        line = region['startLine'] - 1
        if line < 0 or line >= len(lines):
            raise ValueError('SARIF source line is outside the file')
        # SARIF defaults to UTF-16 code units, while tree-sitter uses bytes.
        prefix = lines[line].encode('utf-16-le')[:2 * (region.get('startColumn', 1) - 1)].decode('utf-16-le')
        return uri, len(''.join(lines[:line]).encode('utf-8')) + len(prefix.encode('utf-8'))

    def line(self, location):
        physical = location.get('physicalLocation', location)
        return self.text(physical['artifactLocation']['uri']).splitlines()[physical['region']['startLine'] - 1]

    def test_location(self, location):
        uri, offset = self.offset(location)
        if (not self.unresolved_runtime_include and uri not in self.runtime_files
                and (uri.startswith(('tests/', 'vendor/soulseek-rs-lib/tests/')) or uri in self.test_files)):
            return True
        self.parse(uri)
        return any(start <= offset < end for start, end in self.scopes[uri])

    def function(self, location):
        uri, offset = self.offset(location)
        self.parse(uri)
        return next((name for start, end, name in self.functions[uri] if start <= offset < end), None)


def flow_locations(result):
    return [[x['location'] for x in thread.get('locations', [])]
            for flow in result.get('codeFlows', []) for thread in flow.get('threadFlows', [])]


def reason(result, sources, policy):
    rule = result.get('ruleId')
    locations = result.get('locations', [])
    if len(locations) != 1:
        return None
    location = locations[0]
    uri, _ = sources.offset(location)
    if rule == HARDCODED and sources.test_location(location):
        return 'Non-secret fixture in a Cargo integration test, #[test] function or #[cfg(test)] module.'
    for item in policy['expressions']:
        if item['rule'] != rule or item['path'] != uri:
            continue
        if item['anchor'] not in sources.line(location):
            continue
        if sources.text(uri).count(item['anchor']) != 1:
            continue
        if all(guard['contains'] in sources.text(guard['path']) for guard in item['guards']):
            return item['reason']
    if rule == 'rust/path-injection' and uri == 'src/backup.rs':
        if sources.function(location) not in policy['backup_state']['functions']:
            return None
        flows = flow_locations(result)
        if not flows or not all(flows):
            return None
        for flow in flows:
            first = flow[0]['physicalLocation']['artifactLocation']['uri']
            if first != 'src/api.rs' or not any(expr in sources.line(flow[0]) for expr in ['backups::status', 'backups::inspect']):
                return None
            api = [loc for loc in flow if loc['physicalLocation']['artifactLocation']['uri'] == 'src/api/backups.rs']
            if not any('State(s): State<ApiState>' in sources.line(loc) for loc in api):
                return None
            if not any('s.data_dir' in sources.line(loc) for loc in api):
                return None
            if any(re.search(r'(?:Json|Query)\([^)]*\)\s*:', sources.line(loc)) for loc in api):
                return None
        guards = policy['backup_state']['guards']
        if all(g['contains'] in sources.text(g['path']) for g in guards):
            return policy['backup_state']['reason']
    return None


def review(document, sources, policy):
    report = []
    for run in document.get('runs', []):
        kept = []
        for result in run.get('results', []):
            why = reason(result, sources, policy)
            if why:
                report.append({'rule': result['ruleId'], 'location': result['locations'][0], 'reason': why})
            else:
                kept.append(result)
        run['results'] = kept
    return document, report


def blocking(document, threshold=9.0):
    findings = []
    for run in document.get('runs', []):
        tool = run.get('tool', {})
        components = [tool.get('driver', {}), *tool.get('extensions', [])]
        rules = {r['id']: r for c in components for r in c.get('rules', [])}
        for result in run.get('results', []):
            properties = rules.get(result.get('ruleId'), {}).get('properties', {})
            score = properties.get('security-severity')
            try:
                number = float(score) if score is not None else None
                high = number is not None and (not math.isfinite(number) or not 0 <= number <= 10 or number >= threshold)
            except (ValueError, TypeError):
                high = True
            if (result.get('ruleId') not in rules or high
                    or (score is None and (not properties.get('tags') or 'security' in properties.get('tags', [])))):
                findings.append(result)
    return findings


def scan_errors(document):
    if document.get('version') != '2.1.0' or not document.get('runs'):
        return ['Missing valid SARIF analysis runs']
    errors = []
    for run in document['runs']:
        if not run.get('tool', {}).get('driver', {}).get('name'):
            errors.append('Missing scanner identity')
        for invocation in run.get('invocations', []):
            if invocation.get('executionSuccessful') is False:
                errors.append('Scanner execution failed')
            for notification in invocation.get('toolExecutionNotifications', []):
                descriptor = notification.get('descriptor', {}).get('id', '')
                if notification.get('level') == 'error' or descriptor == 'rust/diagnostic/database-quality':
                    errors.append(notification.get('message', {}).get('text', descriptor))
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', type=Path, required=True)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--report', type=Path)
    parser.add_argument('--source-root', type=Path, default=Path('.'))
    parser.add_argument('--policy', type=Path, default=Path('.github/codeql/reviewed.json'))
    parser.add_argument('--gate', action='store_true')
    parser.add_argument('--gate-only', action='store_true', help='Check already reviewed SARIF without filtering or rewriting it')
    parser.add_argument('--fail-on', choices=['critical', 'high'], default='critical')
    args = parser.parse_args()
    document = json.loads(args.input.read_text(encoding='utf-8'))
    if args.gate_only:
        errors, findings = scan_errors(document), blocking(document, 7.0 if args.fail_on == 'high' else 9.0)
        print(f'CodeQL gate ({args.fail_on}): {len(findings)} blocking findings; {len(errors)} scan failures.')
        for error in errors:
            print(error)
        raise SystemExit(1 if errors or findings else 0)
    if not args.output or not args.report:
        parser.error('Review requires separate --output and --report paths')
    if len({p.resolve() for p in [args.input, args.output, args.report]}) != 3:
        parser.error('Keep the original SARIF; input, output and report need separate paths')
    policy = json.loads(args.policy.read_text(encoding='utf-8'))
    document, report = review(document, Sources(args.source_root), policy)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document), encoding='utf-8')
    args.report.write_text(json.dumps(report, indent=2), encoding='utf-8')
    retained = sum(len(run.get('results', [])) for run in document.get('runs', []))
    blockers = blocking(document, 7.0 if args.fail_on == 'high' else 9.0)
    print(f'CodeQL: {len(report)} reviewed non-secret/intentional flows; {retained} retained findings; {len(blockers)} blockers at {args.fail_on} threshold.')
    if args.gate and (blockers or scan_errors(document)):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
