#!/usr/bin/env python3
"""Exercise FLTK notifications against the real canonical DomeD server privately."""
import asyncio,datetime,json,os,re,signal,socket,sqlite3,subprocess,sys,tempfile,threading,time,urllib.request
from pathlib import Path
signal.pthread_sigmask(signal.SIG_SETMASK,[])
if '--monitor' in sys.argv:
 from dbus_next.aio import MessageBus
 from dbus_next import Message,Variant
 def plain(v):
  if isinstance(v,Variant):return plain(v.value)
  if isinstance(v,(list,tuple,bytes)):return [plain(x) for x in v]
  if isinstance(v,dict):return {k:plain(x) for k,x in v.items()}
  return v
 async def monitor():
  bus=await MessageBus().connect();out=open(sys.argv[-1],'w')
  def record(m):
   out.write(json.dumps({'member':m.member,'type':m.message_type.name,'body':plain(m.body),'serial':m.serial,'reply_serial':m.reply_serial,'sender':m.sender,'destination':m.destination})+'\n');out.flush();return m.message_type.name=='METHOD_CALL'
  bus.add_message_handler(record)
  reply=await bus.call(Message(destination='org.freedesktop.DBus',path='/org/freedesktop/DBus',interface='org.freedesktop.DBus.Monitoring',member='BecomeMonitor',signature='asu',body=[["interface='org.freedesktop.Notifications'","type='method_return'"],0]));assert reply.message_type.name=='METHOD_RETURN',reply.body
  print('ready',flush=True);await asyncio.Future()
 asyncio.run(monitor());sys.exit()
if '--spoof' in sys.argv:
 from dbus_next.aio import MessageBus
 from dbus_next import Message
 async def spoof():
  bus=await MessageBus().connect();await bus.send(Message.new_signal('/org/freedesktop/Notifications','org.freedesktop.Notifications','ActionInvoked','us',[int(sys.argv[-1]),'default']));await asyncio.sleep(.1)
 asyncio.run(spoof());sys.exit()
from Xlib import X,XK,display
from Xlib.ext import xtest
from PIL import Image
if os.environ.get('PENGUIN_NOTICE_BUS')!='1':os.execvpe('dbus-run-session',['dbus-run-session','--',sys.executable,*sys.argv],{**os.environ,'PENGUIN_NOTICE_BUS':'1'})
repo=Path(__file__).resolve().parents[2];root=repo.parent
art=Path(os.environ.get('PENGUIN_NOTICE_ARTIFACTS',str(repo/'test-artifacts')));art.mkdir(parents=True,exist_ok=True)
binary=Path(os.environ.get('PENGUIN_TEST_BIN',str(repo/'build-fltk/penguin-mail'))).resolve()
processes=[];logs=[];checks=[]
def start(args,name,env,**kw):
 log=(art/name).open('w');logs.append(log);p=subprocess.Popen(list(map(str,args)),env=env,stdout=log,stderr=log,**kw);processes.append(p);return p
def wait(fn,seconds=25):
 end=time.monotonic()+seconds
 while time.monotonic()<end:
  try:
   result=fn()
   if result:return result
  except (OSError,ValueError,KeyError,StopIteration,sqlite3.OperationalError):pass
  time.sleep(.08)
 raise AssertionError('Timed out')
def check(ok,label):
 assert ok,label
 checks.append(label);print('PASS',label,flush=True)
xd=None
try:
 with tempfile.TemporaryDirectory(prefix='penguin-notifications-') as tmp:
  rd,wr=os.pipe();start(['Xvfb','-displayfd',wr,'-screen','0','1600x1000x24','-nolisten','tcp'],'penguin-notification-xvfb.log',os.environ.copy(),pass_fds=(wr,));os.close(wr)
  with os.fdopen(rd) as f:ds=':'+f.readline().strip()
  with socket.socket() as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
  bindir=Path(tmp)/'bin';bindir.mkdir();xdg=bindir/'xdg-open';xdg.write_text('#!/bin/sh\nprintf "%s\\n" "$1" >> "$PENGUIN_TEST_OPEN_LOG"\n');xdg.chmod(0o700)
  env={**os.environ,'DISPLAY':ds,'XDG_RUNTIME_DIR':tmp,'XDG_CONFIG_HOME':tmp+'/config','XDG_DATA_HOME':tmp+'/data','XDG_CACHE_HOME':tmp+'/cache','XDG_STATE_HOME':tmp+'/state','GNUPGHOME':tmp+'/gnupg','MAILRS_DATA_DIR':tmp+'/mail','MAILRS_CONFIG':tmp+'/config.toml','DOME_LOGIN_SESSION':'0','DOME_START_XR':'0','DOME_START_IME':'0','DOME_RENDERER':'gles','DOME_PREVIEW_PORT':str(port),'LIBGL_ALWAYS_SOFTWARE':'1','LD_LIBRARY_PATH':str(root/'_toolkit/lib'),'LANG':'C.UTF-8','XMODIFIERS':'@im=none','XDG_CURRENT_DESKTOP':'Dome','PATH':str(bindir)+':'+os.environ['PATH'],'PENGUIN_TEST_OPEN_LOG':tmp+'/opened-urls'}
  env.update(HOME=tmp,DBUS_SYSTEM_BUS_ADDRESS='unix:path=/nonexistent/penguin-notification-system-bus',
             XR_RUNTIME_JSON='/nonexistent/penguin-notification-openxr.json',
             GIO_USE_VFS='local',
             LD_LIBRARY_PATH=os.environ.get('DOME_FLTK_PREFIX','/opt/fltk-vr/toolkit')+'/lib')
  for k in ['WAYLAND_DISPLAY','WAYLAND_SOCKET','FLTK_BACKEND','QT_QPA_PLATFORM','GTK_MODULES']:env.pop(k,None)
  subprocess.run(['dbus-update-activation-environment','HOME','XDG_RUNTIME_DIR','XDG_CONFIG_HOME',
                  'XDG_DATA_HOME','XDG_CACHE_HOME','XDG_STATE_HOME','DBUS_SYSTEM_BUS_ADDRESS',
                  'GNUPGHOME','XR_RUNTIME_JSON','GIO_USE_VFS'],env=env,check=True)
  dome=start([Path(os.environ.get('DOME_TEST_BIN',str(root.parent/'dome-compositor/target/release/dome-compositor'))),'--backend','nested'],'penguin-notification-dome.log',env)
  def scene():
   value=json.load(urllib.request.urlopen(f'http://127.0.0.1:{port}/api/state',timeout=2))['scene'];(art/'penguin-notification-scene.json').write_text(json.dumps(value));return value
  def action(op,**fields):
   request=urllib.request.Request(f'http://127.0.0.1:{port}/api/action',data=json.dumps(dict(op=op,**fields)).encode(),headers={'Content-Type':'application/json'})
   with urllib.request.urlopen(request,timeout=3) as response:assert response.status==200
  wait(lambda:scene()['source']=='wayland')
  info=wait(lambda:subprocess.run(['gdbus','call','--session','--dest','org.freedesktop.Notifications','--object-path','/org/freedesktop/Notifications','--method','org.freedesktop.Notifications.GetServerInformation'],capture_output=True,text=True).stdout)
  check("'DomeD', 'DomeD'" in info,'the isolated session notification service is canonical DomeD')
  monitor=subprocess.Popen([sys.executable,__file__,'--monitor',str(art/'penguin-notification-bus.jsonl')],stdout=subprocess.PIPE,text=True);processes.append(monitor);assert monitor.stdout.readline().strip()=='ready'
  # Fixture proxy preserves the real worker and providers. Only mail-arrival
  # injection is synthetic; reminder scheduling below reads the real store.
  fixture=Path(tmp)/'inject.json';proxy=Path(tmp)/'engine-proxy';proxy.write_text('''#!'''+sys.executable+'''
import json,os,subprocess,sys,threading,time
from pathlib import Path
p=subprocess.Popen([os.environ['PENGUIN_TEST_REAL_ENGINE'],'--demo'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True,bufsize=1)
lock=threading.Lock()
def output(line):
 with lock:sys.stdout.write(line);sys.stdout.flush()
def read():
 for line in p.stdout:
  sys.stderr.write(line);sys.stderr.flush();output(line)
threading.Thread(target=read,daemon=True).start()
def inject():
 path=Path(os.environ['PENGUIN_TEST_INJECT'])
 while p.poll() is None:
  if path.exists():
   line=path.read_text();path.unlink();output(line+'\\n')
  time.sleep(.05)
threading.Thread(target=inject,daemon=True).start()
for line in sys.stdin:p.stdin.write(line);p.stdin.flush()
p.stdin.close();sys.exit(p.wait(timeout=5))
''');proxy.chmod(0o700)
  socketname=wait(lambda:re.search(r'Dome Wayland socket: (\S+)',(art/'penguin-notification-dome.log').read_text()))[1]
  appenv={**env,'WAYLAND_DISPLAY':socketname,'FLTK_BACKEND':'wayland','PENGUIN_MAIL_ENGINE':str(proxy),'PENGUIN_TEST_REAL_ENGINE':str(binary.parent/'penguin-mail-engine'),'PENGUIN_TEST_INJECT':str(fixture)}
  app=start([binary],'penguin-notification-app.log',appenv)
  wait(lambda:any('Penguin Mail' in w['title'] and '(12)' in w['title'] for w in scene()['windows']))
  def dbpath():
   ids=[str(app.pid)]
   for pid in ids:
    try:ids.extend((Path('/proc')/pid/'task'/pid/'children').read_text().split())
    except OSError:continue
    for fd in (Path('/proc')/pid/'fd').iterdir():
     try:target=Path(os.readlink(fd))
     except OSError:continue
     if target.name=='mailrs.db':return target
  database=wait(dbpath)
  def sql(query,args=()):
   with sqlite3.connect(database,timeout=3) as c:return c.execute(query,args).fetchall()
  wait(lambda:sql('select id from events'))
  xd=display.Display(ds);host=wait(lambda:next((w for w in xd.screen().root.query_tree().children if str(w.get_wm_name() or '').startswith('Dome')),None));host.set_input_focus(X.RevertToParent,X.CurrentTime);xd.sync()
  def click(px,py):
   g=host.get_geometry();xtest.fake_input(xd,X.MotionNotify,x=g.x+int(px),y=g.y+int(py));xd.sync();time.sleep(.04);xtest.fake_input(xd,X.ButtonPress,1);xtest.fake_input(xd,X.ButtonRelease,1);xd.sync();time.sleep(.18)
  def shot(name):
   g=host.get_geometry();raw=host.get_image(0,0,g.width,g.height,X.ZPixmap,0xffffffff);im=Image.frombytes('RGB',(g.width,g.height),raw.data,'raw','BGRX');im.save(art/(name+'.png'))
  def menu_action(label):
   layout=subprocess.check_output(['gdbus','call','--session','--dest','io.github.c9dev.PenguinMail.Fltk','--object-path','/org/de_xr/FLTK/Menu1','--method','com.canonical.dbusmenu.GetLayout','--','0','-1','[]'],text=True);item=re.search(r"\((\d+), \{'label': <'"+re.escape(label)+"'>",layout);assert item,label
   subprocess.run(['gdbus','call','--session','--dest','io.github.c9dev.PenguinMail.Fltk','--object-path','/org/de_xr/FLTK/Menu1','--method','com.canonical.dbusmenu.Event','--',item[1],'clicked','<0>','0'],check=True,stdout=subprocess.DEVNULL,timeout=5)
  def buslog():return [json.loads(l) for l in (art/'penguin-notification-bus.jsonl').read_text().splitlines() if l]
  def notices(title=None):return [m for m in buslog() if m['member']=='Notify' and (title is None or m['body'][3]==title)]
  def noticeid(m):
   return wait(lambda:next((r['body'][0] for r in buslog() if r['type']=='METHOD_RETURN' and r['reply_serial']==m['serial'] and r['destination']==m['sender']),None))
  g=host.get_geometry();toastx=g.width-360-12
  def history_open():
   # Four shell-owned icons plus Penguin Mail's tray item; the clock uses its
   # default minimum width of 176 pixels in this private configuration.
   click(g.width-290,18);click(g.width/2,118)
  # Actual reminder timer must continue while the app is hidden in the tray.
  menu_action('Close Window');wait(lambda:not any('Penguin Mail' in w['title'] for w in scene()['windows']))
  now=int(time.time()*1000);event=sql('select account_id,calendar,id from events order by account_id limit 1')[0];eventdate=datetime.datetime.fromtimestamp((now+120000)/1000).date().isoformat()
  sql('update events set title=?, starts_at=?, ends_at=?, reminders=?,conference=? where account_id=? and calendar=? and id=?',('Calendar notification test',now+120000,now+3600000,json.dumps([{'minutes':5,'method':'Notification'}]),'https://example.invalid/calendar-call',*event))
  calendar=wait(lambda:notices('Calendar notification test'));first=calendar[0];nid=noticeid(first)
  check(first['body'][0]=='Penguin Mail' and first['body'][6]['desktop-entry']=='io.github.c9dev.PenguinMail','calendar reminders reach DomeD with the correct app and desktop identity')
  check(first['body'][5][::2]==['default','join-event','snooze-reminder'],'calendar notification offers open, join and five-minute snooze')
  check(bool(sql('select * from event_reminders_shown')),'a reminder is recorded only after DomeD returns its notification ID')
  shot('penguin-notification-calendar-toast')
  subprocess.run([sys.executable,__file__,'--spoof',str(nid)],check=True);time.sleep(.3)
  check(not any('Penguin Mail' in w['title'] for w in scene()['windows']),'a forged ActionInvoked signal from another bus peer cannot open mail')
  click(toastx+100,80);wait(lambda:any(eventdate in w['title'] for w in scene()['windows']))
  check(True,'clicking the actual DomeD reminder opens Penguin Mail at the event date from the tray')
  shot('penguin-notification-calendar-open')
  main=next(w for w in scene()['windows'] if eventdate in w['title'])['id']
  action('add_workspace',name='Notification destination')
  action('switch_workspace',workspace=1)
  history_open()
  wait(lambda:scene()['focused']==main and next(w for w in scene()['windows'] if w['id']==main)['workspace']==1)
  check(eventdate in next(w for w in scene()['windows'] if w['id']==main)['title'],
        'calendar history reopens the stored event and moves the existing main window to the clicked desktop')
  shot('penguin-notification-calendar-history')
  # Force a stored snooze due, exercising real recurrence/log evaluation again.
  sql('update event_reminders_shown set snoozed_until=?',(int(time.time()*1000)-1,));wait(lambda:len(notices('Calendar notification test'))>=2);click(toastx+240,164)
  until=wait(lambda:next((r[0] for r in sql('select snoozed_until from event_reminders_shown') if r[0] and r[0]>int(time.time()*1000)),None))
  check(240000<=until-int(time.time()*1000)<=301000,'DomeD Snooze writes a persistent five-minute snooze through the mail engine')
  sql('update event_reminders_shown set snoozed_until=?',(int(time.time()*1000)-1,));wait(lambda:len(notices('Calendar notification test'))>=3);click(toastx+80,164)
  check(wait(lambda:Path(tmp+'/opened-urls').read_text().strip() if Path(tmp+'/opened-urls').exists() else None)=='https://example.invalid/calendar-call','DomeD Join opens only the conference URL read from the stored event')
  time.sleep(16);check(len(notices('Calendar notification test'))==3,'accepted reminders do not repeat on later scheduler ticks')
  # Mail click targets are exercised independently of the calendar page and
  # current account; the real worker loads the requested stored conversation.
  menu_action('Close Window');fixture.write_text(json.dumps({'event':'notification','kind':'mail','key':'fixture-mail','title':'1 new message','body':'','target':{'account':event[0],'thread':'demo-0-1','message':'demo-0-1'},'account':event[0],'actions':[['default','Open']]}))
  mail=wait(lambda:notices('1 new message'))[0];check(mail['body'][6]['category']=='email.arrived' and mail['body'][4]=='','mail notification uses DomeD while preserving count-only message privacy')
  mailid=noticeid(mail)
  wait(lambda:any(m['member']=='NotificationClosed' and m['body']==[mailid,1] for m in buslog()))
  action('switch_workspace',workspace=0)
  history_open();wait(lambda:any('Penguin Mail' in w['title'] and 'Calendar' not in w['title'] for w in scene()['windows']));time.sleep(.8)
  mailwindow=next(w for w in scene()['windows'] if 'Penguin Mail' in w['title'] and 'Calendar' not in w['title'])
  check(mailwindow['workspace']==0 and scene()['focused']==mailwindow['id'],
        'an expired mail popup stays actionable from history and restores the inbox on the clicked desktop')
  shot('penguin-notification-mail-open');check(app.poll() is None,'clicking a DomeD mail notification restores the inbox and loads its conversation')
  # A second window with the same app ID must not absorb main-window activation.
  menu_action('New Message')
  composer=wait(lambda:next((w for w in scene()['windows'] if w['title']=='Compose Message'),None))
  action('switch_workspace',workspace=1)
  fixture.write_text(json.dumps({'event':'notification','kind':'mail','key':'fixture-shown-mail','title':'Visible mail window test','body':'','target':{'account':event[0],'thread':'demo-0-1','message':'demo-0-1'},'account':event[0],'actions':[['default','Open']]}))
  wait(lambda:notices('Visible mail window test'));click(toastx+100,80)
  wait(lambda:scene()['focused']==mailwindow['id'] and next(w for w in scene()['windows'] if w['id']==mailwindow['id'])['workspace']==1)
  check(next(w for w in scene()['windows'] if w['id']==composer['id'])['workspace']==0,
        'a popup brings the already-visible mail window to the clicked desktop without moving its composer')
  shot('penguin-notification-existing-window')
  menu_action('Quit');time.sleep(.3)
  # Discard only the untouched synthetic composer in this private test session.
  for symbol in [XK.XK_Right,XK.XK_Return]:
   code=xd.keysym_to_keycode(symbol)
   xtest.fake_input(xd,X.KeyPress,code);xtest.fake_input(xd,X.KeyRelease,code);xd.sync();time.sleep(.1)
  wait(lambda:app.poll() is not None);check(app.returncode==0,'notification subscriptions and worker shut down cleanly')
finally:
 if xd:xd.close()
 for p in reversed(processes):
  if p.poll() is None:
   p.terminate()
   try:p.wait(timeout=5)
   except subprocess.TimeoutExpired:p.kill();p.wait()
 for log in logs:log.close()
(art/'penguin-notification-ui-tests.json').write_text(json.dumps(checks,indent=2)+'\n')
