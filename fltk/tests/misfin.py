#!/usr/bin/env python3
"""Exercise the optional engine against a private mutual-TLS Misfin/Gemini server."""
import hashlib,json,os,pathlib,queue,socket,ssl,subprocess,sys,tempfile,threading,time
binary=pathlib.Path(sys.argv[1] if len(sys.argv)>1 else 'build-fltk-misfin/penguin-mail-engine').resolve()
checks=[]
def check(value,label):
    assert value,label
    checks.append(label);print('PASS '+label,flush=True)
with tempfile.TemporaryDirectory(prefix='penguin-misfin-') as name:
    root=pathlib.Path(name)
    def cert(who,uid):
        key=root/(who+'.key');crt=root/(who+'.crt')
        subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-sha256','-days','1','-nodes','-subj',f'/UID={uid}/CN=Penguin Test','-addext','subjectAltName=DNS:localhost','-keyout',str(key),'-out',str(crt)],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        return crt,key
    client,key=cert('client','alice');server,server_key=cert('server','admin');changed,changed_key=cert('changed','admin')
    fp=hashlib.sha256(ssl.PEM_cert_to_DER_cert(server.read_text())).hexdigest()
    mailbox_pin=[b'1'*64]
    captured=[];stop=threading.Event();errors=[]
    listener=socket.socket();listener.bind(('127.0.0.1',0));listener.listen();listener.settimeout(.2);port=listener.getsockname()[1]
    context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);context.minimum_version=ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(server,server_key);context.verify_mode=ssl.CERT_REQUIRED;context.load_verify_locations(client)
    fixture=b'alice@localhost Alice\nbob@localhost\n2026-10-24T12:00:00Z\n# Inbox\nHello from Gemini\n'
    def serve():
        while not stop.is_set():
            try:raw,_=listener.accept()
            except socket.timeout:continue
            except OSError:break
            try:
                with context.wrap_socket(raw,server_side=True) as conn:
                    conn.settimeout(3);header=b''
                    while not header.endswith(b'\r\n'):
                        b=conn.recv(1)
                        if not b:break
                        header+=b
                        if len(header)>1024:raise AssertionError('unbounded header')
                    if not header:continue
                    if header.startswith(b'gemini://'):
                        captured.append(('gemini',header))
                        conn.sendall(b'20 application/gembox\r\n'+fixture)
                        try:conn.unwrap().close()
                        except OSError:pass
                    else:
                        uri,size=header[:-2].split(b'\t');remaining=int(size);body=b''
                        while remaining:
                            data=conn.recv(remaining)
                            if not data:break
                            body+=data;remaining-=len(data)
                        captured.append(('misfin',uri,body))
                        if b'Decline' in body:conn.sendall(b'44 Slow down\r\n')
                        elif b'Bad response' in body:conn.sendall(b'20xBroken\r\n')
                        else:conn.sendall(b'20 '+mailbox_pin[0]+b'\r\n')
            except (ssl.SSLError,ConnectionError,socket.timeout):pass
            except Exception as e:errors.append(e)
    thread=threading.Thread(target=serve,daemon=True);thread.start()
    env={**os.environ,'LANG':'C.UTF-8','LC_ALL':'C.UTF-8','MAILRS_DATA_DIR':name+'/mail','MAILRS_CONFIG':name+'/config.toml','XDG_CONFIG_HOME':name+'/config','XDG_DATA_HOME':name+'/data','XDG_CACHE_HOME':name+'/cache','GNUPGHOME':name+'/gnupg'}
    def engine():
        p=subprocess.Popen([str(binary)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True,env=env)
        responses=queue.Queue()
        def output():
            for line in p.stdout:
                responses.put(json.loads(line))
        threading.Thread(target=output,daemon=True).start();index=0
        def request(op,fail=False,**fields):
            nonlocal index
            index+=1;p.stdin.write(json.dumps({'id':index,'op':op,**fields})+'\n');p.stdin.flush()
            until=time.monotonic()+35
            while time.monotonic()<until:
                reply=responses.get(timeout=max(.1,until-time.monotonic()))
                if reply.get('id')==index:
                    if fail:
                        check(not reply['ok'],op+' rejects invalid input');return reply
                    assert reply['ok'],reply.get('error');return reply['result']
            raise TimeoutError(op)
        return p,request
    p,request=engine()
    try:
        check(request('misfin-state')=={'identities':[],'messages':[]},'Optional store starts empty')
        ident=request('misfin-import-identity',certificate=str(client),key=str(key))['identity']
        check(request('misfin-state')['identities'][0]['address']=='alice@localhost','Identity address comes from certificate USER_ID and DNS SAN')
        request('misfin-import-identity',certificate=str(client),key=str(server_key),fail=True)
        encrypted=root/'encrypted.key'
        subprocess.run(['openssl','pkey','-in',str(key),'-aes-256-cbc','-passout','pass:test-fixture-only','-out',str(encrypted)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,check=True)
        request('misfin-import-identity',certificate=str(client),key=str(encrypted),fail=True)
        dest=f'bob@localhost:{port}'
        peer=request('misfin-probe',identity=ident,destination=dest)
        check(peer['fingerprint']==fp and not peer['trusted'],'Self-signed server is inspected without sending message data')
        before=len(captured)
        request('misfin-send',identity=ident,destination=dest,fingerprint='0'*64,subject='No send',body='Secret',fail=True)
        check(len(captured)==before,'Fingerprint mismatch blocks application data')
        request('misfin-send',identity=ident,destination=dest,fingerprint=fp,subject='Žluťoučký',body='世界\nSecond line')
        check(captured[-1][2].decode()=='\n\n\n# Žluťoučký\n\n世界\nSecond line','Misfin(C) transmits exact UTF-8 framing and body')
        check(request('misfin-probe',identity=ident,destination=dest)['trusted'],'Confirmed server trust is remembered')
        before=len(captured)
        request('misfin-send',identity=ident,destination=dest,fingerprint=fp,subject='Too big',body='界'*6000,fail=True)
        check(len(captured)==before,'Oversized UTF-8 message is rejected before network delivery')
        request('misfin-send',identity=ident,destination=dest,fingerprint=fp,subject='Decline',body='No retry',fail=True)
        check(len(request('misfin-state')['messages'])==1,'Server refusal is not recorded as a sent message')
        request('misfin-send',identity=ident,destination=dest,fingerprint=fp,subject='Bad response',body='No retry',fail=True)
        inbox=f'gemini://localhost:{port}/mail.gembox'
        check(request('misfin-probe',identity=ident,destination=inbox,gemini=True)['fingerprint']==fp,'Gemini mailbox uses the same identity and explicit trust model')
        request('misfin-fetch',identity=ident,destination=inbox,fingerprint=fp)
        request('misfin-fetch',identity=ident,destination=inbox,fingerprint=fp)
        state=request('misfin-state')
        check(len(state['messages'])==2 and state['messages'][-1]['subject']=='Inbox','Gemini archive fetch deduplicates received gemmail')
        archive=root/'local.gembox';archive.write_bytes(fixture)
        request('misfin-import-mail',identity=ident,path=str(archive))
        check(len(request('misfin-state')['messages'])==2,'Local archive import shares deduplication')
        saved=root/'mail/misfin/state.json'
        check(saved.stat().st_mode&0o777==0o600 and saved.parent.stat().st_mode&0o777==0o700,'Private identities and history have owner-only permissions')
        check('PRIVATE KEY' not in json.dumps(state),'Engine replies never expose private keys')
        mailbox_pin[0]=b'2'*64
        receipt=request('misfin-send',identity=ident,destination=dest,fingerprint=fp,subject='Mailbox certificate changed',body='Accepted once')
        check('fingerprint changed' in receipt['warning'],'Recipient certificate changes are reported without losing the success receipt')
        context.load_cert_chain(changed,changed_key)
        request('misfin-send',identity=ident,destination=dest,fingerprint=fp,subject='Changed',body='Secret',fail=True)
        check(len(request('misfin-state')['messages'])==3,'Changed certificate cannot silently replace trusted peer')
        original=request('misfin-state')['identities']
        for address,name in [('bad/name@localhost','Alice'),('alice@localhost:1958','Alice'),
                             ('alice@127.0.0.1','Alice'),('alice@-invalid.test','Alice'),
                             ('alice@localhost',''),('alice@localhost','Bad\nName')]:
            request('misfin-create-identity',address=address,name=name,fail=True)
        check(request('misfin-state')['identities']==original,'Rejected creation leaves existing identities unchanged')
        request('misfin-create-identity',address='alice@LOCALHOST',name='Duplicate',fail=True)
        created=request('misfin-create-identity',address='new@LOCALHOST',name='Žluťoučký 世界')['identity']
        public=request('misfin-certificate',identity=created)
        check(public['address']=='new@localhost' and public['fingerprint']==created and 'PRIVATE KEY' not in json.dumps(public),
              'Created identity exposes only its public certificate and normalized address')
        generated=root/'generated.crt';generated.write_text(public['certificate'])
        check(hashlib.sha256(ssl.PEM_cert_to_DER_cert(public['certificate'])).hexdigest()==created,
              'Created identity fingerprint matches its certificate')
        subprocess.run(['openssl','verify','-CAfile',str(generated),'-purpose','sslclient',str(generated)],check=True,stdout=subprocess.DEVNULL)
        details=subprocess.check_output(['openssl','x509','-in',str(generated),'-text','-noout'],text=True)
        check('DNS:localhost' in details and 'CA:FALSE' in details and '2048 bit' in details,
              'Generated certificate is a self-signed RSA client identity with DNS SAN')
        context.load_cert_chain(server,server_key);context.load_verify_locations(generated)
        check(request('misfin-probe',identity=created,destination=dest)['fingerprint']==fp,
              'Generated private key authenticates with mutual TLS')
        state=request('misfin-state')
        check(len(state['identities'])==2 and 'PRIVATE KEY' not in json.dumps(state),
              'New identity preserves existing mail and keeps its key private')
        p.stdin.close();p.wait(timeout=10)
        p,request=engine()
        check(len(request('misfin-state')['messages'])==3,'Identity and local history survive engine restart')
        check(any(i['id']==created for i in request('misfin-state')['identities']),'Created identity survives engine restart')
        projected=request('accounts')['items']
        existing=next(i for i in projected if i['account'].get('misfin_identity')==ident)['account']
        account=existing['id']
        check(account<0 and existing['email']=='alice@localhost','Existing certificate identity appears as a normal native account')
        allrows=request('list',account=account,mailbox='all')['rows']
        inboxrows=request('list',account=account,mailbox='inbox')['rows']
        sentrows=request('list',account=account,mailbox='sent')['rows']
        check(len(allrows)==3 and len(inboxrows)==1 and len(sentrows)==2,'Existing Misfin history populates normal Inbox and Sent mailboxes')
        check(any(row['account_id']==account for row in request('list',account=0,mailbox='inbox')['rows']),
              'All Accounts includes received Misfin messages')
        message=request('thread',account=account,thread=inboxrows[0]['id'])['messages'][0]
        check(message['meta']['subject']=='Inbox' and message['body']['attachments']==[] and message['meta']['from']['email']=='alice@localhost',
              'Normal reader opens Misfin Gemtext without MIME or attachments')
        request('list',account=account,mailbox='drafts',fail=True)
        request('action',action='trash',targets=[{'account':account,'thread':inboxrows[0]['id']}],fail=True)
        request('compose',account=1,misfin=True,to=dest,subject='No SMTP',text='Secret',fail=True)
        base=dict(account=account,misfin=True,to=dest,cc='',bcc='',subject='Integrated compose',text='Plain Gemtext',files=[],kept_files=[],draft=False,sign=False,encrypt=False)
        before=len(captured)
        for fields in [dict(cc='other@localhost'),dict(bcc='other@localhost'),dict(files=['missing']),
                       dict(kept_files=[{}]),dict(draft=True),dict(send_at=1),dict(sign=True),dict(encrypt=True),
                       dict(misfin=False),dict(to='a@localhost,b@localhost'),dict(text='界'*6000)]:
            request('misfin-check-compose',**(base|fields),fail=True)
        check(len(captured)==before,'Unsupported composer options are refused before contacting a server')
        request('misfin-check-compose',**base)
        request('compose',**base,fingerprint=fp)
        check(len(request('list',account=account,mailbox='sent')['rows'])==3,
              'Normal compose sends through Misfin and adds the Sent copy')
        request('misfin-settings',account=account,inbox_url=inbox)
        request('misfin-import-identity',certificate=str(client),key=str(key))
        check(next(i for i in request('accounts')['items'] if i['account']['id']==account)['account']['misfin_inbox_url']==inbox,
              'Account settings retain the receiving URL across certificate reimport')
        p.stdin.close();p.wait(timeout=10)
        # Simulate the earlier identity-only schema for the second identity.
        legacy=json.loads(saved.read_text())
        for i in legacy['identities']:
            if i['id']==created:i.pop('inbox_url',None)
        saved.write_text(json.dumps(legacy))
        p,request=engine()
        projected=request('accounts')['items']
        check(len(projected)==2 and next(i for i in projected if i['account']['id']==account)['account']['misfin_inbox_url']==inbox,
              'Native account projection survives restart and reads legacy identities without migration')
        check(not errors,'TLS fixture completed without internal errors')
    finally:
        if p.poll() is None:p.terminate();p.wait(timeout=5)
        stop.set();listener.close();thread.join(timeout=4)
print(f'{len(checks)} Misfin checks passed')
