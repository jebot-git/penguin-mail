#!/usr/bin/env python3
"""Check the opt-in native window, real history, text selection and exported menu."""
import hashlib,json,os,pathlib,queue,re,signal,sqlite3,ssl,subprocess,sys,tempfile,time
from Xlib import X,XK,display,error
from Xlib.ext import xtest
from PIL import Image
signal.pthread_sigmask(signal.SIG_SETMASK,[])
if os.environ.get('MISFIN_UI_BUS')!='1':
    os.execvpe('dbus-run-session',['dbus-run-session','--',sys.executable,*sys.argv],{**os.environ,'MISFIN_UI_BUS':'1'})
repo=pathlib.Path(__file__).resolve().parents[2]
binary=pathlib.Path(sys.argv[1] if len(sys.argv)>1 else repo/'build-fltk-misfin/penguin-mail').resolve()
art=repo/'test-artifacts/misfin';art.mkdir(parents=True,exist_ok=True)
children=[];checks=[]
def check(value,name):
    assert value,name
    checks.append(name);print('PASS '+name,flush=True)
def wait(fn):
    until=time.monotonic()+15
    while time.monotonic()<until:
        value=fn()
        if value:return value
        time.sleep(.08)
    raise AssertionError('Timed out')
try:
    with tempfile.TemporaryDirectory(prefix='misfin-ui-') as name:
        root=pathlib.Path(name)
        env={**os.environ,'MAILRS_DATA_DIR':name+'/mail','MAILRS_CONFIG':name+'/config.toml','XDG_CONFIG_HOME':name+'/config','XDG_DATA_HOME':name+'/data','XDG_CACHE_HOME':name+'/cache','GNUPGHOME':name+'/gnupg','LANG':'C.UTF-8','LC_ALL':'C.UTF-8','FLTK_BACKEND':'x11'}
        env.pop('WAYLAND_DISPLAY',None)
        cert=root/'id.crt';key=root/'id.key'
        subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-days','1','-subj','/UID=alice/CN=Misfin UI Fixture','-addext','subjectAltName=DNS:localhost','-keyout',str(key),'-out',str(cert)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,check=True)
        shared_cert=root/'browser.crt';shared_key=root/'browser.key'
        subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-days','1','-subj','/UID=browser/CN=Servo fixture','-addext','subjectAltName=DNS:localhost','-keyout',str(shared_key),'-out',str(shared_cert)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,check=True)
        os.chmod(shared_key,0o600);shared_original=shared_key.read_bytes()
        shared_id=hashlib.sha256(ssl.PEM_cert_to_DER_cert(shared_cert.read_text())).hexdigest()
        catalog=root/'data/de-xr/identities/index.json';catalog.parent.mkdir(parents=True,mode=0o700)
        catalog.write_text(json.dumps({'version':1,'identities':[{'id':shared_id,'name':'Servo fixture','certificate':str(shared_cert),'key':str(shared_key)}]}));os.chmod(catalog,0o600)
        archive=root/'inbox.gembox';archive.write_text('bob@localhost Bob\nalice@localhost\n2026-10-24T12:00:00Z\n# Misfin UI Fixture\nSelectable Unicode: Žluťoučký 世界\n')
        engine=subprocess.Popen([str(binary.parent/'penguin-mail-engine')],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True,env=env);children.append(engine)
        def request(index,op,**values):
            engine.stdin.write(json.dumps({'id':index,'op':op,**values})+'\n');engine.stdin.flush()
            while True:
                reply=json.loads(engine.stdout.readline())
                if reply.get('id')==index:
                    assert reply['ok'],reply;return reply['result']
        ident=request(1,'misfin-import-identity',certificate=str(cert),key=str(key))['identity']
        request(2,'misfin-import-mail',identity=ident,path=str(archive));engine.stdin.close();engine.wait(timeout=5)
        saved=root/'mail/misfin/state.json'
        legacy=json.loads(saved.read_text())
        for identity in legacy['identities']:identity.pop('inbox_url',None)
        saved.write_text(json.dumps(legacy))
        original=saved.read_bytes()
        db=sqlite3.connect(root/'mail/mailrs.db')
        db.execute("INSERT INTO accounts (email,state,provider,added_at) VALUES ('email@example.test','needs_reauth','gmail',0)")
        db.commit();db.close()
        rd,wr=os.pipe();xvfb=subprocess.Popen(['Xvfb','-displayfd',str(wr),'-screen','0','1600x1000x24','-nolisten','tcp'],pass_fds=(wr,),stderr=subprocess.DEVNULL);children.append(xvfb);os.close(wr)
        with os.fdopen(rd) as f:env['DISPLAY']=':'+f.readline().strip()
        xd=display.Display(env['DISPLAY'])
        with (art/'ui.log').open('w') as log:
            app=subprocess.Popen([str(binary)],env=env,stdout=log,stderr=log);children.append(app)
            def windows():return xd.screen().root.query_tree().children
            def win(title):return wait(lambda:next((w for w in windows() if title in str(w.get_wm_name() or '')),None))
            def call(w,method,*args):
                service=wait(lambda:w.get_full_property(xd.intern_atom('_KDE_NET_WM_APPMENU_SERVICE_NAME'),X.AnyPropertyType));path=w.get_full_property(xd.intern_atom('_KDE_NET_WM_APPMENU_OBJECT_PATH'),X.AnyPropertyType)
                return subprocess.check_output(['gdbus','call','--session','--dest',bytes(service.value).decode(),'--object-path',bytes(path.value).decode(),'--method','com.canonical.dbusmenu.'+method,'--',*args],text=True)
            def action(w,label):
                layout=call(w,'GetLayout','0','-1','[]');match=re.search(r"\((\d+), \{'label': <'"+re.escape(label)+"'>",layout);assert match,label
                call(w,'Event',match[1],'clicked','<0>','0')
            def item_id(w,label):
                layout=call(w,'GetLayout','0','-1','[]')
                match=re.search(r"\((\d+), \{'label': <'"+re.escape(label)+"'>",layout)
                assert match,label
                return match[1]
            def enabled(w,label):return 'true' in call(w,'GetProperty',item_id(w,label),'enabled')
            def key(name):
                code=xd.keysym_to_keycode(XK.string_to_keysym(name))
                xtest.fake_input(xd,X.KeyPress,code);xtest.fake_input(xd,X.KeyRelease,code);xd.sync()
            def click(w,x,y):
                w.set_input_focus(X.RevertToParent,X.CurrentTime);w.configure(stack_mode=X.Above)
                g=w.get_geometry();xtest.fake_input(xd,X.MotionNotify,x=g.x+x,y=g.y+y)
                xtest.fake_input(xd,X.ButtonPress,1);xtest.fake_input(xd,X.ButtonRelease,1);xd.sync();time.sleep(.15)
            def type_text(text):
                shift=xd.keysym_to_keycode(XK.string_to_keysym('Shift_L'))
                for c in text:
                    shifted=c in '@:'
                    if shifted:xtest.fake_input(xd,X.KeyPress,shift)
                    symbol={'@':'2',':':'semicolon','/':'slash','.':'period',' ':'space'}.get(c,c)
                    code=xd.keysym_to_keycode(XK.string_to_keysym(symbol))
                    xtest.fake_input(xd,X.KeyPress,code);xtest.fake_input(xd,X.KeyRelease,code)
                    if shifted:xtest.fake_input(xd,X.KeyRelease,shift)
                xd.sync();time.sleep(.2)
            def shot(w,name):
                g=w.get_geometry();raw=w.get_image(0,0,g.width,g.height,X.ZPixmap,0xffffffff)
                Image.frombytes('RGB',(g.width,g.height),raw.data,'raw','BGRX').save(art/name)
            def hidden(w):
                try:return w.get_attributes().map_state!=X.IsViewable
                except error.BadWindow:return True
            main=win('Penguin Mail');time.sleep(.7)
            layout=call(main,'GetLayout','0','-1','[]')
            check('Misfin Messaging' not in layout,'Misfin uses normal accounts without a separate messaging menu')
            # All Accounts contains the preexisting incoming Gemmail.
            click(main,300,115);time.sleep(.5)
            check(enabled(main,'Reply') and not enabled(main,'Archive') and not enabled(main,'Reply All'),
                  'Misfin rows in All Accounts disable unsupported message actions')
            # Select the imported Misfin account after the offline email fixture.
            click(main,520,54);key('Home');key('Down');key('Down');key('Return');time.sleep(.6)
            check(enabled(main,'Account Settings…') and not enabled(main,'Calendar') and not enabled(main,'Contacts'),
                  'Misfin is selectable as an account and disables unrelated account views')
            click(main,300,115);time.sleep(.4);shot(main,'account.png')
            # The ordinary reader retains Unicode selection and copy.
            click(main,800,360)
            ctrl=xd.keysym_to_keycode(XK.string_to_keysym('Control_L'))
            xtest.fake_input(xd,X.KeyPress,ctrl);key('a');key('c');xtest.fake_input(xd,X.KeyRelease,ctrl);xd.sync();time.sleep(.2)
            requestor=xd.screen().root.create_window(0,0,1,1,0,xd.screen().root_depth,X.InputOutput,X.CopyFromParent)
            prop=xd.intern_atom('MISFIN_TEST_SELECTION');target=xd.intern_atom('UTF8_STRING')
            def copied():
                while xd.pending_events():
                    event=xd.next_event()
                    if event.type==X.SelectionNotify:
                        if event.property==X.NONE:return None
                        value=requestor.get_full_property(prop,target)
                        return bytes(value.value).decode('utf-8') if value else None
                return None
            def clipboard():
                requestor.convert_selection(xd.intern_atom('CLIPBOARD'),target,prop,X.CurrentTime);xd.flush()
                return wait(copied)
            check('Žluťoučký 世界' in clipboard(),'Existing Misfin mail opens with Unicode selection in the normal reader')
            action(main,'Reply');compose=win('Compose Message');time.sleep(.4)
            check(not enabled(compose,'Save Draft') and not enabled(compose,'Send Later…') and enabled(compose,'Send'),
                  'Standard Misfin composer disables drafts and scheduling while allowing Send')
            click(compose,240,144);action(compose,'Select All');action(compose,'Copy');time.sleep(.2)
            check(clipboard()=='bob@localhost','Reply uses the mailbox address without the Gemmail display name')
            shot(compose,'compose-misfin.png')
            # Uncheck Misfin to select email, then check it again to select Misfin.
            click(compose,650,55);time.sleep(.5)
            check(enabled(compose,'Save Draft') and enabled(compose,'Send Later…'),
                  'Unchecking Misfin switches to email and restores email composer actions')
            click(compose,650,55);time.sleep(.4)
            check(not enabled(compose,'Save Draft'),'Checking Misfin selects its identity and reapplies protocol limitations')
            action(compose,'Close');time.sleep(.3)
            # FLTK asks before discarding because Misfin has no draft support.
            key('Right');key('Return');time.sleep(.3)
            if not hidden(compose):
                # Choose Discard by its keyboard mnemonic if the dialog focused Cancel.
                key('d');key('Return')
            wait(lambda:hidden(compose))
            action(main,'Account Settings…');settings=win('Account Settings');time.sleep(.3)
            action(settings,'Copy Identity Certificate');time.sleep(.2)
            certificate=clipboard()
            check('BEGIN CERTIFICATE' in certificate and 'PRIVATE KEY' not in certificate,
                  'Account settings copy the preserved public certificate without its private key')
            check(saved.read_bytes()==original,'Opening existing accounts and composing do not rewrite or migrate stored identities')
            click(settings,250,70);type_text('gemini://localhost/mail.gembox')
            click(settings,715,315);wait(lambda:hidden(settings));time.sleep(.3)
            check(json.loads(saved.read_text())['identities'][0]['inbox_url']=='gemini://localhost/mail.gembox',
                  'Misfin receiving configuration is saved with the existing identity')
            click(main,520,54);key('Up');key('Return');time.sleep(.4)
            check(enabled(main,'Calendar') and enabled(main,'Contacts'),'Selecting email restores account views disabled for Misfin')
            click(main,50,142);time.sleep(.3)
            # Add Account offers Misfin alongside the existing email protocols.
            action(main,'Add Account…');creation=win('Add Account');time.sleep(.3)
            click(creation,600,36)
            for _ in range(4):key('Down')
            key('Return');time.sleep(.3)
            click(creation,260,94);type_text('new@localhost')
            click(creation,260,134);type_text('new identity')
            shot(creation,'add-account-misfin.png')
            click(creation,620,602)
            wait(lambda:len(json.loads(saved.read_text())['identities'])==2)
            wait(lambda:hidden(creation));time.sleep(.4)
            check(any(i['address']=='new@localhost' for i in json.loads(saved.read_text())['identities']),
                  'Add Account creates and selects a Misfin identity through the normal account form')
            check(enabled(main,'Account Settings…'),'New Misfin accounts retain their normal account settings action')
            action(main,'Add Account…');shared_form=win('Add Account');time.sleep(.3)
            click(shared_form,600,36)
            for _ in range(4):key('Down')
            key('Return');time.sleep(.3)
            click(shared_form,120,602);shared_dialog=win('Import Shared Misfin Identity');time.sleep(.3)
            shot(shared_dialog,'shared-identity.png');click(shared_dialog,510,242)
            wait(lambda:len(json.loads(saved.read_text())['identities'])==3)
            wait(lambda:hidden(shared_form));time.sleep(.3)
            imported=next(i for i in json.loads(saved.read_text())['identities'] if i['id']==shared_id)
            check(imported['address']=='browser@localhost' and shared_key.read_bytes()==shared_original,
                  'Native shared identity chooser imports the browser certificate and preserves its source key')
            requestor.destroy()
            action(main,'Quit');app.wait(timeout=10);check(app.returncode==0,'Native application shuts down cleanly')
            xd.close()
        (art/'ui-report.json').write_text(json.dumps(checks,indent=2)+'\n')
finally:
    for p in reversed(children):
        if p.poll() is None:
            p.terminate()
            try:p.wait(timeout=5)
            except subprocess.TimeoutExpired:p.kill();p.wait()
