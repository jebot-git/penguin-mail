#!/usr/bin/env python3
"""Refresh an old DomeD font export while the real FLTK application is running."""
import json,os,signal,subprocess,sys,tempfile,time
from pathlib import Path
from Xlib import X,display
from PIL import Image
signal.pthread_sigmask(signal.SIG_SETMASK,[])
if os.environ.get('PENGUIN_FONT_TEST_BUS')!='1':os.execvpe('dbus-run-session',['dbus-run-session','--',sys.executable,*sys.argv],{**os.environ,'PENGUIN_FONT_TEST_BUS':'1'})
repo=Path(__file__).resolve().parents[2];suite=repo.parent;art=repo/'test-artifacts';art.mkdir(exist_ok=True)
dome=Path(os.environ['DOME_TEST_BIN']);checks=[];processes=[]
def wait(fn,seconds=25):
 until=time.monotonic()+seconds
 while time.monotonic()<until:
  value=fn()
  if value:return value
  time.sleep(.07)
 raise AssertionError('Timed out')
def check(ok,label):
 assert ok,label
 checks.append(label);print('PASS',label,flush=True)
with tempfile.TemporaryDirectory(prefix='penguin-font-refresh-') as tmp:
 tmp=Path(tmp);export=tmp/'dome-appearance-123';export.mkdir();config=export/'fonts.conf'
 config.write_text('<?xml version="1.0"?><fontconfig><include>/etc/fonts/fonts.conf</include></fontconfig>')
 # A minimal older session configuration omits DomeD's bundled assets.
 rd,wr=os.pipe();xvfb=subprocess.Popen(['Xvfb','-displayfd',str(wr),'-screen','0','1500x1000x24','-nolisten','tcp'],pass_fds=(wr,),stderr=subprocess.DEVNULL);os.close(wr)
 with os.fdopen(rd) as stream:ds=':'+stream.readline().strip()
 env={**os.environ,'DISPLAY':ds,'FLTK_BACKEND':'x11','LD_LIBRARY_PATH':str(suite/'_toolkit/lib'),'FONTCONFIG_FILE':str(config),'XDG_CURRENT_DESKTOP':'Dome','XDG_RUNTIME_DIR':str(tmp),'XDG_CONFIG_HOME':str(tmp/'config'),'XDG_DATA_HOME':str(tmp/'data'),'XDG_CACHE_HOME':str(tmp/'cache'),'GNUPGHOME':str(tmp/'gnupg'),'MAILRS_DATA_DIR':str(tmp/'mail'),'MAILRS_CONFIG':str(tmp/'config.toml'),'LANG':'en_US.UTF-8','LC_ALL':'en_US.UTF-8','DE_XR_APPEARANCE_TRACE':'1','DOME_FLTK_BITMAP_TRACE':'1','XMODIFIERS':'@im=none'}
 appearance=tmp/'config/dome/appearance.json';appearance.parent.mkdir(parents=True)
 def select(name,size):appearance.write_text(json.dumps({'font':name,'font_size':size,'light':False,'accent':'#6babff'}))
 select('DungGeunMo / Regular',16)
 logfile=art/'penguin-font-refresh.log';log=logfile.open('w');xd=display.Display(ds)
 def capture(w,name):
  w.configure(stack_mode=X.Above);xd.sync();time.sleep(.25);g=w.get_geometry();raw=w.get_image(0,0,g.width,g.height,X.ZPixmap,0xffffffff);im=Image.frombytes('RGB',(g.width,g.height),raw.data,'raw','BGRX');im.save(art/(name+'.png'));return im
 try:
  app=subprocess.Popen([str(repo/'build-fltk/penguin-mail'),'--demo'],env=env,stdout=log,stderr=log);processes.append(app)
  window=wait(lambda:next((w for w in xd.screen().root.query_tree().children if 'Penguin Mail' in str(w.get_wm_name() or '')),None));time.sleep(.7)
  before=capture(window,'penguin-font-dunggeunmo-before')
  prior=subprocess.check_output(['fc-match','-f','%{family}', 'DungGeunMo:pixelsize=16'],env=env,text=True)
  check(prior!='DungGeunMo','old export reproduces a missing DungGeunMo face')
  subprocess.run([str(dome),'--refresh-session-fonts'],env=env,check=True,stdout=log,stderr=log)
  check((tmp/'dome-fonts/fonts.conf').is_file(),'session refresh creates a stable export and repairs the legacy path inherited by the running app')
  # Running twice must not include fonts.conf into itself or churn its timestamp.
  stamp=config.stat().st_mtime_ns
  subprocess.run([str(dome),'--export-fonts',str(export)],env=env,check=True,stdout=log,stderr=log)
  check(config.stat().st_mtime_ns==stamp,'re-export is idempotent and preserves the font configuration timestamp')
  matched=subprocess.check_output(['fc-match','-f','%{family}|%{pixelsize}|%{scalable}','DungGeunMo:pixelsize=16'],env=env,text=True)
  check(matched=='DungGeunMo|16|False','DomeD exports the actual DungGeunMo 16px bitmap strike')
  wait(lambda:'FLTK bitmap: DungGeunMo / Regular 16 -> native' in logfile.read_text())
  after=capture(window,'penguin-font-dunggeunmo-after')
  check(before.crop((0,0,1000,140)).tobytes()!=after.crop((0,0,1000,140)).tobytes(),'a running application picks up the refreshed export without changing the selected font or restarting')
  lines=subprocess.check_output(['fc-list',':scalable=false','-f','%{family[0]}|%{style[0]}|%{pixelsize}|%{file}\n'],env=env,text=True).splitlines()
  fonts=[]
  for line in lines:
   family,style,pixels,path=line.split('|',3)
   if str(export/'fonts') not in path:continue
   for size in pixels.split(','):
    size=round(float(size))
    if 12<=size<=24:fonts.append((family+' / '+style,size))
  fonts=sorted(set(fonts));check(len(fonts)>=15,'Fontconfig sees the full bundled collection, including the newer families')
  for name,size in fonts:
   select(name,size);wait(lambda:f'FLTK bitmap: {name} {size} -> native' in logfile.read_text())
  check(True,f'all {len(fonts)} bundled face and size combinations use native bitmap rendering during live switches')
  select('DungGeunMo / Regular',16);time.sleep(.5);capture(window,'penguin-font-dunggeunmo-live-return')
  check(app.poll() is None,'switching back to DungGeunMo keeps the application running')
 finally:
  for p in reversed(processes):
   if p.poll() is None:
    p.terminate();p.wait(timeout=5)
  xd.close();xvfb.terminate();xvfb.wait(timeout=5);log.close()
(art/'penguin-font-refresh-tests.json').write_text(json.dumps(checks,indent=2)+'\n')
