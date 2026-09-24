"""Real TUI + HTTP against a local MA-shaped fixture, never a live MA server."""
import fcntl
import codecs
import http.server
import json
import os
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

calls = []
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass
    def do_POST(self):
        assert self.path == "/api"
        assert self.headers["Authorization"] == "Bearer local-fixture"
        req = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        calls.append(req)
        cmd = req["command"]
        if cmd == "players/all":
            body = [{"player_id":"member","name":"Fixture speaker","available":True,"volume_level":30,"playback_state":"paused"}]
        elif cmd == "player_queues/get_active_queue":
            body = {"queue_id":"leader","items":1,"elapsed_time":12,"current_item":{"name":"Fixture song","duration":200}}
        elif cmd == "player_queues/items":
            body = [{"queue_item_id":"item1","name":"Fixture song","duration":200}]
        elif cmd == "music/search":
            body = {"tracks":[{"name":"Search fixture","uri":"library://track/1","artists":[{"name":"Fixture artist"}]}]}
        elif cmd == "music/in_progress_items":
            assert req["args"] == {"limit":100}
            body = [{"name":"Fixture episode","item_id":"ep1","provider":"abs","media_type":"podcast_episode",
                     "uri":"library://podcast_episode/ep1","resume_position_ms":724000}]
        elif cmd == "music/recently_added_tracks":
            assert req["args"] == {"limit":100}
            body = [{"name":"Fixture new track","uri":"library://track/new","media_type":"track"}]
        elif cmd == "music/mark_played":
            assert req["args"] == {"media_item":{"item_id":"ep1","provider":"abs","name":"Fixture episode",
                                                 "media_type":"podcast_episode"},"fully_played":True}
            body = None
        elif cmd == "music/favorites/add_item":
            assert req["args"] == {"item":"library://playlist/playlist1"}
            body = None
        elif cmd == "music/playlists/library_items":
            if req["args"] == {"limit":500,"offset":0,"order_by":"sort_name"}:
                body = [{"name":"Editable fixture","item_id":"pl9","provider":"library","media_type":"playlist",
                         "uri":"library://playlist/pl9","is_editable":True},
                        {"name":"Locked fixture","item_id":"pl8","provider":"spotify","media_type":"playlist",
                         "uri":"spotify://playlist/pl8","is_editable":False}]
            else:
                default_args = {"limit":100,"offset":0,"order_by":"sort_name","favorite":None}
                playlist_calls = [call for call in calls if call["command"] == cmd]
                if len(playlist_calls) == 1:
                    assert req["args"] == default_args
                else:
                    assert req["args"] in (
                        default_args,
                        {"limit":100,"offset":0,"order_by":"timestamp_added_desc","favorite":None},
                    )
                body = [{"name":"Fixture playlist","item_id":"playlist1","provider":"library","media_type":"playlist","uri":"library://playlist/playlist1"}]
        elif cmd == "music/playlists/playlist_tracks":
            assert req["args"] == {"item_id":"playlist1","provider_instance_id_or_domain":"library"}
            body = [{"name":"Browse fixture track","uri":"library://track/browsed","media_type":"track"}]
        elif cmd == "music/playlists/add_playlist_tracks":
            assert req["args"] == {"db_playlist_id":"pl9","uris":["library://track/browsed"]}
            body = {}
        elif cmd == "players/cmd/stop":
            assert req["args"] == {"player_id":"member"}
            body = None
        else:
            assert cmd in ("player_queues/play_pause", "player_queues/play_media", "player_queues/delete_item", "player_queues/save_as_playlist"), cmd
            assert req["args"]["queue_id"] == "leader"
            if cmd == "player_queues/save_as_playlist":
                assert req["args"]["name"] == "Smoke list"
            body = None
        data = json.dumps(body).encode()
        self.send_response(200)
        self.send_header("Content-Length",str(len(data)))
        self.end_headers()
        self.wfile.write(data)

server = http.server.ThreadingHTTPServer(("127.0.0.1",0),Handler)
threading.Thread(target=server.serve_forever,daemon=True).start()
master,slave=pty.openpty()
fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack("HHHH",30,110,0,0))
original=termios.tcgetattr(slave)
screen=pyte.Screen(110,30)
stream=pyte.Stream(screen)
decoder=codecs.getincrementaldecoder("utf-8")("replace")

def until(predicate):
    deadline=time.monotonic()+8
    while time.monotonic()<deadline:
        if select.select([master],[],[],0.05)[0]:
            stream.feed(decoder.decode(os.read(master,65536)))
        if predicate(): return
        if proc.poll() is not None: break
    raise AssertionError(f"fixture integration failed; exit={proc.poll()}, screen={screen.display!r}")
def visible(text):
    until(lambda:text in "\n".join(screen.display))

with tempfile.TemporaryDirectory(prefix="ma-tui-smoke-") as tmp:
    path=os.path.join(tmp,"config.toml")
    with open(path,"w") as f:
        f.write(f'server = "http://127.0.0.1:{server.server_port}"\nplayer_id = "test-local"\nlocal_playback = true\n')
    proc=subprocess.Popen([sys.argv[1],"--config",path,"--remote-only"],stdin=slave,stdout=slave,stderr=slave,
        env=dict(os.environ,TERM="xterm-256color",MA_TUI_TOKEN="local-fixture"))
    try:
        visible("Fixture speaker")
        visible("Music library")
        # Continue listening leads the home listing, and shows a resume point.
        os.write(master,b"b\r")
        visible("Fixture episode")
        visible("resume 12:04")
        # Marking progress needs no speaker; the menu opens on P.
        os.write(master,b"P")
        visible("Mark as played")
        os.write(master,b"\r")
        until(lambda:any(c["command"]=="music/mark_played" for c in calls))
        os.write(master,b"\x7f")  # Back to the music home listing.
        # Past the four shelves to Playlists, sort it, and keep browsing it.
        os.write(master,b"\x1b[B\x1b[B\x1b[B\x1b[B\r")
        visible("Fixture playlist")
        os.write(master,b"o")
        until(lambda:any(c["command"]=="music/playlists/library_items" and c["args"].get("order_by")=="timestamp_added_desc" for c in calls))
        visible("sort: recently added")
        visible("Fixture playlist")
        os.write(master,b"f")
        until(lambda:any(c["command"]=="music/favorites/add_item" for c in calls))
        assert not any(c["command"].startswith("player_queues/") for c in calls), "browsing must work before speaker selection"
        os.write(master,b"\x7f\x1b[Z")  # Back to music home, Shift-Tab to Players.
        os.write(master,b"\r")
        visible("Fixture song")
        os.write(master,b"\r")
        visible("Fixture playlist")
        os.write(master,b"\r")
        visible("Browse fixture track")
        for index, option in enumerate(("replace","next","add")):
            os.write(master,b"\r")
            visible("Play now (replace queue)")
            visible("Fixture speaker")
            os.write(master,b"j"*index+b"\r")
            until(lambda:any(c["command"]=="player_queues/play_media" and c["args"].get("media")=="library://track/browsed" and c["args"]["option"]==option for c in calls))
        os.write(master,b"\r")
        visible("Start radio")
        os.write(master,b"jjj\r")
        until(lambda:any(c["command"]=="player_queues/play_media" and c["args"] == {"queue_id":"leader","media":"radio_playlist://playlist/library://track/browsed","option":"replace"} for c in calls))
        os.write(master,b"\r")
        visible("Add to playlist…")
        os.write(master,b"/playlist")
        visible("Add to playlist…")
        os.write(master,b"\r")
        until(lambda:any(c["command"]=="music/playlists/library_items" and c["args"] == {"limit":500,"offset":0,"order_by":"sort_name"} for c in calls))
        until(lambda:"Editable fixture" in "\n".join(screen.display) and "Locked fixture" not in "\n".join(screen.display))
        os.write(master,b"\r")
        until(lambda:any(c["command"]=="music/playlists/add_playlist_tracks" and c["args"] == {"db_playlist_id":"pl9","uris":["library://track/browsed"]} for c in calls))
        os.write(master,b"\x7fP")
        visible("Play now (replace queue)")
        os.write(master,b"jj\r")
        until(lambda:any(c["command"]=="player_queues/play_media" and c["args"].get("media")=="library://playlist/playlist1" and c["args"]["option"]=="add" for c in calls))
        os.write(master,b" ")
        until(lambda:any(c["command"]=="player_queues/play_pause" for c in calls))
        os.write(master,b"/fixture\r")
        visible("Search fixture")
        os.write(master,b"a")
        until(lambda:any(c["command"]=="player_queues/play_media" and c["args"]["option"]=="add" for c in calls))
        os.write(master,b"\r")
        visible("Play now (replace queue)")
        os.write(master,b"\r")
        until(lambda:any(c["command"]=="player_queues/play_media" and c["args"].get("media")=="library://track/1" and c["args"]["option"]=="replace" for c in calls))
        os.write(master,b"?")
        visible("CONTROLS")
        os.write(master,b"jj\r")
        until(lambda:any(c["command"]=="players/cmd/stop" for c in calls))
        os.write(master,b"?")
        visible("CONTROLS")
        os.write(master,b"/Save queue")
        visible("Save queue as playlist…")
        os.write(master,b"\r")
        visible("SAVE QUEUE AS PLAYLIST…")
        os.write(master,b"Smoke list\r")
        until(lambda:any(c["command"]=="player_queues/save_as_playlist" and c["args"] == {"queue_id":"leader","name":"Smoke list"} for c in calls))
        os.write(master,b"\x1bOS")  # F4 focuses the queue; Esc now cancels or steps back.
        visible("QUEUE")
        os.write(master,b"\x1b[3~")
        until(lambda:any(c["command"]=="player_queues/delete_item" and c["args"]["item_id_or_index"]=="item1" for c in calls))
        os.write(master,b"q")
        assert proc.wait(timeout=5)==0
        assert termios.tcgetattr(slave)==original
        print("Connected fixture smoke passed: browse before player selection, playlist favorites and tracks, all queue choices, whole collection, group routing, search, controls, terminal restoration")
    finally:
        if proc.poll() is None: proc.kill();proc.wait()
        server.shutdown()
        os.close(master);os.close(slave)
