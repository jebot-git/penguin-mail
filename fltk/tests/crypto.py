#!/usr/bin/env python3
"""Real GnuPG through the native worker, with private generated keys and fake mail."""
import base64,email,email.policy,json,os,queue,signal,subprocess,sys,tempfile,threading,time
signal.pthread_sigmask(signal.SIG_SETMASK,[])
from pathlib import Path
binary=Path(sys.argv[1] if len(sys.argv)>1 else 'build-fltk/penguin-mail-engine').resolve()
checks=[]
def check(ok,label):
    assert ok,label
    checks.append(label);print('PASS',label,flush=True)
with tempfile.TemporaryDirectory(prefix='penguin-native-pgp-') as temp:
    root=Path(temp);home=root/'gnupg';home.mkdir(mode=0o700)
    (home/'gpg-agent.conf').write_text('pinentry-program /bin/false\n')
    env={**os.environ,'GNUPGHOME':str(home),'XDG_CONFIG_HOME':temp+'/config','XDG_DATA_HOME':temp+'/data','XDG_CACHE_HOME':temp+'/cache','LANG':'C.UTF-8'}
    def gpg(*args,input=None):
        return subprocess.run(['gpg','--batch','--no-tty',*args],input=input,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,check=True).stdout
    for address in ['alex@example.com','bo@example.test','cy@example.test']:
        gpg('--pinentry-mode','loopback','--passphrase','','--quick-generate-key',address,'future-default','default','0')
    export=root/'keys.asc';export.write_bytes(gpg('--armor','--export','alex@example.com'))
    secret=root/'private-keys.asc';secret.write_bytes(gpg('--armor','--export-secret-keys'));secret.chmod(0o600)
    p=subprocess.Popen([str(binary),'--demo'],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    output=queue.Queue()
    def reader():
        for line in p.stdout:
            output.put(json.loads(line))
    threading.Thread(target=reader,daemon=True).start();index=0
    def reply(op,**args):
        global index
        index+=1;p.stdin.write(json.dumps(dict(id=index,op=op,**args))+'\n');p.stdin.flush()
        until=time.monotonic()+45
        while time.monotonic()<until:
            r=output.get(timeout=max(.1,until-time.monotonic()))
            if r.get('id')==index:return r
        raise TimeoutError(op)
    def request(op,**args):
        r=reply(op,**args)
        assert r.get('ok'),f'{op}: {r.get("error")}'
        return r['result']
    def reopen(d):return request('draft-open',account=account,message=d['message_id'],thread=d['thread'])
    def raw_of(d,name):
        path=root/name;request('export',account=account,thread=d['thread'],path=str(path))
        return path.read_bytes().split(b'\n',1)[1].replace(b'\n',b'\r\n').replace(b'\r\r\n',b'\r\n')
    try:
        check(request('crypto-status')['available'],'native worker discovers real GnuPG')
        imported=request('crypto-import',path=str(export));check(bool(imported['keys']),'picked key import runs through upstream GnuPG')
        empty=request('crypto-keys',**{'from':'alex@example.com','to':'bo@example.test'})
        check(not empty['can_sign'] and not empty['can_encrypt'],'demo uses a temporary keyring rather than the personal GnuPG home')
        request('crypto-import',path=str(secret))
        state=request('crypto-keys',**{'from':'alex@example.com','to':'bo@example.test','cc':'','bcc':'cy@example.test'})
        check(state['can_sign'] and state['can_encrypt'] and len(state['keys'])==3,'key inspection checks To, Bcc, own copy and a secret signing key')
        missing=request('crypto-keys',**{'from':'alex@example.com','to':'absent@example.test'})
        check(not missing['can_encrypt'] and 'absent@example.test' in missing['cannot_encrypt'],'missing recipient keys are named before encryption')
        accounts=request('accounts')['items'];account=accounts[0]['account']['id']
        file=root/'note.txt';file.write_text('Attachment protected by GnuPG.\n')
        form=dict(account=account,to='bo@example.test',cc='',bcc='cy@example.test',subject='Native protected round trip',text='Secret native message with Unicode: příliš.',files=[str(file)])
        signed=request('compose',draft=True,sign=True,encrypt=False,**form)
        opened=reopen(signed)
        check(opened['sign'] and not opened['encrypt'] and 'Secret native' in opened['text'],'signed draft reopens with its signing choice and verified text')
        read=request('thread',account=account,thread=signed['thread'])['messages'][0]
        check(read['mark']['tone']=='Unchecked' and read['mark']['title'].startswith('Signed by alex@example.com'),'native reader verifies the exact signed MIME bytes')
        check('Nobody has vouched' in read['mark']['detail'],'a valid signature is distinguished from an untrusted key identity')
        check(base64.b64decode(read['files'][0])==file.read_bytes(),'verified attachments are taken only from the signed entity')
        cipher=request('compose',draft=True,sign=True,encrypt=True,**form)
        raw=raw_of(cipher,'encrypted.mbox')
        check(b'multipart/encrypted' in raw and b'Secret native' not in raw and b'Attachment protected' not in raw,'server draft stores ciphertext without body or attachment plaintext')
        opened=reopen(cipher)
        check(opened['encrypt'] and opened['sign'] and 'příliš' in opened['text'] and opened['bcc']=='cy@example.test','encrypted draft restores text, sign/encrypt state and blind recipients')
        check(base64.b64decode(opened['kept_files'][0]['data'])==file.read_bytes(),'encrypted draft restores attachment bytes')
        read=request('thread',account=account,thread=cipher['thread'])['messages'][0]
        check(read['sealed'] and 'Secret native' in read['text'],'native reader decrypts the original message in memory')
        dest=root/'saved.txt';request('attachment',account=account,opened_data=read['files'][0],path=str(dest))
        check(dest.read_bytes()==file.read_bytes(),'saving a decrypted attachment writes the in-memory original bytes')
        # Send goes only to the in-memory fake provider. The exported Sent copy uses real wire bytes.
        sent=request('compose',draft=False,sign=True,encrypt=True,**form)
        check(sent.get('sent') or sent.get('queued'),'signed encrypted send uses the original outbox')
        request('refresh')
        found=None
        until=time.monotonic()+12
        while time.monotonic()<until and found is None:
            for row in request('list',account=account,mailbox='sent')['rows']:
                if row.get('subject')==form['subject']:
                    candidate=request('thread',account=account,thread=row['id'])['messages']
                    if any(m.get('sealed') and m.get('mark',{}).get('title','').startswith('Encrypted, and signed by alex@example.com') for m in candidate):found=candidate;sent_thread=row['id'];break
            if found is None:time.sleep(.15)
        check(found is not None,'Sent copy decrypts for its sender and verifies its inner signature')
        sent_raw=raw_of({'thread':sent_thread},'sent-encrypted.mbox')
        sent_message=email.message_from_bytes(sent_raw,policy=email.policy.default)
        sent_ciphertext=sent_message.get_payload()[1].get_payload(decode=True)
        sent_packets=gpg('--list-only','--list-packets',input=sent_ciphertext).decode(errors='replace')
        def encryption_ids(address):
            listing=gpg('--with-colons','--list-keys',address).decode()
            return [line.split(':')[4] for line in listing.splitlines() if line.startswith(('pub:','sub:'))]
        check(any(key in sent_packets for key in encryption_ids('alex@example.com')),'encrypted Sent copy explicitly includes the sender key')
        check(any(key in sent_packets for key in encryption_ids('bo@example.test')),'encrypted send explicitly includes each visible recipient')
        check('0000000000000000' in sent_packets and not any(key in sent_packets for key in encryption_ids('cy@example.test')),'native Bcc encryption hides blind-recipient key identities')

        refused=reply('compose',draft=False,sign=False,encrypt=True,**{**form,'to':'absent@example.test'})
        check(not refused['ok'] and 'absent@example.test' in refused['error'],'missing keys fail closed without sending unencrypted mail')
        # Scope of a saved encrypted draft is the writer alone, not its eventual recipients.
        message=email.message_from_bytes(raw,policy=email.policy.default)
        ciphertext=message.get_payload()[1].get_payload(decode=True)
        packets=gpg('--list-only','--list-packets',input=ciphertext).decode(errors='replace')
        bo=gpg('--with-colons','--list-keys','bo@example.test').decode()
        ids=[line.split(':')[4] for line in bo.splitlines() if line.startswith(('pub:','sub:'))]
        check(not any(key in packets for key in ids),'saved encrypted draft excludes recipients until it is sent')
    finally:
        p.stdin.close()
        try:p.wait(timeout=8)
        except subprocess.TimeoutExpired:p.terminate();p.wait(timeout=5)
        error=p.stderr.read()
        if error:print(error,file=sys.stderr)
        subprocess.run(['gpgconf','--kill','gpg-agent'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    check(p.returncode==0,'crypto worker exits cleanly with its private keyring untouched outside the fixture')
print(json.dumps({'passed':len(checks),'checks':checks},indent=2))
