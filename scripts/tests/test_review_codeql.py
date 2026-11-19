import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'review-codeql.py'
spec = importlib.util.spec_from_file_location('review_codeql', SCRIPT)
review = importlib.util.module_from_spec(spec)
spec.loader.exec_module(review)
EMPTY_POLICY = {'expressions': [], 'backup_state': {'functions': [], 'guards': []}}


def location(path, line, column=1):
    return {'physicalLocation': {'artifactLocation': {'uri': path}, 'region': {'startLine': line, 'startColumn': column}}}


def finding(path, line, rule=review.HARDCODED, column=1):
    return {'ruleId': rule, 'level': 'warning', 'locations': [location(path, line, column)]}


def sarif(results, extension=False):
    rules = [{'id': rule, 'properties': {'security-severity': '9.8', 'tags': ['security']}} for rule in {r['ruleId'] for r in results}]
    tool = {'driver': {'name': 'CodeQL', 'rules': [] if extension else rules}}
    if extension:
        tool['extensions'] = [{'name': 'Rust rules', 'rules': rules}]
    return {'version': '2.1.0', 'runs': [{'tool': tool, 'results': results}]}


class ReviewPolicyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.addCleanup(self.temp.cleanup)

    def write(self, path, text):
        file = self.root / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(text, encoding='utf-8')

    def apply(self, results, policy=EMPTY_POLICY):
        return review.review(sarif(results), review.Sources(self.root), policy)

    def test_only_hardcoded_test_credentials_are_reviewed(self):
        self.write('src/lib.rs', '''fn live() { use_password("runtime-secret"); }
#[cfg(test)]
mod tests { fn helper() { use_password("fixture"); } }
#[test]
fn case() { use_password("fixture"); }
#[cfg(any(test, unix))]
fn sometimes_production() { use_password("runtime-secret"); }
''')
        results = [finding('src/lib.rs', line) for line in [1, 3, 5, 7]]
        # A weak runtime password hasher remains a finding even when a test
        # supplies its input. This is not a blanket test-file exclusion.
        results.append(finding('src/lib.rs', 5, review.HASHING))
        doc, report = self.apply(results)
        self.assertEqual(len(report), 2)
        self.assertEqual(len(doc['runs'][0]['results']), 3)

    def test_rust_ast_ignores_decoy_comments_and_raw_strings(self):
        self.write('src/lib.rs', '''const TEXT: &str = r#"#[cfg(test)] mod fake { }"#;
/* #[test] fn fake() { */
fn live() { use_password("runtime"); }
#[cfg(all(test, unix))]
mod real { fn fixture() { use_password("fixture"); } }
''')
        doc, report = self.apply([finding('src/lib.rs', 3), finding('src/lib.rs', 5)])
        self.assertEqual(len(report), 1)
        self.assertEqual(doc['runs'][0]['results'][0]['locations'][0], location('src/lib.rs', 3))

    def test_test_scope_does_not_extend_past_closing_brace_on_same_line(self):
        text = '#[cfg(test)] mod cases { fn test() {} } fn live() { use_password("runtime"); }'
        self.write('src/lib.rs', text)
        doc, report = self.apply([finding('src/lib.rs', 1, column=text.index('use_password') + 1)])
        self.assertEqual(report, [])
        self.assertEqual(len(doc['runs'][0]['results']), 1)

    def test_out_of_line_test_modules_and_integration_fixtures(self):
        self.write('src/lib.rs', '#[cfg(test)]\nmod cases;\nmod live;')
        self.write('src/cases.rs', 'mod helper;\nfn fixture() { use_password("fixture"); }')
        self.write('src/cases/helper.rs', 'fn fixture() { use_password("fixture"); }')
        self.write('src/live.rs', 'fn live() { use_password("runtime"); }')
        self.write('tests/fixtures/auth.rs', 'fn fixture() { use_password("fixture"); }')
        doc, report = self.apply([finding(path, line) for path, line in [('src/cases.rs', 2), ('src/cases/helper.rs', 1), ('tests/fixtures/auth.rs', 1), ('src/live.rs', 1)]])
        self.assertEqual(len(report), 3)
        self.assertEqual(len(doc['runs'][0]['results']), 1)

    def test_invalid_syntax_cannot_create_a_test_scope(self):
        self.write('src/lib.rs', '#[cfg(test)]\nmod cases { fn broken( }\nfn live() {}')
        doc, report = self.apply([finding('src/lib.rs', 3)])
        self.assertEqual(report, [])
        self.assertEqual(len(doc['runs'][0]['results']), 1)

    def test_production_imports_override_test_filename_conventions(self):
        self.write('src/lib.rs', '#[path="../tests/shared.rs"]\nmod shared;\ninclude!("../tests/included.rs");')
        self.write('tests/shared.rs', 'fn live() { use_password("runtime"); }')
        self.write('tests/included.rs', 'fn also_live() { use_password("runtime"); }')
        doc, report = self.apply([finding('tests/shared.rs', 1), finding('tests/included.rs', 1)])
        self.assertEqual(report, [])
        self.assertEqual(len(doc['runs'][0]['results']), 2)
        self.write('src/lib.rs', 'include!(concat!(env!("OUT_DIR"), "/generated.rs"));')
        self.assertEqual(len(self.apply([finding('tests/shared.rs', 1)])[1]), 0)

    def test_reviewed_expression_requires_unique_anchor_and_all_guards(self):
        self.write('src/lib.rs', 'fn token_digest() { fast_hash(random_token); }\nconst SECURE_GENERATION: bool = true;')
        policy = {'expressions': [{'rule': review.HASHING, 'path': 'src/lib.rs', 'anchor': 'fast_hash(random_token)', 'guards': [{'path': 'src/lib.rs', 'contains': 'SECURE_GENERATION: bool = true'}], 'reason': 'Reviewed generated-token digest'}], 'backup_state': EMPTY_POLICY['backup_state']}
        result = finding('src/lib.rs', 1, review.HASHING)
        self.assertEqual(len(self.apply([result], policy)[1]), 1)
        self.write('src/lib.rs', 'fn token_digest() { fast_hash(random_token); }')
        self.assertEqual(len(self.apply([result], policy)[1]), 0)
        self.write('src/lib.rs', 'fn token_digest() { fast_hash(random_token); fast_hash(random_token); }\nconst SECURE_GENERATION: bool = true;')
        self.assertEqual(len(self.apply([result], policy)[1]), 0)

    def test_backup_state_exception_keeps_actual_request_flows(self):
        self.write('src/api.rs', 'fn router() { get(backups::status); }\nconst IMMUTABLE: bool = true;')
        self.write('src/api/backups.rs', 'fn status(State(s): State<ApiState>) {\nlet root = s.data_dir;\n}\nfn inspect(Json(input): Json<Request>) { input.path; }')
        self.write('src/backup.rs', 'fn last_result(data: &Path) { read(data); }')
        policy = {'expressions': [], 'backup_state': {'functions': ['last_result'], 'guards': [{'path': 'src/api.rs', 'contains': 'IMMUTABLE: bool = true'}], 'reason': 'Only immutable server state'}}
        result = finding('src/backup.rs', 1, 'rust/path-injection')
        state_flow = {'threadFlows': [{'locations': [{'location': location(path, line)} for path, line in [('src/api.rs', 1), ('src/api/backups.rs', 1), ('src/api/backups.rs', 2), ('src/backup.rs', 1)]]}]}
        result['codeFlows'] = [state_flow]
        self.assertEqual(len(self.apply([result], policy)[1]), 1)
        hostile = copy.deepcopy(result)
        hostile['codeFlows'].append({'threadFlows': [{'locations': [{'location': location('src/api/backups.rs', 4)}, {'location': location('src/backup.rs', 1)}]}]})
        self.assertEqual(len(self.apply([hostile], policy)[1]), 0)
        self.assertEqual(len(self.apply([finding('src/backup.rs', 1, 'rust/path-injection')], policy)[1]), 0)

    def test_source_paths_cannot_escape_checkout(self):
        sources = review.Sources(self.root)
        for path in ['../outside.rs', 'tests/../../secret.rs', '%2e%2e/secret.rs', '/etc/passwd', 'C:/secret.rs']:
            with self.subTest(path=path), self.assertRaises(ValueError):
                sources.path(path)

    def test_high_critical_unknown_and_extension_rules_block(self):
        result = finding('src/lib.rs', 1)
        for extension in [False, True]:
            doc = sarif([result], extension)
            self.assertEqual(len(review.blocking(doc)), 1)
            component = doc['runs'][0]['tool']['extensions'][0] if extension else doc['runs'][0]['tool']['driver']
            component['rules'][0]['properties']['security-severity'] = '5.0'
            self.assertEqual(len(review.blocking(doc)), 0)
            component['rules'][0]['properties']['security-severity'] = '7.5'
            self.assertEqual(len(review.blocking(doc)), 0)
            self.assertEqual(len(review.blocking(doc, 7.0)), 1)
            component['rules'][0]['properties']['security-severity'] = 'invalid'
            self.assertEqual(len(review.blocking(doc)), 1)
            component['rules'][0]['properties']['security-severity'] = 'NaN'
            self.assertEqual(len(review.blocking(doc)), 1)
        self.assertEqual(len(review.blocking(sarif([result]) | {'runs': [{'tool': {'driver': {'name': 'CodeQL'}}, 'results': [result]}]})), 1)

    def test_failed_and_low_quality_scans_do_not_pass_an_empty_gate(self):
        self.assertTrue(review.scan_errors({}))
        doc = sarif([])
        self.assertFalse(review.scan_errors(doc))
        doc['runs'][0]['invocations'] = [{'executionSuccessful': False}]
        self.assertTrue(review.scan_errors(doc))
        doc['runs'][0]['invocations'] = [{'toolExecutionNotifications': [{'descriptor': {'id': 'rust/diagnostic/database-quality'}, 'message': {'text': 'Unable to resolve dependencies'}}]}]
        self.assertTrue(review.scan_errors(doc))

    def test_cli_retains_original_sarif_and_blocks_an_unreviewed_secret(self):
        self.write('src/lib.rs', 'fn live() { use_password("runtime"); }')
        raw = self.root / 'raw.sarif'
        raw.write_text(json.dumps(sarif([finding('src/lib.rs', 1)], extension=True)), encoding='utf-8')
        original = raw.read_bytes()
        policy = self.root / 'policy.json'
        policy.write_text(json.dumps(EMPTY_POLICY), encoding='utf-8')
        output, report = self.root / 'reviewed.sarif', self.root / 'report.json'
        args = [sys.executable, str(SCRIPT), '--input', str(raw), '--output', str(output), '--report', str(report), '--source-root', str(self.root), '--policy', str(policy), '--gate']
        result = subprocess.run(args, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(raw.read_bytes(), original)
        self.assertEqual(json.loads(report.read_text(encoding='utf-8')), [])
        conflict = args.copy()
        conflict[conflict.index('--report') + 1] = str(raw)
        self.assertEqual(subprocess.run(conflict, capture_output=True).returncode, 2)
        self.assertEqual(raw.read_bytes(), original)
        self.assertEqual(subprocess.run([sys.executable, str(SCRIPT), '--input', str(output), '--gate-only'], capture_output=True).returncode, 1)


if __name__ == '__main__':
    unittest.main()
