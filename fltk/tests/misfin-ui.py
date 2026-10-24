#!/usr/bin/env python3
"""Check the opt-in native window, real history, text selection and exported menu."""
import json,os,pathlib,queue,re,signal,subprocess,sys,tempfile,time
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
            main=win('Penguin Mail');time.sleep(.5)
            action(main,'Misfin Messaging…');form=win('Misfin Messaging');time.sleep(.6)
            layout=call(form,'GetLayout','0','-1','[]')
            check(all(word in layout for word in ['Send Misfin','Fetch Mailbox','Import Misfin Identity','Import Gemmail Archive']),'Misfin window exports its actual commands to the universal menu')
            form.set_input_focus(X.RevertToParent,X.CurrentTime);form.configure(stack_mode=X.Above);xd.sync()
            g=form.get_geometry();xtest.fake_input(xd,X.MotionNotify,x=g.x+100,y=g.y+530);xtest.fake_input(xd,X.ButtonPress,1);xtest.fake_input(xd,X.ButtonRelease,1);xd.sync();time.sleep(.4)
            raw=form.get_image(0,0,g.width,g.height,X.ZPixmap,0xffffffff)
            image=Image.frombytes('RGB',(g.width,g.height),raw.data,'raw','BGRX');image.save(art/'window.png')
            check(len(image.getcolors(image.width*image.height) or [])>8,'Misfin identity, history and reader render in the native window')
            # Reader supports the standard FLTK Select All/Copy actions.
            xtest.fake_input(xd,X.MotionNotify,x=g.x+500,y=g.y+585);xtest.fake_input(xd,X.ButtonPress,1);xtest.fake_input(xd,X.ButtonRelease,1)
            ctrl=xd.keysym_to_keycode(XK.string_to_keysym('Control_L'))
            xtest.fake_input(xd,X.KeyPress,ctrl)
            for letter in ('a','c'):
                code=xd.keysym_to_keycode(XK.string_to_keysym(letter));xtest.fake_input(xd,X.KeyPress,code);xtest.fake_input(xd,X.KeyRelease,code)
            xtest.fake_input(xd,X.KeyRelease,ctrl);xd.sync();time.sleep(.2)
            requestor=xd.screen().root.create_window(0,0,1,1,0,xd.screen().root_depth,X.InputOutput,X.CopyFromParent)
            prop=xd.intern_atom('MISFIN_TEST_SELECTION');target=xd.intern_atom('UTF8_STRING')
            requestor.convert_selection(xd.intern_atom('CLIPBOARD'),target,prop,X.CurrentTime);xd.flush()
            def copied():
                while xd.pending_events():
                    event=xd.next_event()
                    if event.type==X.SelectionNotify:
                        if event.property==X.NONE:return None
                        value=requestor.get_full_property(prop,target)
                        return bytes(value.value).decode('utf-8') if value else None
                return None
            text=wait(copied)
            check('Žluťoučký 世界' in text,'Gemmail reader supports Unicode text selection and clipboard copy')
            requestor.destroy()
            # Close through the exported window action and leave normal mail alive.
            def hidden():
                try:return form.get_attributes().map_state!=X.IsViewable
                except error.BadWindow:return True
            action(form,'Close');wait(hidden)
            check(app.poll() is None,'Closing Misfin leaves normal mail running')
            action(main,'Quit');app.wait(timeout=10);check(app.returncode==0,'Native application shuts down cleanly')
            xd.close()
        (art/'ui-report.json').write_text(json.dumps(checks,indent=2)+'\n')
finally:
    for p in reversed(children):
        if p.poll() is None:
            p.terminate()
            try:p.wait(timeout=5)
            except subprocess.TimeoutExpired:p.kill();p.wait()
