#!/usr/bin/env python3
"""Small executable A/B/C verification reference, NOT an authoritative Hugit release tool.
Demo keys are supplied out-of-band by test harness and have no production authority.
Uses cryptography for Ed25519; never installs dependencies or accesses the network.
"""
from __future__ import annotations
import base64,hashlib,json
from pathlib import Path,PurePosixPath
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

def canonical(x):return json.dumps(x,sort_keys=True,ensure_ascii=False,separators=(',',':')).encode()
def digest(x):return hashlib.sha256(x).hexdigest()
def payload_bytes(p):return b'Hugit-Qualification-DEMO/v1\0'+canonical(p)
def read_artifact(root,path,expected):
 rel=PurePosixPath(path)
 if rel.is_absolute() or '..' in rel.parts or '\\' in path:raise ValueError('unsafe artifact path')
 f=root/path
 if f.is_symlink() or any(x.is_symlink() for x in f.parents if x!=root.parent):raise ValueError('symlink not admitted')
 data=f.read_bytes()
 if digest(data)!=expected:raise ValueError('artifact digest mismatch')
 return data

def verify_fixture(subject,root,bundle,pinned_executor,pinned_reviewer):
 """Verifies this limited fixture's artifacts and expected predicate, not arbitrary tests."""
 if subject.get('purpose')!='demonstration_not_product':raise ValueError('this verifier only admits its demo subject')
 a=digest(canonical(subject));payload=bundle['payload']
 if payload['subject_digest']!=a:raise ValueError('subject mismatch')
 # Pins are arguments provided by trusted test harness, never read from bundle.
 Ed25519PublicKey.from_public_bytes(pinned_executor).verify(base64.b64decode(bundle['executor_signature'],validate=True),payload_bytes(payload))
 Ed25519PublicKey.from_public_bytes(pinned_reviewer).verify(base64.b64decode(bundle['reviewer_signature'],validate=True),payload_bytes(payload))
 for artifact in subject['artifacts']:read_artifact(root,artifact['path'],artifact['sha256'])
 result=payload['result']
 if result['assertion_id']!='A-demo-output42' or result['exit']!=0:raise ValueError('fixture result not qualified')
 if result['environment_digest']!=subject['environment_digest']:raise ValueError('environment mismatch')
 observed=json.loads(read_artifact(root,result['stdout_path'],result['stdout_sha256']))
 if observed!=subject['required_output']:raise ValueError('fixture oracle failed')
 return {'fixture_qualified':True,'subject_digest':a,'qualification_digest':digest(canonical(bundle)),'scope':'demo_fixture_not_Hugit_release_permission'}

def verify_publication_fixture(subject,qualification,publish_dir,c):
 if not qualification['fixture_qualified'] or c['subject_digest']!=qualification['subject_digest'] or c['qualification_digest']!=qualification['qualification_digest']:raise ValueError('A/B/C binding mismatch')
 for a in subject['artifacts']:read_artifact(publish_dir,a['path'],a['sha256'])
 return {'state':'closed_demo_fixture','product_release_authorized':False,'subject_digest':qualification['subject_digest'],'scope':'fixture finite lifecycle only'}
