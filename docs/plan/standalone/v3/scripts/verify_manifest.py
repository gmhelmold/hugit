#!/usr/bin/env python3
"""Verify package bytes against its local inventory. Not a signature/authenticity proof."""
from __future__ import annotations
import argparse, hashlib, json, sys
from pathlib import Path, PurePosixPath

def main() -> int:
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--root',type=Path,default=Path(__file__).resolve().parents[1])
    args=ap.parse_args();root=args.root.resolve();errors=[]
    try:
        manifest=json.loads((root/'manifest-sha256.json').read_text())
        entries=manifest['files'];seen=set()
        for item in entries:
            name=item['path'];rel=PurePosixPath(name)
            if rel.is_absolute() or '..' in rel.parts or '\\' in name or name in seen:
                errors.append({'path':name,'error':'invalid_or_duplicate_path'});continue
            seen.add(name);f=root.joinpath(*rel.parts)
            if f.is_symlink() or any(x.is_symlink() for x in f.parents if x!=root.parent):
                errors.append({'path':name,'error':'symlink_not_admitted'});continue
            if not f.is_file():errors.append({'path':name,'error':'missing'});continue
            h=hashlib.sha256();size=0
            with f.open('rb') as stream:
                while chunk:=stream.read(1024*1024):h.update(chunk);size+=len(chunk)
            if h.hexdigest()!=item['sha256'] or size!=item['bytes']:
                errors.append({'path':name,'error':'bytes_mismatch'})
        actual={f.relative_to(root).as_posix() for f in root.rglob('*') if f.is_file() and '__pycache__' not in f.parts and f.name!='manifest-sha256.json'}
        for name in sorted(actual-seen):errors.append({'path':name,'error':'not_in_manifest'})
        result={'scope':'byte_inventory_not_authenticity_or_product_quality','checked':len(entries),'valid':not errors,'errors':errors}
    except (OSError,ValueError,KeyError,TypeError) as e:
        result={'valid':False,'errors':[{'error':'manifest_input_error','detail':str(e)}]}
    print(json.dumps(result,ensure_ascii=False,indent=2))
    return 0 if result['valid'] else 1
if __name__=='__main__':sys.exit(main())
