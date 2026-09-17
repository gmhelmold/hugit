#!/usr/bin/env python3
"""Validate the unchanged normative v3 import; never claims runtime qualification.
Historical nested ZIPs remain external archival inputs, not normative documents.
"""
from pathlib import Path
import hashlib, importlib.util, json, sys
ROOT=Path(__file__).resolve().parents[2]
PLAN=ROOT/'docs/plan/standalone/v3'
sys.path.insert(0,str(PLAN/'scripts'))
import validate_plan, render_plan

def main():
 p=json.loads((PLAN/'backlog.json').read_text())
 result=validate_plan.validate(p)
 errors=list(result['errors'])
 for name,expected in render_plan.all_views(p).items():
  path=PLAN/name
  if not path.is_file() or path.read_text()!=expected:
   errors.append({'code':'VIEW_DRIFT','message':name})
 metadata=json.loads((PLAN.parent/'IMPORT-MANIFEST.json').read_text())
 actual=hashlib.sha256((PLAN/'backlog.json').read_bytes()).hexdigest()
 if actual!=metadata['canonical_source_file_sha256']:
  errors.append({'code':'APPROVED_SOURCE_CHANGED','message':'Update the import manifest only through reviewed plan changes.'})
 for name,digest in metadata['original_scripts_sha256'].items():
  path=PLAN/'scripts'/name
  if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest()!=digest:
   errors.append({'code':'ORIGINAL_VALIDATOR_CHANGED','message':name})
 print(json.dumps({'scope':'normative_plan_and_import_only','valid':not errors,'errors':errors,
  'historical_archives':'external; not claimed locally verified',
  'product_qualification':'not_run','work_packages':len(p['tasks'])},ensure_ascii=False,indent=2))
 return int(bool(errors))
if __name__=='__main__':raise SystemExit(main())
