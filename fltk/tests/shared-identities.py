#!/usr/bin/env python3
"""Import a browser-compatible shared identity without any network request."""
import hashlib,json,os,pathlib,queue,ssl,subprocess,sys,tempfile,threading
binary=pathlib.Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix='penguin-shared-identity-') as directory:
    root=pathlib.Path(directory);data=root/'data';catalog=data/'de-xr/identities/index.json'
    catalog.parent.mkdir(parents=True,mode=0o700)
    certificate=root/'browser.pem';key=root/'browser.key'
    subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-days','1','-subj','/UID=fixture/CN=Servo fixture','-addext','subjectAltName=DNS:localhost','-keyout',str(key),'-out',str(certificate)],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    os.chmod(key,0o600);fingerprint=hashlib.sha256(ssl.PEM_cert_to_DER_cert(certificate.read_text())).hexdigest()
    document={'version':1,'identities':[{'id':fingerprint,'name':'Servo fixture','certificate':str(certificate),'key':str(key),'address':'untrusted@metadata.invalid'}]}
    catalog.write_text(json.dumps(document));os.chmod(catalog,0o600);original=key.read_bytes()
    env={**os.environ,'HOME':str(root),'MAILRS_DATA_DIR':str(root/'mail'),'MAILRS_CONFIG':str(root/'config.toml'),'XDG_CONFIG_HOME':str(root/'config'),'XDG_DATA_HOME':str(data),'XDG_CACHE_HOME':str(root/'cache'),'GNUPGHOME':str(root/'gnupg'),'LANG':'C.UTF-8','LC_ALL':'C.UTF-8'}
    process=subprocess.Popen([str(binary)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True,env=env);responses=queue.Queue()
    def output():
        for line in process.stdout:responses.put(json.loads(line))
    threading.Thread(target=output,daemon=True).start();counter=0
    def request(op,ok=True,**fields):
        global counter
        counter+=1;process.stdin.write(json.dumps(dict(id=counter,op=op,**fields))+'\n');process.stdin.flush()
        while True:
            response=responses.get(timeout=15)
            if response.get('id')==counter:break
        assert response['ok']==ok,response
        return response.get('result',{})
    try:
        listed=request('misfin-shared-identities')['identities']
        assert listed==[dict(id=fingerprint,name='Servo fixture',address='fixture@localhost')]
        assert request('misfin-state')==dict(identities=[],messages=[])
        imported=request('misfin-import-shared-identity',shared_id=fingerprint)
        assert imported['identity']==fingerprint and imported['account']<0
        state=request('misfin-state');assert state['identities']==[dict(id=fingerprint,address='fixture@localhost')]
        assert 'PRIVATE KEY' not in json.dumps(state) and key.read_bytes()==original
        request('misfin-import-shared-identity',shared_id=fingerprint)
        assert len(request('misfin-state')['identities'])==1
        os.chmod(key,0o644);request('misfin-import-shared-identity',shared_id=fingerprint,ok=False);os.chmod(key,0o600)
        document['identities'][0]['id']='0'*64;catalog.write_text(json.dumps(document))
        assert request('misfin-shared-identities')['identities']==[]
        request('misfin-import-shared-identity',shared_id='0'*64,ok=False)
        assert request('misfin-state')==state
        print('PASS shared browser catalog listing, explicit Misfin adoption, certificate-derived address, deduplication, fingerprint recheck, key privacy and unchanged source key without network')
    finally:
        process.stdin.close()
        try:process.wait(timeout=10)
        except subprocess.TimeoutExpired:process.terminate();process.wait(timeout=5)
