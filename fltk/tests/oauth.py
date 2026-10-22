#!/usr/bin/env python3
"""Check the installed client's browser/PKCE/loopback flow without a Google login."""
import http.client,json,os,queue,signal,subprocess,sys,tempfile,threading,time,urllib.parse
signal.pthread_sigmask(signal.SIG_SETMASK,[])
from pathlib import Path
binary=Path(sys.argv[1] if len(sys.argv)>1 else 'build-fltk/penguin-mail-engine').resolve();checks=[]
def check(ok,label):
    assert ok,label
    checks.append(label);print('PASS',label,flush=True)
with tempfile.TemporaryDirectory(prefix='penguin-native-oauth-') as temp:
    env={**os.environ,'MAILRS_DATA_DIR':temp+'/mail','MAILRS_CONFIG':temp+'/config.toml','GNUPGHOME':temp+'/gnupg','XDG_CONFIG_HOME':temp+'/config','XDG_DATA_HOME':temp+'/data','XDG_CACHE_HOME':temp+'/cache','LANG':'C.UTF-8'}
    for key in ['PENGUIN_MAIL_GOOGLE_CLIENT_ID','PENGUIN_MAIL_GOOGLE_CLIENT_SECRET']:env.pop(key,None)
    p=subprocess.Popen([str(binary)],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True);output=queue.Queue()
    def reader():
        for line in p.stdout:output.put(json.loads(line))
    threading.Thread(target=reader,daemon=True).start();index=0
    def send(op,**args):
        global index
        index+=1;p.stdin.write(json.dumps(dict(id=index,op=op,**args))+'\n');p.stdin.flush();return index
    def until(fn):
        end=time.monotonic()+15
        while time.monotonic()<end:
            r=output.get(timeout=max(.1,end-time.monotonic()))
            if fn(r):return r
        raise TimeoutError('worker reply')
    def request(op,**args):
        id=send(op,**args);r=until(lambda r:r.get('id')==id)
        assert r.get('ok'),r.get('error')
        return r['result']
    try:
        status=request('oauth-status');print('CONFIGURATION',json.dumps(status),flush=True)
        check(isinstance(status['google'],bool),'native worker reports whether this exact build has a Google client')
        if not status['google']:
            id=send('add-browser',provider='google');r=until(lambda r:r.get('id')==id)
            check(not r['ok'] and 'Google sign-in' in r['error'],'missing client is reported before a browser flow starts')
        client=Path(temp)/'client.json';client.write_text(json.dumps({'installed':{'client_id':'native-test.apps.googleusercontent.com','client_secret':'fixture-only','auth_uri':'https://example.invalid/steal','token_uri':'https://example.invalid/steal'}}));client.chmod(0o600)
        for wrong_state in [True,False]:
            id=send('add-browser',provider='google',client_file=str(client))
            event=until(lambda r:r.get('event')=='authorize');url=urllib.parse.urlparse(event['url']);q=urllib.parse.parse_qs(url.query)
            check(url.hostname=='accounts.google.com' and q['client_id']==['native-test.apps.googleusercontent.com'],'picked Desktop client opens the genuine Google authorization endpoint')
            check(q['code_challenge_method']==['S256'] and len(q['code_challenge'][0])==43 and bool(q['state'][0]),'browser sign-in uses PKCE S256 and random state')
            check('https://mail.google.com/' in q['scope'][0] and 'https://www.googleapis.com/auth/calendar' in q['scope'][0],'Gmail and calendar permissions are included in the browser request')
            redirect=urllib.parse.urlparse(q['redirect_uri'][0]);check(redirect.hostname=='127.0.0.1' and redirect.port>0,'authorization returns to a private ephemeral loopback port')
            params={'code':'not-a-real-code','state':'wrong'} if wrong_state else {'error':'access_denied','state':q['state'][0]}
            conn=http.client.HTTPConnection(redirect.hostname,redirect.port,timeout=4)
            conn.request('GET',(redirect.path or '/')+'?'+urllib.parse.urlencode(params));response=conn.getresponse();response.read();conn.close()
            r=until(lambda r:r.get('id')==id)
            check(not r['ok'] and ('state' if wrong_state else 'access_denied') in r['error'],'wrong-state callback fails closed' if wrong_state else 'declining Google consent exits cleanly')
        check(request('accounts')['items']==[],'OAuth diagnostics save no accounts or tokens')
    finally:
        p.stdin.close()
        try:p.wait(timeout=8)
        except subprocess.TimeoutExpired:
            p.terminate()
            try:p.wait(timeout=4)
            except subprocess.TimeoutExpired:p.kill();p.wait(timeout=4)
        error=p.stderr.read()
        if error:print(error,file=sys.stderr)
    check(p.returncode==0,'sign-in worker exits cleanly')
print(json.dumps({'passed':len(checks),'checks':checks,'configuration':status,'live_google_consent_tested':False},indent=2))
