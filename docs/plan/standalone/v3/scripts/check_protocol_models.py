#!/usr/bin/env python3
"""Bounded reference checks for plan contracts; NEVER Hugit runtime tests.
SQL examples exercise SQLite locally; the other checks model declared transitions.
"""
from __future__ import annotations
import argparse,json,sqlite3,itertools,hashlib
from collections import deque
from pathlib import Path

def delivery_model(unsafe=False):
 # epoch, owner(-/export/revoke), cached grant, exported, revoke_ack, export_done, revoke_done
 initial=(1,'-',False,False,False,False,False);todo=deque([(initial,[])]);seen={initial};bad=[]
 while todo:
  s,tr=todo.popleft();ep,owner,grant,pub,ack,xdone,rdone=s
  if pub and ack and tr and tr[-1]=='export_publish':bad.append(tr);continue
  edges=[]
  if not xdone:
   if owner=='-' and not grant:
    if ep==1:edges.append(('export_check',(ep,'-' if unsafe else 'export',True,pub,ack,False,rdone)))
    else:edges.append(('export_denied',(ep,owner,False,pub,ack,True,rdone)))
   if grant and (owner=='export' or unsafe):edges.append(('export_publish',(ep,'-',False,True,ack,True,rdone)))
  if not rdone and owner=='-':edges.append(('revoke_ack',(2,'-',grant,pub,True,xdone,True)))
  for action,n in edges:
   if action=='export_publish' and ack:
    bad.append(tr+[action]);continue
   if n not in seen:seen.add(n);todo.append((n,tr+[action]))
 return {'states':len(seen),'counterexamples':bad,'scope':'bounded_gate_model_not_io_or_scheduling_proof'}

def sql_cursor():
 db=sqlite3.connect(':memory:');db.executescript('''create table meta(g integer);insert into meta values(10);create table runs(id integer primary key,status text,seq integer);insert into runs values(1,'failed',8),(2,'failed',9);create trigger bump after update on runs begin update meta set g=g+1;end;''')
 def page(cursor=None):
  db.execute('begin');g=db.execute('select g from meta').fetchone()[0]
  if cursor and cursor[0]!=g:db.rollback();return {'error':'CURSOR_INVALIDATED','generation':g}
  rows=db.execute("select id,status from runs where status='failed' and id>? order by id limit 1",(cursor[1] if cursor else 0,)).fetchall();db.commit();return {'rows':rows,'cursor':(g,rows[-1][0] if rows else cursor[1])}
 one=page();two=page(one['cursor']);assert [r[0] for r in one['rows']+two['rows']]==[1,2]
 db.execute("update runs set status='passed',seq=11 where id=2");db.commit();invalid=page(one['cursor']);assert invalid['error']=='CURSOR_INVALIDATED'
 unsafe=db.execute("select id from runs where status='failed' and seq<=10 and id>1").fetchall();assert unsafe==[]
 return {'sqlite_version':sqlite3.sqlite_version,'page1':one,'unchanged_page2':two,'changed_page2':invalid,'unsafe_watermark_result':unsafe,'unsafe_expected_id_2_missing':True,'scope':'SQL_reference_not_Hugit_schema'}

def replay():
 db=sqlite3.connect(':memory:');db.executescript('create table events(origin text,event text,payload text,unique(origin,event));create table deliveries(id text primary key,collector text,origin text,event text);')
 def send(origin,event,payload,collector,delivery):
  db.execute('begin');old=db.execute('select payload from events where origin=? and event=?',(origin,event)).fetchone()
  if old and old[0]!=payload:db.rollback();return 'CONFLICT'
  db.execute('insert or ignore into events values(?,?,?)',(origin,event,payload));db.execute('insert into deliveries values(?,?,?,?)',(delivery,collector,origin,event));db.commit();return 'ACK'
 assert send('session-store-generation','42','run-A','collector-A','delivery-1')=='ACK'
 # committed event; watermark not saved; restart a different collector
 assert send('session-store-generation','42','run-A','collector-B','delivery-2')=='ACK'
 events=db.execute('select count(*) from events').fetchone()[0];deliveries=db.execute('select count(*) from deliveries').fetchone()[0];assert (events,deliveries)==(1,2)
 assert send('session-store-generation','43','run-A','collector-B','delivery-3')=='ACK'
 assert send('session-store-generation','42','different','collector-B','delivery-4')=='CONFLICT'
 assert send('new-source-generation','42','run-new','collector-B','delivery-5')=='ACK'
 unsafe=len({('collector-A','42'),('collector-B','42')});assert unsafe==2
 return {'replayed_event_occurrences':events,'deliveries':deliveries,'new_execution_occurrences_total':2,'generation_reset_distinct':True,'conflicting_bytes':'CONFLICT','unsafe_collector_key_count':unsafe,'scope':'SQL_identity_reference_not_adapter_runtime'}

def restore():
 backup={'epoch':1,'private_allowed':['secret-a']};raw=json.dumps(backup,sort_keys=True).encode();digest=hashlib.sha256(raw).hexdigest()
 def load(anchor=None,reauthorize=False):
  if anchor is not None:return {'freshness':'authority_preserved','private_allowed':[],'revoked':True}
  if reauthorize:return {'freshness':'new_local_decision_not_historical_knowledge','private_allowed':['secret-a'],'authorization_event':'new-local'}
  return {'freshness':'unknown','private_allowed':[]}
 a=load();b=load();assert a==b and not a['private_allowed'];known=load({'epoch':2});new=load(reauthorize=True)
 assert known['revoked'] and new['authorization_event']=='new-local'
 return {'same_backup_digest_two_worlds':digest,'cold_restore':a,'known_authority':known,'explicit_recovery':new,'scope':'information_boundary_reference_not_restore_implementation'}

def promotion():
 legacy=[]
 for order in [('target','witness'),('witness','target')]:
  s={'target':False,'witness':False}
  for step in order:
   s[step]=True;legacy.append({'order':order,'after':step,**s,'unsafe_implies_committed':s['witness'],'false_proof':s['witness'] and not s['target']})
 assert any(x['false_proof'] for x in legacy)
 # Objects/ref/SQL are distinct, plus observed corruption and loss after restart.
 states=[]
 for obj,ref,sql in itertools.product([False,True],repeat=3):
  observed_valid=obj and ref
  status='MANAGED_PUBLISHED' if observed_valid else ('AMBIGUOUS' if sql or ref else 'PREPARED_RETRYABLE')
  states.append({'objects_valid':obj,'managed_ref_matches':ref,'sql_prior_record':sql,'reconciled':status,'human_branch_claim':False})
 assert all(x['reconciled']!='MANAGED_PUBLISHED' or (x['objects_valid'] and x['managed_ref_matches']) for x in states)
 return {'legacy_partial_states':legacy,'managed_recovery_states':states,'unsafe_control_found':True,'scope':'bounded_observation_recovery_not_power_failure_proof'}

def main():
 ap=argparse.ArgumentParser();ap.add_argument('--root',type=Path,default=Path(__file__).resolve().parents[1]);a=ap.parse_args()
 safe=delivery_model();unsafe=delivery_model(True);assert not safe['counterexamples'] and unsafe['counterexamples']
 report={'scope':'limited_reference_contract_checks_not_Hugit_product','gate_safe':safe,'gate_unsafe_control':unsafe,'cursor':sql_cursor(),'replay':replay(),'restore':restore(),'promotion':promotion(),'all_assertions_passed':True,'limits':['No exhaustive OS or filesystem model','No machine/power-failure experiment','No native Hugit v3','No independent reviewer']}
 path=a.root/'validation/reference-protocol-checks.json';path.parent.mkdir(exist_ok=True);path.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n');print(json.dumps({'all_assertions_passed':True,'scope':report['scope'],'safe_gate_states':safe['states'],'unsafe_counterexample':unsafe['counterexamples']},indent=2))
if __name__=='__main__':main()
