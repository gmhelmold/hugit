#!/usr/bin/env python3
"""Offline tests. No credentials, GitHub calls or product execution."""
import copy,json,re,unittest
from pathlib import Path
import sync_github as s
P=json.loads((s.PLAN_ROOT/'backlog.json').read_text())
SHA=P['source_commit']
class ImportTests(unittest.TestCase):
 def test_60_contracts(self):
  for t in P['tasks']:
   body=s.body_for(t,P,SHA)
   self.assertLess(len(body),64000)
   self.assertIn(s.marker(t['id']),body)
   for key,label in s.AXES:
    self.assertIn('## '+label,body)
    for c in t['axioms'][key]:
     self.assertIn(c['id'],body);self.assertIn(c['statement'],body)
     for a in c['assertion_refs']:self.assertIn(a,body)
 def test_dependency_reduction(self):
  r=s.graph(P)
  self.assertEqual(len(r),60)
  self.assertEqual(sum(map(len,r.values())),121)
  self.assertEqual(sum(len(t['depends_on']) for t in P['tasks']),295)
 def test_preserves_human_text(self):
  t=P['tasks'][0];body=s.body_for(t,P,SHA)
  old='Human preface\n'+body+'Human appendix\n'
  updated=s.replace_generated(old,s.body_for(t,P,'1'*40))
  self.assertTrue(updated.startswith('Human preface\n'))
  self.assertTrue(updated.endswith('Human appendix\n'))
 def test_ambiguous_owned_blocks_refused(self):
  for old in ['plain',s.START+s.START+s.END,s.START+s.END+s.END]:
   with self.assertRaises(ValueError):s.replace_generated(old,s.START+'x'+s.END)
 def test_duplicate_identity_refused(self):
  i={'body':s.marker('HUG-001')}
  with self.assertRaises(ValueError):s.scan([i,i])
 def test_pull_requests_not_issues(self):
  found,parent=s.scan([{'body':s.marker('HUG-001'),'pull_request':{}}])
  self.assertEqual(found,{})
 def test_dependency_links_resolve(self):
  m={t['id']:{'number':500+i} for i,t in enumerate(P['tasks'])}
  t=next(t for t in P['tasks'] if t['id']=='HUG-056')
  body=s.body_for(t,P,SHA,499,m)
  for d in t['depends_on']:self.assertIn('#'+str(m[d]['number']),body)
 def test_does_not_add_closing_keywords(self):
  for t in P['tasks']:
   self.assertNotRegex(s.body_for(t,P,SHA),r'(?im)^\s*(?:fixes|closes|resolves)\s+#\d+')
if __name__=='__main__':unittest.main()
