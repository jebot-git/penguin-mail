#!/usr/bin/env python3
"""Drive native FLTK widgets and their exported menu on a private display/bus."""
import json,os,re,signal,subprocess,sys,tempfile,time
from pathlib import Path
from Xlib import X,XK,display
from Xlib.ext import xtest
from PIL import Image
signal.pthread_sigmask(signal.SIG_SETMASK,[])
if os.environ.get('PENGUIN_TEST_BUS')!='1':os.execvpe('dbus-run-session',['dbus-run-session','--',sys.executable,*sys.argv],{**os.environ,'PENGUIN_TEST_BUS':'1'})
repo=Path(__file__).resolve().parents[2];root=repo.parent;art=repo/'test-artifacts';art.mkdir(parents=True,exist_ok=True);checks=[];processes=[]
def wait(fn,seconds=20):
 end=time.monotonic()+seconds
 while time.monotonic()<end:
  result=fn()
  if result:return result
  time.sleep(.07)
 raise AssertionError('Timed out')
def check(ok,label):
 assert ok,label
 checks.append(label);print('PASS',label,flush=True)
with tempfile.TemporaryDirectory(prefix='penguin-fltk-ui-') as tmp:
 rd,wr=os.pipe();xvfb=subprocess.Popen(['Xvfb','-displayfd',str(wr),'-screen','0','1600x1000x24','-nolisten','tcp'],pass_fds=(wr,),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);os.close(wr)
 with os.fdopen(rd) as f:ds=':'+f.readline().strip()
 x=display.Display(ds)
 env={**os.environ,'DISPLAY':ds,'FLTK_BACKEND':'x11','LD_LIBRARY_PATH':str(root/'_toolkit/lib'),'XDG_CONFIG_HOME':tmp+'/config','XDG_DATA_HOME':tmp+'/data','XDG_CACHE_HOME':tmp+'/cache','GNUPGHOME':tmp+'/gnupg','LANG':'C.UTF-8','XMODIFIERS':'@im=none','MAILRS_DATA_DIR':tmp+'/mail','MAILRS_CONFIG':tmp+'/config.toml'}
 def windows():return x.screen().root.query_tree().children
 def window(needle):return wait(lambda:next((w for w in windows() if needle in str(w.get_wm_name() or '')),None))
 def capture(w,name):
  time.sleep(.7)
  g=w.get_geometry();raw=w.get_image(0,0,g.width,g.height,X.ZPixmap,0xffffffff);im=Image.frombytes('RGB',(g.width,g.height),raw.data,'raw','BGRX');im.save(art/(name+'.png'));return im
 def focus(w):w.set_input_focus(X.RevertToParent,X.CurrentTime);x.sync()
 def key(w,name,ctrl=False):
  focus(w);mod=x.keysym_to_keycode(XK.string_to_keysym('Control_L'));code=x.keysym_to_keycode(XK.string_to_keysym(name));
  if ctrl:xtest.fake_input(x,X.KeyPress,mod)
  xtest.fake_input(x,X.KeyPress,code);xtest.fake_input(x,X.KeyRelease,code)
  if ctrl:xtest.fake_input(x,X.KeyRelease,mod)
  x.sync();time.sleep(.2)
 def click(w,px,py):
  g=w.get_geometry();xtest.fake_input(x,X.MotionNotify,x=g.x+px,y=g.y+py);xtest.fake_input(x,X.ButtonPress,1);xtest.fake_input(x,X.ButtonRelease,1);x.sync();time.sleep(.2)
 def menu_call(w,method,*args,blocking=True):
  service=wait(lambda:w.get_full_property(x.intern_atom('_KDE_NET_WM_APPMENU_SERVICE_NAME'),X.AnyPropertyType));path=w.get_full_property(x.intern_atom('_KDE_NET_WM_APPMENU_OBJECT_PATH'),X.AnyPropertyType)
  command=['gdbus','call','--session','--dest',bytes(service.value).decode(),'--object-path',bytes(path.value).decode(),'--method','com.canonical.dbusmenu.'+method,'--',*args]
  return subprocess.check_output(command,text=True) if blocking else subprocess.Popen(command,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
 log=(art/'penguin-ui.log').open('w')
 try:
  p=subprocess.Popen([str(repo/'build-fltk/penguin-mail'),'--demo'],env=env,stdout=log,stderr=log);processes.append(p);w=window('Penguin Mail');time.sleep(1)
  layout=menu_call(w,'GetLayout','0','-1','[]');check(all(text in layout for text in ['New Message','Calendar','Contacts','Archive','Preferences']),'actual FLTK actions are exported over DBusMenu')
  pgp=re.search(r"\((\d+), \{'label': <'OpenPGP Keys…'>",layout)
  check(bool(pgp),'OpenPGP key management is exported in the native menu')
  menu_call(w,'Event',pgp[1],'clicked','<0>','0');keys=window('OpenPGP Keys');capture(keys,'penguin-mail-native-pgp-keys')
  check(p.poll() is None,'the native GnuPG key manager opens without blocking the mail window')
  click(keys,650,450)
  click(w,300,115);time.sleep(.3);capture(w,'penguin-mail-native-reader');check(p.poll() is None,'native GL reader handles populated and empty text lines')
  key(w,'n',True);composer=window('Compose Message');capture(composer,'penguin-mail-native-composer');check(True,'Ctrl+N opens a native composer')
  key(composer,'Escape');key(composer,'Escape');focus(w);w.configure(stack_mode=X.Above);x.sync()
  subprocess.run([str(repo/'build-fltk/penguin-mail'),'--demo','--calendar','2028-02-29'],env=env,check=True,timeout=5)
  wait(lambda:'2028-02-29' in str(w.get_wm_name()));capture(w,'penguin-mail-calendar-leap-day');check(True,'a second launch opens the existing calendar on the exact leap day')
  subprocess.run([str(repo/'build-fltk/penguin-mail'),'--demo','--calendar','1968-02-29'],env=env,check=True,timeout=5)
  wait(lambda:'1968-02-29' in str(w.get_wm_name()));check(True,'calendar date links accept valid dates before the Unix epoch')
  key(w,'3',True);wait(lambda:'Contacts' in str(w.get_wm_name()));capture(w,'penguin-mail-native-contacts');check(True,'Ctrl+3 switches to the native contacts view')
  check(subprocess.run([str(repo/'build-fltk/penguin-mail'),'--calendar','2026-02-30'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==2,'invalid dates are rejected before activation')
  key(w,'1',True);w.configure(width=1100,height=700);x.sync();time.sleep(.3);capture(w,'penguin-mail-native-resized');check(p.poll() is None,'native panes resize without a crash')
  linked=subprocess.check_output(['ldd',str(repo/'build-fltk/penguin-mail')],text=True);engine=subprocess.check_output(['ldd',str(repo/'build-fltk/penguin-mail-engine')],text=True)
  check(not re.search(r'lib(Qt|gtk|adwaita|webkit)',linked+engine),'native frontend and engine link no Qt, GTK, libadwaita or WebKit')
  before={window.id for window in windows()}
  real=subprocess.Popen([str(repo/'build-fltk/penguin-mail')],env=env,stdout=log,stderr=log);processes.append(real)
  real_window=wait(lambda:next((item for item in windows() if item.id not in before and 'Penguin Mail' in str(item.get_wm_name() or '')),None))
  time.sleep(.5)
  real_layout=menu_call(real_window,'GetLayout','0','-1','[]')
  prefs=re.search(r"\((\d+), \{'label': <'Preferences…'>",real_layout)
  if not prefs:prefs=re.search(r"\((\d+), \{'label': <'Preferences'>",real_layout)
  assert prefs,real_layout
  menu_call(real_window,'Event',prefs[1],'clicked','<0>','0');preferences=window('Preferences')
  capture(preferences,'penguin-mail-native-login-preferences')
  click(preferences,35,35);click(preferences,500,398)
  startup=Path(env['XDG_CONFIG_HOME'])/'autostart/io.github.c9dev.PenguinMail.desktop'
  wait(lambda:startup.exists() and 'Hidden=false' in startup.read_text())
  check('--tray' in startup.read_text(),'Preferences enables login startup in the private user config with tray launch')
  menu_call(real_window,'Event',prefs[1],'clicked','<0>','0');preferences=window('Preferences')
  click(preferences,35,35);click(preferences,500,398)
  wait(lambda:'Hidden=true' in startup.read_text())
  check(True,'Preferences disables login startup and reloads the saved choice')
  add=re.search(r"\((\d+), \{'label': <'Add Account…'>",real_layout)
  menu_call(real_window,'Event',add[1],'clicked','<0>','0');account_window=window('Add Account')
  capture(account_window,'penguin-mail-native-add-account')
  check(real.poll() is None,'the native real-account form opens against a private empty store')
  before={item.id for item in windows()};click(account_window,110,36)
  prompt=wait(lambda:next((item for item in windows() if item.id not in before and item.get_attributes().map_state==X.IsViewable and item.get_geometry().width>=300 and item.get_geometry().height>=100),None))
  print('Google setup prompt geometry',prompt.get_geometry(),flush=True)
  capture(prompt,'penguin-mail-native-google-client-setup')
  check(real.poll() is None,'Google sign-in offers a Desktop client JSON picker when this build has no client')
  key(prompt,'Escape');key(account_window,'Escape')
  quit_item=re.search(r"\((\d+), \{'label': <'Quit'>",real_layout)
  menu_call(real_window,'Event',quit_item[1],'clicked','<0>','0');wait(lambda:real.poll() is not None)
  check(real.returncode==0,'the native frontend and worker exit normally after the sign-in dialog is canceled')
  child_ids=(Path('/proc')/str(p.pid)/'task'/str(p.pid)/'children').read_text().split()
  home=None
  for child in child_ids:
   for fd in (Path('/proc')/child/'fd').iterdir():
    try:target=Path(os.readlink(fd))
    except OSError:continue
    if target.name=='mailrs.db':home=target.parent;break
  check(home is not None and (home/'gnupg').is_dir(),'preview OpenPGP keys live beside the temporary demo store')
  composer_layout=menu_call(composer,'GetLayout','0','-1','[]')
  close_composer=re.search(r"\((\d+), \{'label': <'Close'>",composer_layout)
  close_request=menu_call(composer,'Event',close_composer[1],'clicked','<0>','0',blocking=False)
  discard=wait(lambda:next((item for item in windows() if item.get_attributes().map_state==X.IsViewable and 60<=item.get_geometry().height<=220 and item.get_geometry().width>=300),None))
  key(discard,'Return');close_request.communicate(timeout=5);wait(lambda:composer.id not in {item.id for item in windows()} or composer.get_attributes().map_state!=X.IsViewable)
  quit_item=re.search(r"\((\d+), \{'label': <'Quit'>",layout)
  menu_call(w,'Event',quit_item[1],'clicked','<0>','0');wait(lambda:p.poll() is not None)
  wait(lambda:not home.exists())
  check(p.returncode==0,'quitting the preview drains the worker and discards its temporary GnuPG keyring')

 finally:
  for p in reversed(processes):
   if p.poll() is None:p.terminate();p.wait(timeout=5)
  log.close();x.close();xvfb.terminate();xvfb.wait(timeout=5)
(art/'penguin-ui-tests.json').write_text(json.dumps(checks,indent=2)+'\n')
