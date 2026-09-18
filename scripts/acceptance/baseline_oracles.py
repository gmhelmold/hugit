#!/usr/bin/env python3
"""Read-only HUG-003 oracles over retained CLI observations.

Verification never executes commands, hooks, reducers, or text from a report.
Explicit --exercise mode runs only hard-coded private Linux fixtures using
the binary selected by the caller; it never runs commands supplied by a report.
Exit 0 verifies characterization, NOT product acceptance. --product-gate returns
1 for a reproduced regression; malformed/incomplete evidence returns 2.
The record is historical evidence for its selected executable, not a new run.
Static sources come from an explicit frozen snapshot, not the live checkout.
Verification only reads. --self-test creates and removes its own temporary fixtures.
"""
from __future__ import annotations
import argparse
import copy
import hashlib
import json
import os
import stat
import tempfile
from pathlib import Path
import re
import sys

FINDINGS = ('F03', 'F04', 'F06', 'F09', 'F10')
STATIC_FINDINGS = ('F01', 'F02', 'F05')
MAX_INPUT = 1024 * 1024
MAX_SOURCE = 1024 * 1024
STATIC_SOURCE_COMMIT = "ce2fcf1b8243eea2bd1be5aff25dfb3f6571ead4"
# Trusted historical inputs: never selected or weakened by the report.
STATIC_SOURCES = {'crates/hugit-cli/src/checks/run.rs': {'sha256': '09e444a498b63e80036df960e08b655a8f1c9fd9b755f9ccaad11959d5560979',
                                        'requires': ['captured_hermetic_env',
                                                     'run_memoized',
                                                     'ad-hoc check', 'Ok(Some(status)) => break Ok(status.code().unwrap_or(-1))', 'if poll_result.is_ok()', 'let _ = t.join();']},
 'docs/review/round13/wedge-decider.md': {'sha256': 'cf4f525fc3c4855e6b41b8b9fa2945a45b7aa9369555d666f3801086896a3659',
                                          'requires': ['finally DRY', 'OUTSIDE', 'memoized-CI']},
 'docs/plan/standalone/v3/work-packages/HUG-012.md': {'sha256': 'a379962763b1b24e4e8586d11403ad2a48ba28f76e5c8495af278c202350bf9c',
                                                      'requires': ['falso verde F01',
                                                                   'Worktree mutável não autoriza '
                                                                   'skip automático',
                                                                   'shell é reexecutado ou reuso '
                                                                   'negado']},
 'docs/plan/standalone/v3/work-packages/HUG-013.md': {'sha256': '07c143e579dae2fd77c5d646194e3958af2fc0eb431a7e381b468cd567e2725e',
                                                      'requires': ['shell tests, rede, relógio, '
                                                                   'randomness e bancos fora do '
                                                                   'allowlist',
                                                                   'Native cache validity pertence '
                                                                   'ao motor qualificado']},
 'crates/hugit-queue/src/core/union.rs': {'sha256': 'b074891b80002c1eeb2ce91e0446e4b92767b40d8c8d09214516a02115ca9e99',
                                          'requires': ['let (locus, extra_exec) = '
                                                       'bisect_failure(&ids, oracle);',
                                                       'let proceeding: Vec<String> = ids',
                                                       'pub fn outcomes_for_landing',
                                                       'UnionOutcome::Green',
                                                       'return '
                                                       '(FailureLocus::SingleItem(ids[i].to_string()), '
                                                       'executed);']},
 'docs/plan/standalone/v3/work-packages/HUG-014.md': {'sha256': 'dcadd6213e4b4f1ff4a327a30c7681faaf98b90736f1a2dee57e129d6238b4c5',
                                                      'requires': ['Após exclusão de locus, '
                                                                   'avaliar candidato restante '
                                                                   'inteiro',
                                                                   'A+B e C+D conflitantes não '
                                                                   'liberam C+D',
                                                                   'Remover culpado não prova '
                                                                   'resto verde.']}}
STATIC_POLICIES = {'F01': {'outcome': 'limit_justified',
         'correction_packages': ['HUG-012', 'HUG-013'],
         'sources': ['crates/hugit-cli/src/checks/run.rs',
                     'docs/review/round13/wedge-decider.md',
                     'docs/plan/standalone/v3/work-packages/HUG-012.md',
                     'docs/plan/standalone/v3/work-packages/HUG-013.md'],
         'disposition': 'static_limit_justified_checker_published',
         'evidence_scope': 'historical_source_snapshot_not_current_product_acceptance'},
 'F02': {'outcome': 'static_defect_observed',
         'correction_packages': ['HUG-014', 'HUG-043'],
         'sources': ['crates/hugit-queue/src/core/union.rs',
                     'docs/plan/standalone/v3/work-packages/HUG-014.md'],
         'disposition': 'static_defect_observed_checker_published',
         'evidence_scope': 'source_inspection_with_analytic_counterexample_not_runtime_execution'}}


STATIC_SOURCES.update({'docs/plan/standalone/v3/work-packages/HUG-009.md': {'requires': ['deadline completo',
                                                                   'Filho encerra antes de neto '
                                                                   'soltar pipe'],
                                                      'sha256': 'a8b44310b60741ad630ca43a218066394f8e04e128f23383df53ca08dad482e8'},
 'docs/plan/standalone/v3/work-packages/HUG-026.md': {'requires': ['neto com pipe aberto',
                                                                   'EOF verdadeiro completa'],
                                                      'sha256': '8a8749f812dfec632b3145ea34f96e73194685969ee8c23b62c292fe6dec52bb'}})
STATIC_POLICIES['F05'] = {'correction_packages': ['HUG-009', 'HUG-026'],
 'disposition': 'static_defect_observed_checker_published',
 'evidence_scope': 'source_inspection_deadline_excludes_successful_child_pipe_joins',
 'outcome': 'static_defect_observed',
 'sources': ['crates/hugit-cli/src/checks/run.rs',
             'docs/plan/standalone/v3/work-packages/HUG-009.md',
             'docs/plan/standalone/v3/work-packages/HUG-026.md']}

class EvidenceError(ValueError):
    pass

def need(condition, code):
    if not condition:
        raise EvidenceError(code)

def parse(raw):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            need(key not in result, 'duplicate_key')
            result[key] = value
        return result
    def nonfinite(_):
        raise EvidenceError('nonfinite_number')
    return json.loads(raw, object_pairs_hook=unique, parse_constant=nonfinite)

def reconstruct(report):
    need(type(report['schema_version']) is int and report['schema_version'] == 1
         and report['status'] == 'characterized', 'not_characterized')
    need(re.fullmatch('[0-9a-f]{64}', report['binary_sha256']) is not None, 'binary_identity')
    need(re.fullmatch('[0-9a-f]{40}', report['subject_claim']) is not None, 'subject_identity')
    need(report['product_accepted'] is False and report['whole_wp_ready'] is False, 'authority_escalation')
    commands = {}
    need(0 < len(report['commands']) <= 64, 'command_budget')
    for row in report['commands']:
        need(row['label'] not in commands, 'duplicate_command')
        need(type(row['exit']) is int, 'process_status')
        commands[row['label']] = row
    def payload(label, success=True):
        row = commands[label]
        need(not success or row['exit'] == 0, 'command_not_successful')
        data = parse(row['stdout'])
        need(isinstance(data, dict), 'stdout_projection_mismatch')
        return data
    files = report['persisted_files']
    log = parse(files['events.json'])
    goal_log = parse(files['goals.json'])
    cache = parse(files['checks.ac'])
    need(isinstance(log, list) and isinstance(goal_log, list) and isinstance(cache, dict), 'state_shape')
    checks = [parse(x['payload']) for x in log if x['kind'] == 'check.recorded']
    need(all(isinstance(c, dict) and type(c.get('cache_hit')) is bool for c in checks),
         'persisted_cache_hit_type')
    calls = [payload('cache-' + str(i)) for i in range(4)]
    key = calls[0]['memo_key']
    view = payload('check-show')
    key_rows = [x for x in checks if x.get('memo_key') == key]
    need(view['check_count'] == len(key_rows), 'check_count_not_persisted')
    need(view['kpis']['hits'] == sum(x.get('cache_hit') is True for x in key_rows), 'hit_count_not_persisted')
    pr = payload('pr-check-show')
    bound_rows = [x for x in key_rows if x.get('pr_id') == 'oracle-pr']
    need(pr.get('pr') == 'oracle-pr' and pr['check_count'] == len(bound_rows), 'pr_binding_not_persisted')
    fail = commands['failed-check']
    fail_json = payload('failed-check', success=False)
    need('exit 7' in fail['argv'], 'failure_command_drift')
    ids = []
    for i in range(4):
        identity = 'oracle-goal-' + str(i)
        value = payload(identity)
        need(value['intent_id'] == identity and value['already_exists'] is False, 'intent_not_created')
        ids.append(identity)
    persisted_ids = [parse(x['payload'])['intent_id'] for x in goal_log if x['kind'] == 'intent.landed']
    need(persisted_ids == ids, 'intent_identity_not_persisted')
    campaigns = payload('ledger')['campaigns']
    need(len(campaigns) == 1 and campaigns[0]['campaign'] == 'oracle', 'ledger_scope')
    out_key = payload('output-check')['memo_key']
    direct = commands['output-control']
    return dict(cache_calls=calls, execution_marker=files['repo/.git/oracle-marker'],
                check_show=view, pr_call=payload('new-pr-binding'), pr_show=pr,
                failed_check=dict(exit=fail['exit'], json=fail_json), intent_ids=ids,
                persisted_intent_count=len(persisted_ids),
                work_event_count=sum(x['kind'] not in ('campaign.opened', 'intent.landed') for x in goal_log),
                goal_campaign=campaigns[0], output_result=cache[out_key]['result'],
                output_marker=files['repo/.git/oracle-output-marker'],
                direct_output={k: direct[k] for k in ('stdout', 'stderr', 'exit')})

def verdict(expected, observed, satisfied):
    return dict(expected=expected, observed=observed,
                outcome='satisfied' if satisfied else 'regression_reproduced')

def evaluate(w):
    """Independent assertions over retained witnesses; never import Hugit code."""
    calls = w['cache_calls']
    need(len(calls) == 4 and all(type(c.get('cache_hit')) is bool for c in calls)
         and [c['cache_hit'] for c in calls] == [False, True, True, True],
         'cache_sequence_not_demonstrated')
    need(all(c.get('ok') is True and type(c.get('exit')) is int and c['exit'] == 0 for c in calls),
         'positive_check_not_demonstrated')
    keys = [c.get('memo_key') for c in calls]
    need(all(isinstance(k, str) and k for k in keys) and len(set(keys)) == 1, 'cache_key_changed')
    need(w['execution_marker'] == 'executed\n', 'physical_execution_not_demonstrated')
    show = w['check_show']
    need(type(show.get('check_count')) is int and isinstance(show.get('kpis'), dict), 'missing_check_projection')
    expected = dict(check_count=4, hits=3, hit_rate_pct=75.0)
    need(type(show['kpis'].get('hits')) is int
         and type(show['kpis'].get('hit_rate_pct')) in (int, float), 'missing_cache_metrics')
    seen = dict(check_count=show['check_count'], hits=show['kpis'].get('hits'),
                hit_rate_pct=show['kpis'].get('hit_rate_pct'))
    result = {'F03': verdict(expected, seen, expected == seen)}
    pr_call = w['pr_call']; pr = w['pr_show']
    need(pr_call.get('cache_hit') is True and pr_call.get('memo_key') == keys[0], 'pr_reuse_not_demonstrated')
    need(type(pr.get('check_count')) is int, 'missing_pr_projection')
    seen = dict(check_count=pr['check_count'], already_recorded=pr_call.get('already_recorded'))
    result['F04'] = verdict({'minimum_bound_checks': 1}, seen, pr['check_count'] >= 1)
    failure = w['failed_check']
    need(type(failure['json'].get('exit')) is int and failure['json']['exit'] == 7
         and failure['json'].get('ok') is False, 'failing_check_not_demonstrated')
    result['F06'] = verdict({'shell_gate_nonzero': True},
        {'process_exit': failure['exit'], 'payload_exit': 7, 'payload_ok': False}, failure['exit'] != 0)
    need(w['intent_ids'] == ['oracle-goal-0', 'oracle-goal-1', 'oracle-goal-2', 'oracle-goal-3']
         and w['persisted_intent_count'] == 4 and w['work_event_count'] == 0, 'goal_creation_not_demonstrated')
    c = w['goal_campaign']
    need(all(type(c.get(k)) is int for k in ('asked', 'done', 'proven')), 'missing_goal_projection')
    seen = {k: c[k] for k in ('asked', 'done', 'proven')}
    expected = dict(asked=4, done=0, proven=0)
    result['F09'] = verdict(expected, seen, expected == seen)
    output = w['output_result']
    need(w['direct_output'] == {'stdout': 'ORACLE_STDOUT\n', 'stderr': 'ORACLE_STDERR\n', 'exit': 0},
         'output_control_not_demonstrated')
    need(w['output_marker'] == 'executed\n' and type(output.get('exit')) is int and output['exit'] == 0,
         'output_execution_not_demonstrated')
    # Empty refs reproduce the baseline. A future populated ref is not accepted
    # until its resolver is qualified and the actual bytes are checked.
    refs = {k: output.get(k) for k in ('stdout_ref', 'stderr_ref', 'artifacts')}
    need(refs == dict(stdout_ref='', stderr_ref='', artifacts=[]), 'output_retention_changed_requires_byte_oracle')
    result['F10'] = verdict({'stdout_bytes': 'ORACLE_STDOUT\n', 'stderr_bytes': 'ORACLE_STDERR\n',
                            'resolvable_nonempty_refs': True}, refs, False)
    return result

def read_bounded_regular(path, limit):
    """Bounded cooperative-host read; this is not a hostile same-user sandbox."""
    before = path.lstat()
    need(stat.S_ISREG(before.st_mode) and not path.is_symlink(), 'source_type')
    need(before.st_size <= limit, 'source_budget')
    flags = os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0) | getattr(os, 'O_NONBLOCK', 0)
    flags |= getattr(os, 'O_BINARY', 0)
    with os.fdopen(os.open(path, flags), 'rb') as stream:
        opened = os.fstat(stream.fileno())
        need(stat.S_ISREG(opened.st_mode)
             and (opened.st_dev, opened.st_ino) == (before.st_dev, before.st_ino), 'source_type')
        raw = stream.read(limit + 1)
    need(len(raw) <= limit, 'source_budget')
    return raw

def read_static_source(root, relative):
    base = root.resolve(strict=True)
    need(base.is_dir(), 'source_root')
    current = base
    for part in Path(relative).parts[:-1]:
        current = current / part
        need(stat.S_ISDIR(current.lstat().st_mode) and not current.is_symlink(), 'source_type')
    return read_bounded_regular(base / relative, MAX_SOURCE)

def verify_static(report, root):
    static = report.get('static_findings')
    need(isinstance(static, dict) and set(static) == set(STATIC_FINDINGS), 'static_finding_set')
    for fid in STATIC_FINDINGS:
        row = static[fid]
        policy = STATIC_POLICIES[fid]
        need(isinstance(row, dict), 'static_shape')
        need(row.get('source_commit') == STATIC_SOURCE_COMMIT, 'static_source_commit')
        need(row.get('classification') == policy['outcome']
             and row.get('outcome') == policy['outcome'], 'static_classification')
        need(row.get('evidence_scope') == policy['evidence_scope'], 'static_evidence_scope')
        need(row.get('correction_packages') == policy['correction_packages'], 'static_owners')
        observed = row.get('observed')
        need(isinstance(observed, dict) and observed.get('product_fix_claimed') is False,
             'static_false_fix')
        limits = row.get('limits')
        need(isinstance(limits, list) and len(limits) >= 3
             and all(isinstance(x, str) and x.strip() for x in limits), 'static_limits')
        need(isinstance(row.get('expected'), str) and row['expected'].strip(), 'static_expected')
        evidence = row.get('evidence')
        need(isinstance(evidence, list) and len(evidence) == len(policy['sources'])
             and all(isinstance(x, dict) and isinstance(x.get('path'), str) for x in evidence),
             'static_evidence_set')
        # Check the entire authorized set before performing any source read.
        need({x['path'] for x in evidence} == set(policy['sources']), 'static_source_set')
        for item in evidence:
            path = item['path']
            trusted = STATIC_SOURCES[path]
            need(item.get('sha256') == trusted['sha256']
                 and item.get('requires') == trusted['requires'], 'static_contract_drift')
            raw = read_static_source(root, path)
            need(hashlib.sha256(raw).hexdigest() == trusted['sha256'], 'static_digest')
            text = raw.decode('utf-8')
            need(all(x in text for x in trusted['requires']), 'static_anchor')
        need(report['finding_registry'][fid]['disposition'] == policy['disposition'], 'static_registry')
    return static

def outcome_counts(results, static):
    counts = {k: sum(x['outcome'] == k for x in results.values())
              for k in ('satisfied', 'regression_reproduced')}
    for item in static.values():
        key = item['outcome']
        counts[key] = counts.get(key, 0) + 1
    return counts

def verify(report, root=Path('.')):
    need(isinstance(report, dict), 'report_shape')
    results = evaluate(reconstruct(report))
    static = verify_static(report, root)
    need(set(results) == set(FINDINGS)
         and json.dumps(results, sort_keys=True) == json.dumps(report['findings'], sort_keys=True),
         'forged_findings')
    need(set(report['finding_registry']) == {'F%02d' % i for i in range(1, 20)}, 'finding_omitted')
    need(report['origin']['workflow_head_sha'] == report['subject_claim'], 'workflow_subject_mismatch')
    need(report['origin']['binary_metadata']['sha256'] == report['binary_sha256'], 'binary_metadata_mismatch')
    pending = set(report['finding_registry']) - set(FINDINGS) - set(STATIC_FINDINGS)
    for fid, entry in report['finding_registry'].items():
        expected = ('pending_followup_in_HUG003' if fid in pending else
                    STATIC_POLICIES[fid]['disposition'] if fid in STATIC_FINDINGS else
                    'runtime_observed_locally_checker_published')
        need(isinstance(entry, dict) and entry.get('disposition') == expected, 'registry_disposition')
    counts = outcome_counts(results, static)
    need(json.dumps(report['counts'], sort_keys=True) == json.dumps(counts, sort_keys=True),
         'counts_mismatch')
    return results, static

def self_test(report, root):
    def command(r, name):
        return next(c for c in r['commands'] if c['label'] == name)
    def false_as_zero(r):
        row = command(r, 'cache-0')
        payload = parse(row['stdout']); payload['cache_hit'] = 0
        row['stdout'] = json.dumps(payload)
    mutations = {
        'false_as_zero': false_as_zero,
        'boolean_schema_version': lambda r: r.update(schema_version=True),
        'forged_green': lambda r: r['findings']['F09'].update(outcome='satisfied'),
        'missing_command': lambda r: r['commands'].pop(),
        'duplicate_command': lambda r: r['commands'].append(copy.deepcopy(r['commands'][0])),
        'missing_effect': lambda r: r['persisted_files'].update({'repo/.git/oracle-marker': ''}),
        'invented_goals': lambda r: r['persisted_files'].update({'goals.json': '[]'}),
        'missing_stdout': lambda r: command(r, 'cache-0').update(stdout=''),
        'forged_stdout': lambda r: command(r, 'check-show').update(stdout='{"check_count":4}'),
        'lost_finding': lambda r: r['finding_registry'].pop('F01'),
        'false_authority': lambda r: r.update(product_accepted=True),
        'wrong_executable': lambda r: r.update(binary_sha256='0'*64),
    }
    outcomes = {}
    verify(report, root)
    mutations.update({
        'lost_static_finding': lambda r: r['static_findings'].pop('F01'),
        'false_static_fix': lambda r: r['static_findings']['F01']['observed'].update(product_fix_claimed=True),
        'wrong_static_digest': lambda r: r['static_findings']['F01']['evidence'][0].update(sha256='0'*64),
        'weakened_static_anchor': lambda r: r['static_findings']['F01']['evidence'][0].update(requires=['fn']),
        'substituted_static_source': lambda r: r['static_findings']['F01']['evidence'][0].update(path='README.md'),
        'wrong_static_commit': lambda r: r['static_findings']['F01'].update(source_commit='0'*40),
        'empty_static_limits': lambda r: r['static_findings']['F01'].update(limits=['', '', '']),
        'malformed_static_row': lambda r: r['static_findings'].update(F01=[]),
        'promoted_pending_finding': lambda r: r['finding_registry']['F05'].update(disposition='satisfied'),
        'forged_counts': lambda r: r['counts'].update(satisfied=19),
        'omitted_F02_inspection': lambda r: r['static_findings'].pop('F02'),
        'F02_false_fix': lambda r: r['static_findings']['F02']['observed'].update(product_fix_claimed=True),
        'F02_false_runtime_claim': lambda r: r['static_findings']['F02'].update(evidence_scope='runtime_reproduced'),
        'F02_unverified_reclassification': lambda r: r['static_findings']['F02'].update(outcome='limit_justified'),
        'omitted_F05_inspection': lambda r: r['static_findings'].pop('F05'),
        'F05_false_fix': lambda r: r['static_findings']['F05']['observed'].update(product_fix_claimed=True),
        'F05_false_runtime_claim': lambda r: r['static_findings']['F05'].update(evidence_scope='runtime_reproduced'),
    })
    for name, mutate in mutations.items():
        altered = copy.deepcopy(report); mutate(altered)
        try:
            verify(altered, root)
        except (EvidenceError, KeyError, ValueError, TypeError, IndexError, RecursionError):
            outcomes[name] = 'rejected'
        else:
            raise EvidenceError('mutation_accepted:' + name)
    outcomes.update(source_self_test(report, root))
    return outcomes

def source_self_test(report, root):
    """Only synthetic private fixtures are mutated; the supplied snapshot is untouched."""
    originals = {path: read_static_source(root, path) for path in STATIC_SOURCES}
    outcomes = {}
    def refused(name, action, code):
        try:
            action()
        except EvidenceError as error:
            need(str(error) == code, 'wrong_refusal:' + name)
            outcomes[name] = 'rejected'
        else:
            raise EvidenceError('mutation_accepted:' + name)
    with tempfile.TemporaryDirectory(prefix='hugit-baseline-source-test-') as owned:
        base = Path(owned) / 'snapshot'
        for path, raw in originals.items():
            target = base / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(raw)
        verify(report, base)
        outcomes['frozen_snapshot_positive'] = 'passed'
        relative = next(iter(STATIC_SOURCES))
        target = base / relative
        target.write_bytes(originals[relative] + b'\nchanged fixture\n')
        refused('changed_source_bytes', lambda: verify(report, base), 'static_digest')
        target.write_bytes(originals[relative])
        # Enforce the budget through the same reader, using a tiny benign file.
        budget = Path(owned) / 'budget.txt'
        budget.write_bytes(b'12345')
        refused('bounded_source_read', lambda: read_bounded_regular(budget, 4), 'source_budget')
        refused('nonregular_source', lambda: read_bounded_regular(base, MAX_SOURCE), 'source_type')
        # A separate evolving checkout cannot alter the retained source snapshot.
        current = Path(owned) / 'current-checkout'
        current.mkdir()
        (current / 'edited.txt').write_text('new product code, not historical evidence')
        verify(report, base)
        outcomes['separate_snapshot_preserved'] = 'passed'
        if os.name == 'posix':
            target.unlink()
            target.symlink_to(budget)
            refused('source_symlink', lambda: verify(report, base), 'source_type')
            target.unlink(); target.write_bytes(originals[relative])
            parent = target.parent
            saved = parent.with_name('saved-source-dir')
            parent.rename(saved); parent.symlink_to(saved, target_is_directory=True)
            refused('source_parent_symlink', lambda: verify(report, base), 'source_type')
            parent.unlink(); saved.rename(parent)
        else:
            outcomes['source_symlink'] = 'not_run_on_non_posix'
            outcomes['source_parent_symlink'] = 'not_run_on_non_posix'
        verify(report, base)
    need(not Path(owned).exists(), 'owned_fixture_cleanup')
    need(all(read_static_source(root, path) == raw for path, raw in originals.items()),
         'supplied_snapshot_changed')
    outcomes['owned_fixture_cleanup'] = 'passed'
    outcomes['supplied_snapshot_unchanged'] = 'passed'
    return outcomes

# Explicit execution mode is separate from all read-only verification paths.
RUNTIME_SCHEMA = 'hugit.baseline-runtime/1'
RUNTIME_LABELS = ('version', 'git-init', 'git-root', 'health-before', 'attach',
                  'git-add', 'git-commit', 'git-head', 'health-after',
                  'read-valid', 'read-tampered', 'git-status', 'deadline')
HOLDER = '''import os, pathlib, time
root = pathlib.Path(__file__).parent
r, w = os.pipe()
pid = os.fork()
if pid == 0:
    os.close(r)
    (root / 'holder-ready').write_text('ready')
    os.write(w, b'R'); os.close(w)
    until = time.monotonic() + 6
    while not (root / 'release').exists() and time.monotonic() < until:
        time.sleep(0.01)
    (root / 'holder-finished').write_text('finished')
    os._exit(0)
os.close(w)
assert os.read(r, 1) == b'R'
os.close(r)
(root / 'parent-exiting').write_text('ready')
os._exit(0)
'''

def observed_command(argv, cwd, env, label, records, tick=None):
    """Fixed fixture commands only. Bounded pipes/time; never execute report text."""
    import selectors
    import subprocess
    import time
    start = time.monotonic()
    output = [bytearray(), bytearray()]
    row = dict(label=label, argv=[str(x) for x in argv])
    records.append(row)
    with subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE) as child:
        try:
            with selectors.DefaultSelector() as sel:
                for i, stream in enumerate((child.stdout, child.stderr)):
                    os.set_blocking(stream.fileno(), False)
                    sel.register(stream, selectors.EVENT_READ, i)
                while sel.get_map() or child.poll() is None:
                    elapsed = time.monotonic() - start
                    need(elapsed < 12, 'fixture_command_timeout:' + label)
                    if tick:
                        tick(elapsed, child.poll())
                    for key, _ in sel.select(0.02):
                        data = os.read(key.fileobj.fileno(), 65536)
                        if not data:
                            sel.unregister(key.fileobj)
                        else:
                            need(len(output[key.data]) + len(data) <= 512 * 1024,
                                 'fixture_output_budget:' + label)
                            output[key.data].extend(data)
                row['exit'] = child.wait(timeout=1)
        finally:
            if child.poll() is None:
                child.kill()  # Only the directly owned process, never a group.
                child.wait(timeout=2)
            row.update(elapsed_ms=round((time.monotonic() - start) * 1000, 3),
                       stdout=output[0].decode('utf-8', errors='replace'),
                       stderr=output[1].decode('utf-8', errors='replace'))
    return row

def runtime_json(row):
    data = parse(row['stdout'])
    need(isinstance(data, dict), 'runtime_json_shape')
    return data

def verify_runtime(report, subject):
    """Reconstruct observed assertions, not merely stored outcome flags."""
    need(isinstance(report, dict) and report.get('schema') == RUNTIME_SCHEMA, 'runtime_schema')
    need(report.get('status') == 'characterized' and report.get('subject_claim') == subject
         and re.fullmatch('[0-9a-f]{40,64}', subject) is not None, 'runtime_subject')
    need(report.get('product_accepted') is False and report.get('whole_wp_ready') is False,
         'runtime_authority')
    binary = report['binary']
    need(re.fullmatch('[0-9a-f]{64}', binary['sha256']) is not None
         and binary['sha256'] == binary['copy_sha256'] == binary['after_sha256'], 'runtime_binary')
    need(report['environment']['system'] == 'Linux', 'runtime_cell')
    rows = report['commands']
    need(isinstance(rows, list) and len(rows) == len(RUNTIME_LABELS), 'runtime_commands')
    need([r['label'] for r in rows] == list(RUNTIME_LABELS), 'runtime_command_set')
    commands = {r['label']: r for r in rows}
    need(all(type(r['exit']) is int and isinstance(r['stdout'], str)
             and isinstance(r['stderr'], str) and type(r['elapsed_ms']) in (int, float)
             and 0 <= r['elapsed_ms'] < 12000 for r in rows), 'runtime_command_shape')
    need(commands['version']['stdout'].strip() == binary['version']
         and binary['version'].startswith('hugit '), 'runtime_version')
    need(all(commands[n]['exit'] == 0 for n in RUNTIME_LABELS if n != 'read-tampered'),
         'runtime_command_failure')
    need(runtime_json(commands['health-before'])['mode'] == 'inactive'
         and runtime_json(commands['attach'])['attached'] is True, 'runtime_setup')
    after = runtime_json(commands['health-after'])
    need(after['mode'] == 'active' and after['log']['state'] == 'valid', 'runtime_capture_health')
    oid = commands['git-head']['stdout'].strip()
    need(re.fullmatch('[0-9a-f]{40}|[0-9a-f]{64}', oid) is not None, 'runtime_commit_oid')
    log = parse(report['event_log'])
    need(isinstance(log, list) and 0 < len(log) < 100, 'runtime_log_shape')
    matching = [i for i, e in enumerate(log) if e['kind'] == 'ref.update'
                and parse(e['payload']).get('target') == oid
                and parse(e['payload']).get('ref') == 'refs/heads/main']
    need(bool(matching), 'runtime_commit_not_captured')
    need(not any(e['kind'].startswith(('intent.', 'goal.')) for e in log), 'runtime_invented_goal')
    tampered = copy.deepcopy(log)
    payload = parse(tampered[matching[0]]['payload'])
    payload['target'] = '0' * len(oid)
    tampered[matching[0]]['payload'] = json.dumps(payload, sort_keys=True, separators=(',', ':'))
    need(parse(report['tampered_log']) == tampered, 'runtime_tamper_not_partial')
    error = runtime_json(commands['read-tampered'])
    need(commands['read-tampered']['exit'] == 2
         and error.get('error', {}).get('kind') == 'chain_broken', 'runtime_tamper_not_refused')
    need(commands['git-status']['stdout'] == '', 'runtime_worktree_changed')
    # Match the fixture specification, not a report-selected deadline or outcome.
    deadline = commands['deadline']
    result = runtime_json(deadline)
    need('--timeout-secs' in deadline['argv']
         and deadline['argv'][deadline['argv'].index('--timeout-secs') + 1] == '1', 'runtime_deadline_drift')
    need(result.get('cache_hit') is False and result.get('local_executions') == 1
         and result.get('ok') is True and type(result.get('exit')) is int
         and result['exit'] == 0, 'runtime_deadline_behavior_changed_requires_review')
    need(report['barrier'] == {'parent_exiting': 'ready', 'holder_ready': 'ready',
                              'holder_finished': 'finished', 'release_after_ms': 3000}, 'runtime_barrier')
    need(2500 <= deadline['elapsed_ms'] < 10000, 'runtime_deadline_not_reproduced')
    need(report['owned_fixture_removed'] is True, 'runtime_cleanup')
    need(report.get('helper_sha256') == hashlib.sha256(HOLDER.encode()).hexdigest(), 'runtime_helper')
    return {'F05': {'expected': {'timeout_secs': 1, 'end_to_end_ceiling_ms': 1500},
                    'observed': {'elapsed_ms': deadline['elapsed_ms'], 'process_exit': deadline['exit'],
                                 'payload_exit': result['exit'], 'payload_ok': result['ok']},
                    'outcome': 'regression_reproduced'},
            'automatic_commit_capture': {'oid': oid, 'outcome': 'satisfied'},
            'partial_tamper_refusal': {'exit': 2, 'error': 'chain_broken', 'outcome': 'satisfied'}}

def _exercise_runtime(binary, subject, git_override, records):
    """Linux-only finite fixture. Changes exclusively its own TemporaryDirectory."""
    import platform
    import shutil
    import time
    need(platform.system() == 'Linux', 'runtime_requires_linux')
    need(isinstance(subject, str) and re.fullmatch('[0-9a-f]{40,64}', subject), 'runtime_subject')
    binary = binary.resolve(strict=True)
    binary_bytes = read_bounded_regular(binary, 128 * 1024 * 1024)
    digest = hashlib.sha256(binary_bytes).hexdigest()
    git = git_override or shutil.which('git', path=os.defpath)
    need(git is not None, 'runtime_git_missing')
    with tempfile.TemporaryDirectory(prefix='hugit-baseline-runtime-') as owned:
        root = Path(owned)
        repo = root / 'repo'; repo.mkdir()
        home = root / 'home'; home.mkdir()
        template = root / 'empty-template'; template.mkdir()
        bindir = root / 'bin'; bindir.mkdir()
        executable = bindir / 'hugit'; executable.write_bytes(binary_bytes); executable.chmod(0o700)
        env = {'PATH': str(bindir) + os.pathsep + os.defpath, 'HOME': str(home),
               'XDG_CONFIG_HOME': str(home / 'xdg'), 'GIT_CONFIG_NOSYSTEM': '1',
               'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_TEMPLATE_DIR': str(template),
               'GIT_TERMINAL_PROMPT': '0', 'LC_ALL': 'C', 'TZ': 'UTC',
               'GIT_AUTHOR_NAME': 'Baseline Fixture', 'GIT_AUTHOR_EMAIL': 'fixture@example.invalid',
               'GIT_COMMITTER_NAME': 'Baseline Fixture', 'GIT_COMMITTER_EMAIL': 'fixture@example.invalid',
               'GIT_AUTHOR_DATE': '2000-01-01T00:00:00Z', 'GIT_COMMITTER_DATE': '2000-01-01T00:00:00Z'}
        def run(label, args, tick=None):
            return observed_command([str(x) for x in args], repo, env, label, records, tick)
        def ok(label, args):
            r = run(label, args); need(r['exit'] == 0, 'runtime_setup:' + label); return r
        version = ok('version', [executable, '--version'])['stdout'].strip()
        need(version.startswith('hugit '), 'runtime_version_missing')
        ok('git-init', [git, 'init', '-b', 'main'])
        located = ok('git-root', [git, 'rev-parse', '--show-toplevel'])['stdout'].strip()
        need(located == str(repo.resolve()), 'runtime_setup_git_root')
        need(runtime_json(ok('health-before', [executable, 'health']))['mode'] == 'inactive', 'runtime_setup_health')
        ok('attach', [executable, 'attach'])
        manifest = repo / '.git/hugit-runtime/hooks-v1/manifest.json'
        need(manifest.exists(), 'runtime_missing_hook_manifest')
        read_bounded_regular(manifest, MAX_INPUT)  # Always-zero/JSON-only substitutes cannot pass.
        (repo / 'fixture.txt').write_text('controlled baseline commit\n')
        ok('git-add', [git, 'add', 'fixture.txt'])
        ok('git-commit', [git, 'commit', '-m', 'baseline automatic capture fixture'])
        oid = ok('git-head', [git, 'rev-parse', 'HEAD'])['stdout'].strip()
        logpath = repo / '.git/hugit/event-log.json'
        until = time.monotonic() + 10
        while True:
            raw = read_bounded_regular(logpath, MAX_INPUT) if logpath.exists() else b'[]'
            events = parse(raw)
            if any(e['kind'] == 'ref.update' and parse(e['payload']).get('target') == oid for e in events):
                break
            need(time.monotonic() < until, 'runtime_capture_not_observed')
            time.sleep(0.02)  # Predicate polling, not a sleep-only synchronization assumption.
        ok('health-after', [executable, 'health'])
        good = root / 'valid.json'; good.write_bytes(raw)
        tampered = copy.deepcopy(events)
        index = next(i for i, e in enumerate(events) if e['kind'] == 'ref.update'
                     and parse(e['payload']).get('target') == oid)
        payload = parse(tampered[index]['payload']); payload['target'] = '0' * len(oid)
        tampered[index]['payload'] = json.dumps(payload, sort_keys=True, separators=(',', ':'))
        badraw = json.dumps(tampered, sort_keys=True).encode()
        bad = root / 'tampered.json'; bad.write_bytes(badraw)
        ok('read-valid', [executable, 'check', 'show', '--log', good])
        run('read-tampered', [executable, 'check', 'show', '--log', bad])
        need(good.read_bytes() == raw and bad.read_bytes() == badraw, 'runtime_reader_mutated_copy')
        ok('git-status', [git, 'status', '--porcelain=v1', '--untracked-files=all'])
        helper = repo / '.git/pipe-fixture'; helper.mkdir()
        helperfile = helper / 'holder.py'; helperfile.write_text(HOLDER)
        (root / 'deadline-log.json').write_text('[]\n')
        ready_at = [None]
        def release(elapsed, status):
            if (helper / 'holder-ready').exists() and ready_at[0] is None:
                ready_at[0] = elapsed
            if ready_at[0] is not None and elapsed - ready_at[0] >= 3:
                (helper / 'release').touch(exist_ok=True)
        import shlex
        command = 'exec ' + shlex.quote(sys.executable) + ' ' + shlex.quote(str(helperfile))
        try:
            run('deadline', [executable, 'check', 'run', '--def', 'deadline-fixture',
                            '--toolchain', 'baseline-fixture-v1', '--root', repo,
                            '--log', root / 'deadline-log.json', '--ac', root / 'deadline.ac',
                            '--timeout-secs', '1', '--cmd', command], release)
        finally:
            (helper / 'release').touch(exist_ok=True)
            # The child is finite (6 s maximum) and always receives release on error.
            until = time.monotonic() + 7
            while (helper / 'holder-ready').exists() and not (helper / 'holder-finished').exists():
                need(time.monotonic() < until, 'runtime_holder_cleanup_unconfirmed')
                time.sleep(0.02)
        result = dict(schema=RUNTIME_SCHEMA, status='characterized', subject_claim=subject,
                      product_accepted=False, whole_wp_ready=False,
                      binary=dict(sha256=digest, copy_sha256=hashlib.sha256(executable.read_bytes()).hexdigest(),
                                  version=version, source_to_binary_binding='caller_selected_executable; CI build identity recorded externally'),
                      environment=dict(system=platform.system(), release=platform.release(),
                                       machine=platform.machine(), python=platform.python_version(),
                                       inherited_environment=False, network_prohibition_instrumented=False),
                      commands=records, event_log=raw.decode(), tampered_log=badraw.decode(),
                      helper_sha256=hashlib.sha256(HOLDER.encode()).hexdigest(),
                      barrier={name.replace('-', '_'): (helper / name).read_text()
                               for name in ('parent-exiting', 'holder-ready', 'holder-finished')})
        result['barrier']['release_after_ms'] = 3000
        # Synthetic-only paths are retained: rewriting log bytes would change their hashes.
        result['fixture_root'] = str(root)
    result['owned_fixture_removed'] = not root.exists()
    result['binary']['after_sha256'] = hashlib.sha256(read_bounded_regular(binary, 128 * 1024 * 1024)).hexdigest()
    result['assertions'] = verify_runtime(result, subject)
    return result

class RuntimeFixtureError(EvidenceError):
    def __init__(self, code, records):
        super().__init__(code)
        self.records = records

def exercise_runtime(binary, subject, git_override=None):
    import subprocess
    records = []
    try:
        return _exercise_runtime(binary, subject, git_override, records)
    except (EvidenceError, OSError, ValueError, KeyError, TypeError, IndexError,
            subprocess.SubprocessError) as error:
        code = str(error)[:120] if isinstance(error, EvidenceError) else 'runtime_invalid_or_missing_evidence'
        raise RuntimeFixtureError(code, records) from error

def check_runtime_document(report, subject):
    assertions = verify_runtime(report, subject)
    need(report.get('assertions') == assertions, 'runtime_forged_assertions')
    return assertions

def runtime_self_test(binary, subject, result):
    outcomes = {}
    for name, change in {
        'runtime_forged_verdict': lambda r: r['assertions']['F05'].update(outcome='satisfied'),
        'runtime_false_green': lambda r: r.update(product_accepted=True),
        'runtime_lost_command': lambda r: r['commands'].pop(),
        'runtime_wrong_oid': lambda r: r['commands'][7].update(stdout='0'*40+'\n'),
        'runtime_not_partial': lambda r: r.update(tampered_log=r['event_log']),
        'runtime_omitted_log': lambda r: r.update(event_log='[]'),
        'runtime_cleanup_missing': lambda r: r.update(owned_fixture_removed=False),
        'runtime_deadline_weakened': lambda r: r['commands'][-1]['argv'].__setitem__(
            r['commands'][-1]['argv'].index('--timeout-secs')+1, '300'),
        'runtime_unobserved_barrier': lambda r: r['barrier'].update(holder_ready=''),
    }.items():
        altered = copy.deepcopy(result); change(altered)
        try:
            check_runtime_document(altered, subject)
        except (EvidenceError, KeyError, ValueError, TypeError, IndexError):
            outcomes[name] = 'rejected'
        else:
            raise EvidenceError('runtime_mutation_accepted:' + name)
    with tempfile.TemporaryDirectory(prefix='hugit-baseline-substitute-') as tmp:
        path = Path(tmp) / 'fake-hugit'
        for name, content in {
            'always_zero_executable': '#!/bin/sh\nexit 0\n',
            'invented_json_no_effects': '#!/bin/sh\ncase "$1" in\n--version) echo "hugit fake";;\nhealth) echo \'{"mode":"inactive"}\';;\nattach) echo \'{"attached":true}\';;\nesac\n',
        }.items():
            path.write_text(content); path.chmod(0o700)
            try:
                exercise_runtime(path, subject)
            except RuntimeFixtureError as error:
                expected = ('runtime_version_missing' if name == 'always_zero_executable'
                            else 'runtime_missing_hook_manifest')
                need(str(error) == expected, 'wrong_substitute_refusal:' + name)
                outcomes[name] = {'outcome': 'rejected', 'error': str(error), 'commands': error.records}
            else:
                raise EvidenceError('runtime_substitute_accepted:' + name)
        path.write_text('#!/bin/sh\nexit 0\n')
        try:
            exercise_runtime(binary, subject, git_override=str(path))
        except RuntimeFixtureError as error:
            need(str(error) == 'runtime_setup_git_root', 'wrong_setup_refusal')
            outcomes['malformed_git_setup'] = {'outcome': 'rejected', 'error': str(error), 'commands': error.records}
        else:
            raise EvidenceError('runtime_setup_accepted')
    need(not Path(tmp).exists(), 'runtime_substitute_cleanup')
    outcomes['substitute_fixture_cleanup'] = 'passed'
    return outcomes

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    modes = ap.add_mutually_exclusive_group(required=True)
    modes.add_argument('--report', type=Path)
    modes.add_argument('--exercise', action='store_true')
    modes.add_argument('--runtime-report', type=Path)
    ap.add_argument('--hugit-bin', type=Path)
    ap.add_argument('--subject')
    ap.add_argument('--root', type=Path, default=Path('.'),
                    help='source snapshot at ' + STATIC_SOURCE_COMMIT + '; not the evolving checkout')
    ap.add_argument('--self-test', action='store_true')
    ap.add_argument('--product-gate', action='store_true')
    args = ap.parse_args()
    try:
        if args.exercise:
            need(args.hugit_bin is not None and args.subject is not None, 'runtime_arguments')
            result = exercise_runtime(args.hugit_bin, args.subject)
            if args.self_test:
                result['controls'] = runtime_self_test(args.hugit_bin, args.subject, result)
            print(json.dumps(result, sort_keys=True))
            return 1 if args.product_gate else 0
        if args.runtime_report is not None:
            raw = read_bounded_regular(args.runtime_report, MAX_INPUT)
            assertions = check_runtime_document(parse(raw), args.subject)
            print(json.dumps(dict(evidence_consistent=True, product_accepted=False,
                whole_wp_ready=False, report_sha256=hashlib.sha256(raw).hexdigest(),
                assertions=assertions, scope='read_only_runtime_report_verification'), sort_keys=True))
            return 1 if args.product_gate else 0
        raw = read_bounded_regular(args.report, MAX_INPUT)
        report = parse(raw)
        results, static = verify(report, args.root)
        tests = self_test(report, args.root) if args.self_test else {}
        counts = outcome_counts(results, static)
        print(json.dumps(dict(evidence_consistent=True, product_accepted=False,
            whole_wp_ready=False, report_sha256=hashlib.sha256(raw).hexdigest(),
            findings=results, static_source_commit=STATIC_SOURCE_COMMIT,
            static_findings=static, counts=counts,
            pending_findings=sorted(set(report['finding_registry']) - set(FINDINGS) - set(STATIC_FINDINGS)),
            controls=tests,
            scope='retained_runtime_observations_and_static_inspection; no product executed'), sort_keys=True))
        return 1 if args.product_gate and counts['regression_reproduced'] else 0
    except RuntimeFixtureError as error:
        print(json.dumps(dict(status='setup_or_observation_error', product_accepted=False,
            whole_wp_ready=False, error=str(error), commands=error.records), sort_keys=True))
        return 2
    except (OSError, EvidenceError, KeyError, ValueError, TypeError, IndexError, AttributeError, RecursionError) as error:
        code = str(error)[:120] if isinstance(error, EvidenceError) else 'invalid_or_missing_evidence'
        print(json.dumps(dict(evidence_consistent=False, product_accepted=False, error=code)))
        return 2

if __name__ == '__main__':
    sys.exit(main())
