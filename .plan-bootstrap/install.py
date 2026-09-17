"""One-shot transport for the user-approved Hugit standalone v3 plan."""
import hashlib
import importlib.util
import json
import lzma
import pathlib
import sys

PAYLOAD_SHA = '81afb0ff78002edbdda0b86e168bf00b9d85460db928ff5414711a83c553e852'
SOURCE_SHA = '5234276523649da9b35abe578ed6cbf2ce25c43651bfc0f6ab2d549b37e01409'
EXACT_PATHS = {
    '.github/PULL_REQUEST_TEMPLATE/standalone-work-package.md',
    '.github/ISSUE_TEMPLATE/standalone-plan-change.yml',
}

def write(root, relative, text):
    p = pathlib.PurePosixPath(relative)
    if p.is_absolute() or '..' in p.parts or '\\' in relative:
        raise ValueError('unsafe path')
    target = root.joinpath(*p.parts)
    if not target.resolve().is_relative_to(root.resolve()):
        raise ValueError('path escapes checkout')
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(text, encoding='utf-8')

def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

def main():
    mode, bundle_path = sys.argv[1:]
    root = pathlib.Path.cwd()
    bundle = pathlib.Path(bundle_path)
    if mode == 'unpack':
        names = [f'part-{i:02}.bin' for i in range(1, 17)]
        names += [f'tail-{i:02}.bin' for i in range(1, 13)]
        raw = b''.join((root / '.plan-bootstrap' / n).read_bytes() for n in names)
        if hashlib.sha256(raw).hexdigest() != PAYLOAD_SHA:
            raise ValueError('transport digest mismatch')
        decoder = lzma.LZMADecompressor()
        data = decoder.decompress(raw, max_length=16 * 1024 * 1024)
        if not decoder.eof or decoder.unused_data:
            raise ValueError('payload exceeds budget or contains trailing data')
        json.loads(data)
        bundle.write_bytes(data)
        bundle.chmod(0o600)
        print('Transport verified:', len(raw), 'bytes')
        return
    if mode != 'install':
        raise ValueError('unknown mode')
    payload = json.loads(bundle.read_text(encoding='utf-8'))
    for relative, text in payload['files'].items():
        if not (relative.startswith('docs/plan/standalone/') or
                relative.startswith('scripts/plan/') or relative in EXACT_PATHS):
            raise ValueError('unapproved destination: ' + relative)
        write(root, relative, text)
    restore = load(root / 'scripts/plan/restore_seed.py', 'hugit_restore')
    plan = restore.restore(payload['seed'], payload['assertion_orders'])
    content = json.dumps(plan, ensure_ascii=False, indent=2) + '\n'
    if hashlib.sha256(content.encode()).hexdigest() != SOURCE_SHA:
        raise ValueError('canonical plan digest mismatch')
    write(root, 'docs/plan/standalone/v3/backlog.json', content)
    renderer = load(root / 'docs/plan/standalone/v3/scripts/render_plan.py', 'hugit_render')
    for relative, text in renderer.all_views(plan).items():
        write(root, 'docs/plan/standalone/v3/' + relative, text)
    print('Canonical v3 restored exactly; generated views materialized.')

if __name__ == '__main__':
    main()
