#!/usr/bin/env python3
"""Exercise the real DomeD clock and date handoff in a private Wayland session."""
import calendar,datetime,json,os,re,signal,socket,subprocess,sys,tempfile,time,urllib.request
from pathlib import Path
from Xlib import X,XK,display
from Xlib.ext import xtest
from PIL import Image
signal.pthread_sigmask(signal.SIG_SETMASK,[])
if os.environ.get('PENGUIN_CLOCK_BUS')!='1':os.execvpe('dbus-run-session',['dbus-run-session','--',sys.executable,*sys.argv],{**os.environ,'PENGUIN_CLOCK_BUS':'1'})
repo=Path(__file__).resolve().parents[2];root=repo.parent;art=repo/'test-artifacts';art.mkdir(parents=True,exist_ok=True);processes=[];logs=[];checks=[]
canonical=root.parent/'dome-compositor/target/release/dome-compositor'
default_dome=canonical if canonical.is_file() else root/'dome-compositor/target/debug/dome-compositor'
def start(args,name,env,**kw):
 log=(art/name).open('w');logs.append(log);p=subprocess.Popen(list(map(str,args)),env=env,stdout=log,stderr=log,**kw);processes.append(p);return p
def wait(fn,seconds=25):
 end=time.monotonic()+seconds
 while time.monotonic()<end:
  try:
   result=fn()
   if result:return result
  except (OSError,ValueError,KeyError,StopIteration):pass
  time.sleep(.1)
 raise AssertionError('Timed out')
def check(ok,label):
 assert ok,label
 checks.append(label);print('PASS',label,flush=True)
try:
 with tempfile.TemporaryDirectory(prefix='dome-penguin-clock-') as tmp:
  rd,wr=os.pipe();start(['Xvfb','-displayfd',wr,'-screen','0','1600x1000x24','-nolisten','tcp'],'penguin-clock-xvfb.log',os.environ.copy(),pass_fds=(wr,));os.close(wr)
  with os.fdopen(rd) as f:ds=':'+f.readline().strip()
  with socket.socket() as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
  env={**os.environ,'DISPLAY':ds,'XDG_RUNTIME_DIR':tmp,'XDG_CONFIG_HOME':tmp+'/config','XDG_DATA_HOME':tmp+'/data','XDG_CACHE_HOME':tmp+'/cache','XDG_STATE_HOME':tmp+'/state','GNUPGHOME':tmp+'/gnupg','MAILRS_DATA_DIR':tmp+'/mail','MAILRS_CONFIG':tmp+'/config.toml','DOME_LOGIN_SESSION':'0','DOME_START_XR':'0','DOME_START_IME':'0','DOME_RENDERER':'gles','DOME_PREVIEW_PORT':str(port),'LIBGL_ALWAYS_SOFTWARE':'1','LD_LIBRARY_PATH':str(root/'_toolkit/lib'),'DOME_PENGUIN_MAIL':str(repo/'build-fltk/penguin-mail'),'LANG':'C.UTF-8','XMODIFIERS':'@im=none'}
  for key in ['WAYLAND_DISPLAY','WAYLAND_SOCKET','FLTK_BACKEND','QT_QPA_PLATFORM','GTK_MODULES']:env.pop(key,None)
  dome=start([Path(os.environ.get('DOME_TEST_BIN',str(default_dome))),'--backend','nested'],'penguin-clock-dome.log',env)
  def scene():return json.load(urllib.request.urlopen(f'http://127.0.0.1:{port}/api/state',timeout=2))['scene']
  wait(lambda:scene()['source']=='wayland')
  xd=display.Display(ds);host=wait(lambda:next((w for w in xd.screen().root.query_tree().children if str(w.get_wm_name() or '').startswith('Dome')),None));host.set_input_focus(X.RevertToParent,X.CurrentTime);xd.sync()
  def click(px,py):
   g=host.get_geometry();xtest.fake_input(xd,X.MotionNotify,x=g.x+int(px),y=g.y+int(py));xd.sync();time.sleep(.05)
   xtest.fake_input(xd,X.ButtonPress,1);xtest.fake_input(xd,X.ButtonRelease,1);xd.sync();time.sleep(.35)
  def shot(name):
   g=host.get_geometry();raw=host.get_image(0,0,g.width,g.height,X.ZPixmap,0xffffffff);im=Image.frombytes('RGB',(g.width,g.height),raw.data,'raw','BGRX');im.save(art/(name+'.png'));return im
  g=host.get_geometry();left=g.width-400;top=42
  closed=shot('penguin-clock-closed');click(g.width-80,18);opened=shot('penguin-clock-calendar')
  check(closed.crop((left,top,g.width-8,412)).tobytes()!=opened.crop((left,top,g.width-8,412)).tobytes(),'clicking the clock opens a native shell calendar popup')
  click(left+170,top+345)
  for row in range(4):click(left+100,top+370+row*32+16)
  settings=Path(tmp)/'config/dome/clock.json';saved=wait(lambda:json.loads(settings.read_text()) if settings.exists() else None)
  check(saved=={'twelve_hour':True,'seconds':True,'weekday':False,'date':True},'all four time display settings persist with the expected values')
  time.sleep(1);shot('penguin-clock-settings')
  # Collapse settings, move a month, and select its 15th in the actual hit grid.
  click(left+170,top+345);click(left+370,top+20)
  today=datetime.date.today();year=today.year+(today.month==12);month=today.month%12+1
  wanted=datetime.date(year,month,15);first=datetime.date(year,month,1);index=first.weekday()+14
  click(left+14+(index%7+.5)*(392-28)/7,top+72+(index//7+.5)*35)
  windows=wait(lambda:[w for w in scene()['windows'] if 'Penguin Mail' in w['title'] and wanted.isoformat() in w['title']])
  check(len(windows)==1,'clicking a day launches Penguin Mail calendar at that exact date')
  check(windows[0]['app_id']=='io.github.c9dev.PenguinMail','Penguin Mail maps as a native Wayland client with its desktop identity')
  time.sleep(.4);shot('penguin-clock-date-handoff')
  # Open another day while the same application runs. Its single-instance bus must reuse it.
  click(g.width-80,18);today=datetime.date.today();index=datetime.date(today.year,today.month,1).weekday()+8
  click(left+14+(index%7+.5)*(392-28)/7,top+72+(index//7+.5)*35)
  date=datetime.date(today.year,today.month,9).isoformat();wait(lambda:any('Penguin Mail' in w['title'] and date in w['title'] for w in scene()['windows']))
  check(sum('Penguin Mail' in w['title'] for w in scene()['windows'])==1,'a later clock date activates the existing Penguin Mail instance')
  layout=subprocess.check_output(['gdbus','call','--session','--dest','io.github.c9dev.PenguinMail.Fltk','--object-path','/org/de_xr/FLTK/Menu1','--method','com.canonical.dbusmenu.GetLayout','--','0','-1','[]'],text=True)
  close=re.search(r"\((\d+), \{'label': <'Close Window'>",layout);check(bool(close),'Wayland menu association exports the real Close Window action')
  subprocess.run(['gdbus','call','--session','--dest','io.github.c9dev.PenguinMail.Fltk','--object-path','/org/de_xr/FLTK/Menu1','--method','com.canonical.dbusmenu.Event','--',close[1],'clicked','<0>','0'],check=True,stdout=subprocess.DEVNULL)
  wait(lambda:not any('Penguin Mail' in w['title'] for w in scene()['windows']))
  check(True,'closing the mail window hides it while DomeD hosts its tray item')
  clock_width=max(176,len(datetime.datetime.now().strftime("%Y-%m-%d %I:%M:%S %p"))*8+24)
  click(g.width-clock_width-44-14,18)
  wait(lambda:any('Penguin Mail' in w['title'] for w in scene()['windows']))
  check(True,'clicking the native DomeD tray item restores Penguin Mail')
  check(dome.poll() is None,'clock interactions and settings keep the compositor running')
  xd.close()
finally:
 for p in reversed(processes):
  if p.poll() is None:
   p.terminate()
   try:p.wait(timeout=4)
   except subprocess.TimeoutExpired:p.kill();p.wait()
 for log in logs:log.close()
(art/'penguin-clock-ui-tests.json').write_text(json.dumps(checks,indent=2)+'\n')
