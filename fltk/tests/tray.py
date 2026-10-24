#!/usr/bin/env python3
"""Exercise the actual login entry, SNI pixmaps and deferred menu replies."""
import os, pathlib, re, signal, subprocess, sys, tempfile, time
signal.pthread_sigmask(signal.SIG_SETMASK, [])
if os.environ.get('PENGUIN_TRAY_TEST_BUS') != '1':
    os.execvpe('xvfb-run', ['xvfb-run', '-a', 'dbus-run-session', '--', sys.executable, *sys.argv],
              {**os.environ, 'PENGUIN_TRAY_TEST_BUS':'1', 'FLTK_BACKEND':'x11', 'LIBGL_ALWAYS_SOFTWARE':'1'})
with tempfile.TemporaryDirectory(prefix='penguin-tray-') as config:
    process = subprocess.Popen([sys.argv[1], config], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    monitor = None
    try:
        service, path = process.stdout.readline().split()
        def call(interface, member, *args, target='/StatusNotifierItem'):
            return subprocess.check_output(['gdbus','call','--session','--dest',service,
                '--object-path',target,'--method',interface+'.'+member,'--',*args], text=True, timeout=3)
        def prop(name):
            return call('org.freedesktop.DBus.Properties','Get','org.kde.StatusNotifierItem',name)
        normal = prop('IconPixmap')
        assert '24, 24' in normal
        log=pathlib.Path(config)/'signals'
        with log.open('w') as output:
            monitor=subprocess.Popen(['gdbus','monitor','--session','--dest',service,'--object-path','/StatusNotifierItem'],stdout=output,text=True)
        time.sleep(.1)
        process.stdin.write('3\n');process.stdin.flush();time.sleep(.08)
        unread = prop('IconPixmap')
        assert normal != unread and 'NewIcon' in log.read_text()
        assert '-unread' in prop('IconName')
        process.stdin.write('0\n');process.stdin.flush();time.sleep(.08)
        assert prop('IconPixmap') == normal
        assert log.read_text().count('NewIcon') == 2
        call('org.kde.StatusNotifierItem','ContextMenu','0','0')
        assert prop('IconPixmap') == normal
        start=time.monotonic()
        layout=call('com.canonical.dbusmenu','GetLayout','0','-1','[]',target=path)
        elapsed=time.monotonic()-start
        assert elapsed < .15, elapsed
        item=re.search(r"\((\d+), \{'label': <'Slow'>",layout).group(1)
        start=time.monotonic()
        call('com.canonical.dbusmenu','Event',item,'clicked','<0>','0',target=path)
        duration=time.monotonic()-start
        assert duration < .2, duration
        time.sleep(.55)
        call('org.kde.StatusNotifierItem','Activate','0','0');time.sleep(.08)
        assert prop('IconPixmap') == unread
        entry=pathlib.Path(config)/'autostart/io.github.c9dev.PenguinMail.desktop'
        assert 'Hidden=true' in entry.read_text() and '--tray' in entry.read_text()
        print(f'PASS login enable/disable and escaped Unicode Exec, unread/clear icons and signals, context menu, deferred activation; layout {elapsed*1000:.1f} ms, slow action reply {duration*1000:.1f} ms')
    finally:
        if monitor:
            monitor.terminate();monitor.wait(timeout=5)
        if process.poll() is None:
            process.stdin.write('q\n');process.stdin.flush()
            try:process.wait(timeout=5)
            except subprocess.TimeoutExpired:process.kill();process.wait()
        assert process.returncode == 0
