#!/usr/bin/env python3
"""Build the native frontend and its upstream Rust engine without installing."""
from pathlib import Path
import argparse, os, re, shlex, shutil, subprocess
root=Path(__file__).resolve().parents[1]
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--prefix',type=Path,default=root.parent/'_toolkit')
p.add_argument('--build-dir',type=Path,default=root/'build-fltk')
p.add_argument('--jobs',type=int,default=3)
p.add_argument('--misfin',action='store_true',help='Build optional Misfin(C) messaging')
a=p.parse_args();env=dict(os.environ)
# Match upstream's source-build configuration without executing a shell file.
secrets=root/'packaging/secrets.env'
if secrets.is_file():
    allowed={'PENGUIN_MAIL_GOOGLE_CLIENT_ID','PENGUIN_MAIL_GOOGLE_CLIENT_SECRET','PENGUIN_MAIL_MICROSOFT_CLIENT_ID'}
    for line in secrets.read_text().splitlines():
        values=shlex.split(line,comments=True)
        if not values:continue
        if values[0]=='export':values=values[1:]
        if len(values)!=1 or '=' not in values[0]:raise SystemExit('Invalid OAuth assignment in packaging/secrets.env')
        key,value=values[0].split('=',1)
        if key in allowed:env.setdefault(key,value)

def version(command):
    try:
        result=subprocess.check_output([str(command),'--version'],text=True)
        match=re.search(r'(\d+)\.(\d+)',result)
        return tuple(map(int,match.groups())) if match else (0,0)
    except (OSError,subprocess.CalledProcessError):return (0,0)
if version('rustc') < (1,98):
    candidates=[root.parent.parent/'dome-compositor/.local-deps/toolchain/rustup/toolchains',root.parent/'dome-compositor/.local-deps/toolchain/rustup/toolchains']
    available=[bin for base in candidates if base.exists() for bin in base.glob('*/bin') if version(bin/'rustc') >= (1,98)]
    if not available:raise SystemExit('Rust 1.98 or newer is required. Put its rustc and cargo on PATH.')
    env['PATH']=str(available[0])+os.pathsep+env.get('PATH','')
env['CARGO_BUILD_JOBS']=str(max(1,a.jobs))
subprocess.run(['cmake','-S',str(root/'fltk'),'-B',str(a.build_dir),'-G','Ninja','-DCMAKE_BUILD_TYPE=Release',f'-DCMAKE_PREFIX_PATH={a.prefix.resolve()}',f'-DPENGUIN_ENABLE_MISFIN={"ON" if a.misfin else "OFF"}'],env=env,check=True)
subprocess.run(['cmake','--build',str(a.build_dir),'--parallel',str(max(1,a.jobs))],env=env,check=True)
