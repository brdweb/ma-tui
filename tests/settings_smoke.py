"""Real terminal setup, failed login, keyring persistence and theme reload.

Uses synthetic credentials and a temporary secret-tool double; never the user's
keyring, config, Music Assistant server, or playback devices.
"""
import codecs
import fcntl
import http.server
import json
import os
import pathlib
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time
import pyte

token_mode = len(sys.argv) > 2 and sys.argv[2] == 'token'
fail_token_once = token_mode
prefix = '/prefix/' + 'long-path/' * 40 + 'music-assistant'
calls = []
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args): pass
    def reply(self, value, status=200):
        data = json.dumps(value).encode()
        self.send_response(status)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)
    def do_GET(self):
        assert self.path == prefix + '/info'
        assert 'Authorization' not in self.headers
        # Offline 2.10.5 /info fixture; upstream API_SCHEMA_VERSION is 65.
        self.reply({'server_version':'2.10.5', 'schema_version': 65})
    def do_POST(self):
        global fail_token_once
        req = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        if self.path == prefix + '/auth/login':
            calls.append('login')
            assert req['provider_id'] == 'builtin'
            if req['credentials']['password'] != 'fixture-password':
                self.reply({'success':False, 'error':'fixture-password'}, 401)
            else:
                self.reply({'success':True, 'token':'fixture-token'})
            return
        assert self.path == prefix + '/api'
        assert self.headers['Authorization'] == 'Bearer fixture-token'
        calls.append(req['command'])
        if req['command'] == 'auth/me' and fail_token_once:
            fail_token_once = False
            self.reply({'error':'do not echo fixture-token'}, 405)
        elif req['command'] == 'auth/me': self.reply({'username':'fixture-user'})
        elif req['command'] == 'players/all': self.reply([{'player_id':'speaker', 'name':'Fixture speaker', 'available':True}])
        else: raise AssertionError(req['command'])

server = http.server.ThreadingHTTPServer(('127.0.0.1',0), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
with tempfile.TemporaryDirectory(prefix='ma-tui-settings-') as tmp:
    tmp = pathlib.Path(tmp)
    config = tmp / 'config.toml'
    config.write_text(f'server="http://127.0.0.1:{server.server_port}{prefix}"\nplayer_id="fixture-id"\nlocal_playback=false\n')
    helper = tmp / 'secret-tool'
    helper.write_text('''#!/usr/bin/python3
import os,sys,pathlib
p=pathlib.Path(os.environ['FIXTURE_KEYRING'])
assert 'fixture-token' not in sys.argv
if sys.argv[1]=='store':
 p.write_text(sys.stdin.read())
else:
 if not p.exists(): sys.exit(1)
 print(p.read_text())
''')
    helper.chmod(0o700)
    theme = tmp / 'state/omarchy/current/theme/colors.toml'
    theme.parent.mkdir(parents=True)
    theme.write_text("background='#010203'\nforeground='#eeeeee'\naccent='#ff0000'\n")
    env = dict(os.environ, TERM='xterm-256color', PATH=f'{tmp}:'+os.environ['PATH'],
               FIXTURE_KEYRING=str(tmp/'keyring'), XDG_STATE_HOME=str(tmp/'state'))
    env.pop('MA_TUI_TOKEN',None)
    env.pop('MATUI_TOKEN',None)
    env.pop('NO_COLOR',None)
    for restart in [False, True]:
        master,slave=pty.openpty()
        fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',30,110,0,0))
        original=termios.tcgetattr(slave)
        screen=pyte.Screen(110,30)
        stream=pyte.Stream(screen)
        decoder=codecs.getincrementaldecoder('utf-8')('replace')
        output=bytearray()
        proc=subprocess.Popen([sys.argv[1],'--config',str(config)] + ([] if restart else ['--setup']),
                              stdin=slave,stdout=slave,stderr=slave,env=env)
        def until(predicate):
            deadline=time.monotonic()+10
            while time.monotonic()<deadline:
                if select.select([master],[],[],0.05)[0]:
                    chunk=os.read(master,65536);output.extend(chunk);stream.feed(decoder.decode(chunk))
                if predicate(): return
                if proc.poll() is not None: break
            raise AssertionError(f'exit={proc.poll()}, palette={screen.buffer[1][0]!r}; fg={screen.buffer[0][2]!r}; screen={screen.display!r}')
        def visible(value): until(lambda:value in '\n'.join(screen.display))
        try:
            if not restart:
                visible('CONNECTION SETTINGS')
                assert b'\x1b[?2004h' in output
                os.write(master,b'\x15')
                os.write(master,b'\x1b[200~' + f'http://127.0.0.1:{server.server_port}{prefix}\r\n'.encode() + b'\x1b[201~')
                if token_mode:
                    os.write(master,b'\t\t\t\x1b[200~fixture-token\x1b[201~\t\t\t\t\r')
                    visible('HTTP 405')
                    assert not (tmp/'keyring').exists()
                    assert any('Profile token:' in line and '•' in line for line in screen.display), 'failed test cleared the token'
                    os.write(master,b'\r')  # Retry the retained token without repasting it.
                else:
                    os.write(master,b'\tfixture-user\twrong-password\t\t\t\t\t\r')
                    visible('Login rejected (HTTP 401)')
                    assert not (tmp/'keyring').exists()
                    assert any('Password:' in line and '•' in line for line in screen.display), 'failed test cleared the password'
                    os.write(master,b'\t\t\t\x15\x1b[200~fixture-password\r\n\x1b[201~\t\t\t\t\t\r')
            visible('Fixture speaker')
            assert b'fixture-password' not in output
            assert b'fixture-token' not in output
            assert 'fixture-token' not in config.read_text()
            assert (config.stat().st_mode & 0o777)==0o600
            assert (tmp/'keyring').read_text()=='fixture-token'
            theme.write_text("background='#fafafa'\nforeground='#111111'\naccent='#0000ff'\n")
            until(lambda:screen.buffer[1][0].bg == 'fafafa')
            os.write(master,b'q')
            assert proc.wait(timeout=5)==0
            assert termios.tcgetattr(slave)==original
        finally:
            if proc.poll() is None: proc.kill();proc.wait()
            os.close(master);os.close(slave)
    assert calls.count('login')==(0 if token_mode else 2), 'restart should reuse saved credentials'
server.shutdown()
print(('Token' if token_mode else 'Password') + ' settings PTY passed: failed/successful login, masked secrets, long URL/password paste, saved login after restart, live palette reload')
