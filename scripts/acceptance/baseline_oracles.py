#!/usr/bin/env python3
"""Read-only HUG-003 oracles over retained CLI observations.

This program never runs a command, reducer, hook, or content from the input.
Exit 0 verifies characterization, NOT product acceptance. --product-gate returns
1 for a reproduced regression; malformed/incomplete evidence returns 2.
The record is historical evidence for its selected executable, not a new run.
"""
from __future__ import annotations
import argparse
import copy
import hashlib
import json
from pathlib import Path
import re
import sys

FINDINGS = ('F03', 'F04', 'F06', 'F09', 'F10')
MAX_INPUT = 1024 * 1024

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
    need(report['schema_version'] == 1 and report['status'] == 'characterized', 'not_characterized')
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
    need(len(calls) == 4 and [c.get('cache_hit') for c in calls] == [False, True, True, True],
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

def verify(report):
    results = evaluate(reconstruct(report))
    need(set(results) == set(FINDINGS) and results == report['findings'], 'forged_findings')
    need(set(report['finding_registry']) == {'F%02d' % i for i in range(1, 20)}, 'finding_omitted')
    need(report['origin']['workflow_head_sha'] == report['subject_claim'], 'workflow_subject_mismatch')
    need(report['origin']['binary_metadata']['sha256'] == report['binary_sha256'], 'binary_metadata_mismatch')
    return results

def self_test(report):
    def command(r, name):
        return next(c for c in r['commands'] if c['label'] == name)
    mutations = {
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
    verify(report)
    for name, mutate in mutations.items():
        altered = copy.deepcopy(report); mutate(altered)
        try:
            verify(altered)
        except (EvidenceError, KeyError, ValueError, TypeError, IndexError, RecursionError):
            outcomes[name] = 'rejected'
        else:
            raise EvidenceError('mutation_accepted:' + name)
    return outcomes

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--report', type=Path, required=True)
    ap.add_argument('--self-test', action='store_true')
    ap.add_argument('--product-gate', action='store_true')
    args = ap.parse_args()
    try:
        with args.report.open('rb') as stream:
            raw = stream.read(MAX_INPUT + 1)
        need(len(raw) <= MAX_INPUT, 'input_budget')
        report = parse(raw)
        results = verify(report)
        tests = self_test(report) if args.self_test else {}
        counts = {k: sum(x['outcome'] == k for x in results.values())
                  for k in ('satisfied', 'regression_reproduced')}
        print(json.dumps(dict(evidence_consistent=True, product_accepted=False,
            whole_wp_ready=False, report_sha256=hashlib.sha256(raw).hexdigest(),
            findings=results, counts=counts, controls=tests,
            scope='retained_observations_only; no product executed'), sort_keys=True))
        return 1 if args.product_gate and counts['regression_reproduced'] else 0
    except (OSError, EvidenceError, KeyError, ValueError, TypeError, IndexError, RecursionError) as error:
        code = str(error)[:120] if isinstance(error, EvidenceError) else 'invalid_or_missing_evidence'
        print(json.dumps(dict(evidence_consistent=False, product_accepted=False, error=code)))
        return 2

if __name__ == '__main__':
    sys.exit(main())
