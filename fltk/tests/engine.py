#!/usr/bin/env python3
"""Exercise real upstream sync/actions against the isolated demo servers."""
import json, os, subprocess, sys, tempfile, threading, queue, time
from pathlib import Path
binary=Path(sys.argv[1] if len(sys.argv)>1 else 'target/debug/penguin-mail-engine').resolve()
checks=[]
def check(ok, name):
    if not ok: raise AssertionError(name)
    checks.append(name); print('PASS', name, flush=True)
with tempfile.TemporaryDirectory(prefix='penguin-fltk-engine-test-') as directory:
    env={**os.environ,'XDG_CONFIG_HOME':directory+'/config','XDG_DATA_HOME':directory+'/data','XDG_CACHE_HOME':directory+'/cache','GNUPGHOME':directory+'/gnupg'}
    p=subprocess.Popen([str(binary),'--demo'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,env=env)
    output=queue.Queue()
    def read():
        for line in p.stdout:
            try: output.put(json.loads(line))
            except Exception as e: output.put({'error':str(e)})
    threading.Thread(target=read,daemon=True).start()
    index=0
    def request(op, **args):
        global index
        index+=1; p.stdin.write(json.dumps({'id':index,'op':op,**args})+'\n');p.stdin.flush()
        until=time.monotonic()+30
        while time.monotonic()<until:
            r=output.get(timeout=max(.1,until-time.monotonic()))
            if r.get('id')==index:
                if not r.get('ok'): raise AssertionError(f'{op}: {r.get("error")}')
                return r['result']
        raise TimeoutError(op)
    try:
        a=request('accounts');check(len(a['items'])==3,'demo seeds three accounts without a real keyring')
        rows=request('list',mailbox='inbox')['rows'];check(len(rows)==12,'unified inbox lists real stored demo conversations')
        target={'account':rows[0]['account_id'],'thread':rows[0]['id']}
        thread=request('thread',**target);check('temporary database' in thread['messages'][0]['text'],'reader loads the original message body')
        request('action',action='archive',targets=[target]);check(len(request('list',mailbox='inbox')['rows'])==11,'archive changes the upstream server and local listing')
        request('undo');check(len(request('list',mailbox='inbox')['rows'])==12,'undo restores archived mail through the upstream action history')
        request('action',action='read',targets=[target]);check(not next(r for r in request('list',mailbox='inbox')['rows'] if r['id']==target['thread'] and r['account_id']==target['account'])['unread'],'read marks survive a list refresh')
        request('undo')
        request('action',action='trash',targets=[target]);check(any(r['id']==target['thread'] for r in request('list',mailbox='trash')['rows']),'Trash lists mail changed by the real triage operation');request('undo')
        search=request('list',mailbox='search',query='subject:booking')['rows'];check(len(search)==3,'upstream search syntax works across accounts')
        account=a['items'][0]['account']['id']
        compose=dict(account=account,to='someone@example.net',cc='',bcc='',subject='FLTK integration test',text='A test message.',files=[])
        draft=request('compose',draft=True,**compose);check(bool(draft['draft_id']),'saving a draft uses the real upstream draft path')
        if 'message_id' in draft:
            reopened=request('draft-open',account=account,message=draft['message_id'],thread=draft['thread'])
            check(reopened['subject']==compose['subject'] and reopened['to']==compose['to'] and 'A test message.' in reopened['text'],'saved drafts reopen through the original raw MIME bytes')
        attachment=Path(directory)/'attachment.txt';attachment.write_text('Native attachment round trip.\n')
        with_file={**compose,'subject':'Draft with attachment','files':[str(attachment)]}
        attached=request('compose',draft=True,**with_file)
        reopened=request('draft-open',account=account,message=attached['message_id'],thread=attached['thread'])
        import base64
        check(len(reopened['kept_files'])==1 and base64.b64decode(reopened['kept_files'][0]['data'])==attachment.read_bytes(),'draft reopening preserves attachment bytes')
        retained={**reopened,'files':[],'draft':True};retained.pop('op',None)
        updated=request('compose',**retained)
        again=request('draft-open',account=account,message=updated['message_id'],thread=updated['thread'])
        check(base64.b64decode(again['kept_files'][0]['data'])==attachment.read_bytes(),'saving an edited draft retains existing attachments')
        sent=request('compose',draft=False,**compose);check(sent.get('sent') or sent.get('queued'),'sending uses the upstream outbox and fake provider')
        later=request('compose',draft=False,send_at=int(time.time()*1000)+3600000,**compose);check(later.get('queued'),'Send Later persists a scheduled outgoing message')
        check(len(request('list',mailbox='scheduled')['rows'])>=1,'scheduled mail appears in Send Later')
        cal=request('calendar',account=0);check(len(cal['events'])>=3 and len(cal['calendars'])==3,'calendar reads all accounts through CalendarCopy')
        when=int(time.time()*1000)+86400000
        event=request('event-save',account=account,title='FLTK test appointment',start=when,end=when+3600000,place='Test room',description='Integration test')
        check(event['event']['title']=='FLTK test appointment','calendar event creation goes through the upstream write queue')
        cal=request('calendar',account=account);check(any(e['occurrence']['event']['title']=='FLTK test appointment' for e in cal['events']),'calendar lists a newly created event')
        event_id=event['event']['id'];request('event-delete',account=account,event=event_id)
        book=request('contacts');check(len(book['items'])>0,'contacts are read from upstream address storage')
        request('contact-save',account=account,name='Test Person',email='native-test@example.net')
        check(any(c['email']=='native-test@example.net' for c in request('contacts')['items']),'contact creation keeps the provider and local address book in sync')
        destination=Path(directory)/'export.mbox';request('export',**target,path=str(destination));check(destination.read_bytes().startswith(b'From '),'conversation export writes a real mbox file')
        check(not (Path(directory)/'data'/'penguin-mail'/'mailrs.db').exists(),'demo does not create a personal mail store')
    finally:
        p.stdin.close()
        try:p.wait(timeout=5)
        except subprocess.TimeoutExpired:p.terminate();p.wait(timeout=5)
        error=p.stderr.read()
        if error:print(error,file=sys.stderr)
    check(p.returncode==0,'engine drains outstanding work and exits cleanly on pipe close')
print(json.dumps({'passed':len(checks),'checks':checks},indent=2))
