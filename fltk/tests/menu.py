#!/usr/bin/env python3
"""Check native menu fallback and shortcuts against a private menu host."""
import asyncio,json,os,re,signal,subprocess,sys,tempfile,time
from pathlib import Path
if '--registrar' in sys.argv:
 from dbus_next.aio import MessageBus
 async def hold():
  bus=await MessageBus().connect();await bus.request_name('com.canonical.AppMenu.Registrar');print('ready',flush=True);await asyncio.Future()
 asyncio.run(hold());sys.exit()
from Xlib import X,XK,display
from Xlib.ext import xtest
from PIL import Image
signal.pthread_sigmask(signal.SIG_SETMASK,[])
if os.environ.get('PENGUIN_MENU_BUS')!='1':os.execvpe('dbus-run-session',['dbus-run-session','--',sys.executable,*sys.argv],{**os.environ,'PENGUIN_MENU_BUS':'1'})
repo=Path(__file__).resolve().parents[2];root=repo.parent;art=repo/'test-artifacts';art.mkdir(parents=True,exist_ok=True);processes=[];checks=[]
def wait(fn,seconds=60):
 end=time.monotonic()+seconds
 while time.monotonic()<end:
  v=fn()
  if v:return v
  time.sleep(.1)
 raise AssertionError('Timed out')
def check(ok,label):
 assert ok,label
 checks.append(label);print('PASS',label,flush=True)
with tempfile.TemporaryDirectory(prefix='penguin-menu-') as tmp:
 rd,wr=os.pipe();xvfb=subprocess.Popen(['Xvfb','-displayfd',str(wr),'-screen','0','1500x1000x24','-nolisten','tcp'],pass_fds=(wr,),stderr=subprocess.DEVNULL);processes.append(xvfb);os.close(wr)
 with os.fdopen(rd) as f:ds=':'+f.readline().strip()
 env={**os.environ,'DISPLAY':ds,'FLTK_BACKEND':'x11','XDG_CURRENT_DESKTOP':'Dome','LD_LIBRARY_PATH':str(root/'_toolkit/lib'),'XDG_RUNTIME_DIR':tmp,'XDG_CONFIG_HOME':tmp+'/config','XDG_DATA_HOME':tmp+'/data','XDG_CACHE_HOME':tmp+'/cache','MAILRS_DATA_DIR':tmp+'/mail','MAILRS_CONFIG':tmp+'/config.toml','LANG':'C.UTF-8','XMODIFIERS':'@im=none'}
 xd=display.Display(ds)
 def windows():return xd.screen().root.query_tree().children
 def win(label):return wait(lambda:next((w for w in windows() if label in str(w.get_wm_name() or '')),None))
 def shot(w,name):
  w.configure(stack_mode=X.Above);xd.sync();time.sleep(.4);g=w.get_geometry();raw=w.get_image(0,0,g.width,g.height,X.ZPixmap,0xffffffff);im=Image.frombytes('RGB',(g.width,g.height),raw.data,'raw','BGRX');im.save(art/(name+'.png'));return im
 def key(w,name,ctrl=False):
  w.set_input_focus(X.RevertToParent,X.CurrentTime);xd.sync();mod=xd.keysym_to_keycode(XK.string_to_keysym('Control_L'));code=xd.keysym_to_keycode(XK.string_to_keysym(name))
  if ctrl:xtest.fake_input(xd,X.KeyPress,mod)
  xtest.fake_input(xd,X.KeyPress,code);xtest.fake_input(xd,X.KeyRelease,code)
  if ctrl:xtest.fake_input(xd,X.KeyRelease,mod)
  xd.sync();time.sleep(.25)
 def call(w,method,*args,block=True):
  service=wait(lambda:w.get_full_property(xd.intern_atom('_KDE_NET_WM_APPMENU_SERVICE_NAME'),X.AnyPropertyType));path=w.get_full_property(xd.intern_atom('_KDE_NET_WM_APPMENU_OBJECT_PATH'),X.AnyPropertyType)
  cmd=['gdbus','call','--session','--dest',bytes(service.value).decode(),'--object-path',bytes(path.value).decode(),'--method','com.canonical.dbusmenu.'+method,'--',*args]
  return subprocess.check_output(cmd,text=True) if block else subprocess.Popen(cmd,stdout=subprocess.PIPE,text=True)
 def action(w,name,block=True):
  layout=call(w,'GetLayout','0','-1','[]');item=re.search(r"\((\d+), \{'label': <'"+re.escape(name)+"'>",layout);assert item,name
  return call(w,'Event',item[1],'clicked','<0>','0',block=block)
 log=(art/'penguin-menu-ui.log').open('w')
 try:
  app=subprocess.Popen([str(repo/'build-fltk/penguin-mail'),'--demo'],env=env,stdout=log,stderr=log);processes.append(app);w=win('Penguin Mail');time.sleep(1)
  local=shot(w,'penguin-menu-fallback');crop=(0,2,400,10)
  check(len(set(local.crop(crop).getdata()))>3,'Dome session without a universal-menu host retains a visible local menu')
  host=subprocess.Popen([sys.executable,__file__,'--registrar'],stdout=subprocess.PIPE,text=True);processes.append(host);assert host.stdout.readline().strip()=='ready';time.sleep(1)
  external=shot(w,'penguin-menu-universal');check(len(set(external.crop(crop).getdata()))<=3,'a working Dome universal-menu host collapses the local menu')
  check('New Message' in call(w,'GetLayout','0','-1','[]'),'collapsed menu continues exporting its actual commands')
  key(w,'n',True);composer=win('Compose Message');composer_image=shot(composer,'penguin-menu-composer-universal');check(len(set(composer_image.crop(crop).getdata()))<=3,'Ctrl+N still opens the composer and its local menu also collapses')
  host.terminate();host.wait(timeout=5);time.sleep(.8)
  restored=shot(w,'penguin-menu-restored');restored_composer=shot(composer,'penguin-menu-composer-restored');check(len(set(restored.crop(crop).getdata()))>3 and len(set(restored_composer.crop(crop).getdata()))>3,'losing the universal-menu host restores both local menus')
  key(w,'3',True);wait(lambda:'Contacts' in str(w.get_wm_name()));check(True,'local shortcuts still work after menu fallback')
  close=action(composer,'Close',False);time.sleep(.2)
  prompt=wait(lambda:next((i for i in windows() if i.get_attributes().map_state==X.IsViewable and 60<i.get_geometry().height<220),None));key(prompt,'Return');close.communicate(timeout=5)
  action(w,'Quit');wait(lambda:app.poll() is not None);check(app.returncode==0,'menu transitions and composer shutdown exit cleanly')
  # A non-Dome session retains its bar even if a registrar happens to exist.
  host=subprocess.Popen([sys.executable,__file__,'--registrar'],stdout=subprocess.PIPE,text=True);processes.append(host);host.stdout.readline()
  other=subprocess.Popen([str(repo/'build-fltk/penguin-mail'),'--demo'],env={**env,'XDG_CURRENT_DESKTOP':'XFCE'},stdout=log,stderr=log);processes.append(other);w=win('Penguin Mail');time.sleep(.8)
  image=shot(w,'penguin-menu-other-desktop');check(len(set(image.crop(crop).getdata()))>3,'another desktop keeps its local menu despite a registrar on the bus')
  action(w,'Quit');wait(lambda:other.poll() is not None)
 finally:
  xd.close()
  for p in reversed(processes):
   if p.poll() is None:
    p.terminate()
    try:p.wait(timeout=5)
    except subprocess.TimeoutExpired:p.kill();p.wait()
  log.close()
(art/'penguin-menu-ui-tests.json').write_text(json.dumps(checks,indent=2)+'\n')
