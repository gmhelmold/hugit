"""Restore the approved normative plan; exact byte checks precede installation."""
import hashlib,importlib.util,json,lzma,pathlib,sys
PREFIX='50ee168f10704c4ff2c56bd72dd693a4295f8767d6327d3594d5145b6f38cc95'
SUPPLEMENT='5940375ab193e3ee82ba6c87f281760202b63d5e57807d8cfdbc663e6d59d2c5'
SOURCE='5234276523649da9b35abe578ed6cbf2ce25c43651bfc0f6ab2d549b37e01409'
def checked(raw,expected):
 if hashlib.sha256(raw).hexdigest()!=expected:raise ValueError('byte identity mismatch')
 return raw

def write(root,relative,text):
 p=pathlib.PurePosixPath(relative)
 allowed=relative.startswith(('docs/plan/standalone/','scripts/plan/')) or relative in ['.github/PULL_REQUEST_TEMPLATE/standalone-work-package.md','.github/ISSUE_TEMPLATE/standalone-plan-change.yml']
 if not allowed or p.is_absolute() or '..' in p.parts or '\\' in relative:raise ValueError('unapproved path '+relative)
 path=root.joinpath(*p.parts)
 if not path.resolve().is_relative_to(root.resolve()):raise ValueError('path escapes checkout')
 path.parent.mkdir(parents=True,exist_ok=True);path.write_text(text,encoding='utf-8')

def load(path,name):
 spec=importlib.util.spec_from_file_location(name,path);mod=importlib.util.module_from_spec(spec);spec.loader.exec_module(mod);return mod

def recover(root,dest):
 raw=b''.join((root/f'.plan-bootstrap/fragment-{i:02}.bin').read_bytes() for i in range(1,16));checked(raw,PREFIX)
 text=lzma.LZMADecompressor().decompress(raw,max_length=8*1024*1024).decode('utf-8')
 dec=json.JSONDecoder();seed,pos=dec.raw_decode(text,len('{"seed":'))
 header=',"assertion_orders":';assert text[pos:].startswith(header)
 orders,pos=dec.raw_decode(text,pos+len(header))
 header=',"files":{';assert text[pos:].startswith(header);pos+=len(header);files={}
 while True:
  try:
   key,end=dec.raw_decode(text,pos);assert text[end]==':'
   value,end=dec.raw_decode(text,end+1);files[key]=value
   if text[end]!=',':break
   pos=end+1
  except json.JSONDecodeError:break
 if len(files)!=7:raise ValueError('unexpected complete file count')
 supplement=b''.join((root/f'.plan-bootstrap/supplement-{i:02}.bin').read_bytes() for i in range(1,10));checked(supplement,SUPPLEMENT)
 decoder=lzma.LZMADecompressor();unpacked=decoder.decompress(supplement,max_length=1024*1024)
 assert decoder.eof and not decoder.unused_data
 extras=json.loads(unpacked)
 if set(files)&set(extras):raise ValueError('duplicate file authority')
 files.update(extras)
 dest.write_text(json.dumps({'seed':seed,'assertion_orders':orders,'files':files},ensure_ascii=False),encoding='utf-8')
 print('Exact prefix and supplemental scripts verified; retained files:',len(files))

def install(root,bundle):
 data=json.loads(bundle.read_text())
 for rel,text in data['files'].items():write(root,rel,text)
 restore=load(root/'scripts/plan/restore_seed.py','restore_seed')
 plan=restore.restore(data['seed'],data['assertion_orders'])
 content=json.dumps(plan,ensure_ascii=False,indent=2)+'\n';checked(content.encode(),SOURCE)
 write(root,'docs/plan/standalone/v3/backlog.json',content)
 manifest=json.loads((root/'docs/plan/standalone/IMPORT-MANIFEST.json').read_text())
 for name,digest in manifest['original_scripts_sha256'].items():checked((root/'docs/plan/standalone/v3/scripts'/name).read_bytes(),digest)
 renderer=load(root/'docs/plan/standalone/v3/scripts/render_plan.py','render_plan')
 for rel,text in renderer.all_views(plan).items():write(root,'docs/plan/standalone/v3/'+rel,text)
 print('Canonical v3 source SHA256:',SOURCE,'; generated views:',len(renderer.all_views(plan)))

if __name__=='__main__':
 mode,bundle=sys.argv[1:];root=pathlib.Path.cwd();dest=pathlib.Path(bundle)
 if mode=='recover':recover(root,dest)
 elif mode=='install':install(root,dest)
 else:raise ValueError('unknown mode')
