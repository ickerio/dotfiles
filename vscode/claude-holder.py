#!/usr/bin/env python3
# Claude Code extension's process wrapper (claudeCode.claudeProcessWrapper, set on the PC):
# keeps the panel's `claude` running when the window quits or reloads, and lets the next
# window reattach to it.
#
#   extension host -> front (this) -> holder (setsid, relays stdio) -> claude
#
# The holder owns claude's pipes, so the extension host dying only cuts it off from the
# extension. Idle when that happens: claude is stopped like normal (--resume starts it again).
# Mid-turn, or with background tasks (agents, shells) still going: it all runs on, requests meant
# for the extension (permission prompts, IDE tools) wait, and when a window reopens the session
# its --resume front attaches to the running claude instead. The panel loads the transcript, then
# gets the live stream and waiting prompts.
import io
import json
import os
import re
import signal
import socket
import stat
import subprocess
import sys
import threading
import time

RUN_DIR = (os.path.join(os.environ["XDG_RUNTIME_DIR"], "claude-holder") if os.environ.get("XDG_RUNTIME_DIR")
           else f"/tmp/claude-holder-{os.getuid()}")
LOG = os.path.expanduser("~/.cache/claude-holder.log")
LOG_MAX = 1 << 20
# Nobody's come back after this long: answer waiting requests so the turn can end
GIVE_UP = 12 * 3600
# Busy and detached, nothing waiting on anyone, and claude's said nothing for this long: the turn's
# over. Some user messages (/clear, for one) never get a result, which would keep claude up forever
SILENT = 30 * 60
USER = re.compile(rb'"type":\s*"user"')
RESULT = re.compile(rb'"type":\s*"result"')
CONTROL = re.compile(rb'"type":\s*"control_')
SYSTEM = re.compile(rb'"type":\s*"system"')
TASK_DONE = ("completed", "failed", "killed")
DENY = "VS Code disconnected while this was running. Stop here and wait for the user to come back."


def log(msg):
    try:
        with open(LOG, "a") as f:
            f.write(f"{time.strftime('%F %T')} [{os.getpid()}] {msg}\n")
    except OSError:
        pass


def remove(*paths):
    for path in paths:
        try:
            os.unlink(path)
        except (OSError, TypeError):
            pass


def check_run_dir():
    # Whatever's in here gets trusted with the session (permission answers included),
    # so it has to be this user's alone
    os.makedirs(RUN_DIR, mode=0o700, exist_ok=True)
    st = os.lstat(RUN_DIR)
    if not stat.S_ISDIR(st.st_mode) or st.st_uid != os.getuid() or st.st_mode & 0o077:
        raise OSError(f"{RUN_DIR} isn't private to this user")


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def resume_id(args):
    for i, arg in enumerate(args):
        if arg.startswith("--resume="):
            return arg.split("=", 1)[1]
        if arg == "--resume" and i + 1 < len(args):
            return args[i + 1]
    return None


def parse(line):
    try:
        return json.loads(line)
    except ValueError:
        return None


def write_all(fd, data):
    while data:
        data = data[os.write(fd, data):]


def hang_up(conn):
    # close() alone leaves the fd open while a makefile() reader still has it
    try:
        conn.shutdown(socket.SHUT_RDWR)
    except OSError:
        pass
    conn.close()


class Holder:
    def __init__(self, argv, life_fd, exit_fd):
        self.lock = threading.Lock()  # claude's stdin
        self.out_lock = threading.Lock()  # the attached extension
        self.gen = 0
        self.write = None  # to the attached extension, None while detached
        self.close = None
        self.busy = False
        self.tasks = set()  # background tasks claude has running
        self.stdin_open = True
        self.detached_at = None
        self.last_output = time.monotonic()
        self.gave_up = False
        self.session = resume_id(argv[1:])
        self.pending = {}  # requests claude made of the extension, not answered yet
        self.init_id = None
        self.init_response = None
        self.exit_fd = exit_fd
        self.entry = os.path.join(RUN_DIR, f"{os.getpid()}.json")
        self.sock_path = os.path.join(RUN_DIR, f"{os.getpid()}.sock")
        # Left behind by a SIGKILLed holder whose pid has come round again
        remove(self.sock_path)
        self.server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.server.bind(self.sock_path)
        self.server.listen()

        # stdin unbuffered so nothing sits in a buffer; the readers buffered, as a raw pipe's
        # readline() is a read() per byte
        self.claude = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE, bufsize=0)
        self.out = io.BufferedReader(self.claude.stdout, 1 << 16)
        self.err = io.BufferedReader(self.claude.stderr)
        log(f"started claude {self.claude.pid}, session {self.session}")
        self.register()
        signal.signal(signal.SIGTERM, lambda *_: self.claude.terminate())
        self.attach_fds(life_fd)
        for target in (self.from_claude, self.claude_stderr, self.accept, self.watchdog):
            threading.Thread(target=target, daemon=True).start()

    def register(self):
        if self.session:
            with open(self.entry, "w") as f:
                json.dump({"pid": os.getpid(), "session": self.session, "sock": self.sock_path}, f)

    def to_claude(self, data):
        with self.lock:
            if not self.stdin_open:
                return
            try:
                write_all(self.claude.stdin.fileno(), data)
            except OSError:
                pass

    def stop_claude(self):
        with self.lock:
            if self.stdin_open:
                self.stdin_open = False
                self.claude.stdin.close()

    def to_extension(self, data):
        with self.out_lock:
            if self.write is None:
                return
            gen = self.gen
            try:
                self.write(data)
                return
            except OSError:
                pass
        self.detach(gen, "write failed")

    # Attaching

    def attached(self, write, close):
        # Caller holds out_lock. A window still attached is one that's reloading: cut it off
        # here rather than through detach(), which would stop an idle claude
        if self.write is not None:
            try:
                self.close()
            except OSError:
                pass
        self.detached_at = None
        self.gave_up = False
        self.gen += 1
        self.write = write
        self.close = close
        return self.gen

    def detach(self, gen, why):
        with self.out_lock:
            if gen != self.gen or self.write is None:
                return
            self.write = None
            try:
                self.close()
            except OSError:
                pass
            self.detached_at = time.monotonic()
        if self.busy or self.tasks:
            log(f"detached ({why}), {'busy' if self.busy else 'idle'} with {len(self.tasks)} tasks, "
                "keeping claude running")
        else:
            log(f"detached ({why}), idle, stopping claude")
            self.stop_claude()

    def attach_fds(self, life_fd):
        # The first extension host, through the stdio the front handed down
        def close():
            # Lets the extension host see this process as closed
            devnull = os.open(os.devnull, os.O_RDWR)
            os.dup2(devnull, 1)
            os.dup2(devnull, 2)
            os.close(devnull)

        with self.out_lock:
            gen = self.attached(lambda data: write_all(1, data), close)
        lines = iter(sys.stdin.buffer.readline, b"")
        threading.Thread(target=self.from_extension, args=(gen, lines, False), daemon=True).start()

        def watch_front():
            # EOF once the front exits, however it was killed
            os.read(life_fd, 1)
            self.detach(gen, "front exited")
        threading.Thread(target=watch_front, daemon=True).start()

    def accept(self):
        # Later extension hosts, through a reattaching front
        while True:
            conn, _ = self.server.accept()
            with self.out_lock:
                # Worth attaching to: a turn or background task that's still running, or a claude
                # whose window is reloading and hasn't let go yet (refusing that would start a
                # second claude on the same session)
                taking_over = self.write is not None
                ok = self.stdin_open and (self.busy or self.tasks or taking_over)
                if ok:
                    try:
                        conn.sendall(b"ok\n")
                    except OSError:
                        conn.close()
                        continue
                    gen = self.attached(conn.sendall, lambda conn=conn: hang_up(conn))
            if not ok:
                try:
                    conn.sendall(b"no\n")
                except OSError:
                    pass
                conn.close()
                continue
            lines = iter(conn.makefile("rb").readline, b"")
            log(f"{'took over' if taking_over else 'reattached'} ({len(self.pending)} requests waiting)")
            threading.Thread(target=self.from_extension, args=(gen, lines, True), daemon=True).start()

    # Relaying

    def from_extension(self, gen, lines, reattached):
        for line in lines:
            if gen != self.gen:
                break
            if CONTROL.search(line, 0, 100):
                msg = parse(line) or {}
                request = msg.get("request") or {}
                if msg.get("type") == "control_response":
                    self.pending.pop((msg.get("response") or {}).get("request_id"), None)
                elif request.get("subtype") == "initialize":
                    if reattached and self.init_response:
                        # claude's already initialized: answer with the first window's handshake,
                        # then hand over whatever's been waiting on the extension
                        response = dict(self.init_response, request_id=msg.get("request_id"))
                        self.to_extension(json.dumps({"type": "control_response", "response": response}).encode() + b"\n")
                        for waiting in list(self.pending.values()):
                            self.to_extension(waiting)
                        continue
                    self.init_id = msg.get("request_id")
            elif USER.search(line, 0, 200):
                self.busy = True
            self.to_claude(line)
        self.detach(gen, "extension closed")

    def answer(self, msg):
        request = msg.get("request", {})
        response = {"subtype": "error", "request_id": msg.get("request_id"), "error": DENY}
        if request.get("subtype") == "can_use_tool":
            response = {"subtype": "success", "request_id": msg.get("request_id"),
                        "response": {"behavior": "deny", "message": DENY}}
        log(f"answered {request.get('subtype')} after giving up")
        self.to_claude(json.dumps({"type": "control_response", "response": response}).encode() + b"\n")

    def from_claude(self):
        for line in iter(self.out.readline, b""):
            self.last_output = time.monotonic()
            msg = None
            if CONTROL.search(line, 0, 100) or RESULT.search(line, 0, 100) or SYSTEM.search(line, 0, 100) \
                    or (not self.session and b'"session_id"' in line):
                msg = parse(line)
            if msg is not None:
                # Not just the first one: /clear carries on in the same claude under a new id,
                # and that's the id the next window will --resume
                session = msg.get("session_id")
                if session and session != self.session:
                    if self.session:
                        log(f"session now {session}")
                    self.session = session
                    self.register()
                kind = msg.get("type")
                if kind == "control_request":
                    if self.gave_up and self.write is None:
                        self.answer(msg)
                        continue
                    self.pending[msg.get("request_id")] = line
                elif kind == "control_cancel_request":
                    self.pending.pop(msg.get("request_id"), None)
                elif kind == "control_response":
                    response = msg.get("response") or {}
                    if self.init_id and response.get("request_id") == self.init_id:
                        self.init_response = response
                elif kind == "system":
                    subtype = msg.get("subtype")
                    if subtype == "task_started":
                        self.tasks.add(msg.get("task_id"))
                    elif subtype == "task_notification":
                        self.tasks.discard(msg.get("task_id"))
                        # claude takes a turn on what the task came back with, ending in a result
                        self.busy = True
                    elif subtype == "task_updated" and (msg.get("patch") or {}).get("status") in TASK_DONE:
                        self.tasks.discard(msg.get("task_id"))
                elif kind == "result":
                    self.busy = False
                    if self.write is None:
                        if self.tasks:
                            log(f"turn finished while detached, waiting on {len(self.tasks)} tasks")
                        else:
                            log("turn finished while detached, stopping claude")
                            self.stop_claude()
            self.to_extension(line)

    def claude_stderr(self):
        # Only the first extension host has a stderr to pass it to. Not logged otherwise: the
        # extension runs claude with --debug-to-stderr, so it's a firehose
        for line in iter(self.err.readline, b""):
            with self.out_lock:
                if self.gen == 1 and self.write is not None:
                    try:
                        write_all(2, line)
                    except OSError:
                        pass

    def watchdog(self):
        while True:
            time.sleep(60)
            with self.out_lock:
                detached_at = self.detached_at if self.write is None else None
            if detached_at is None or not self.stdin_open:
                continue
            now = time.monotonic()
            if not self.gave_up and now - detached_at > GIVE_UP:
                log("nobody came back, answering waiting requests")
                self.gave_up = True
                for line in list(self.pending.values()):
                    self.answer(parse(line) or {})
                self.pending.clear()
            # Background tasks can be quiet for hours (a training run, say): they end it themselves
            if self.busy and not self.tasks and not self.pending and now - self.last_output > SILENT:
                log("busy but silent while detached, stopping claude")
                self.stop_claude()

    def run(self):
        code = self.claude.wait()
        log(f"claude exited {code}")
        remove(self.entry, self.sock_path)
        with self.out_lock:
            if self.write is not None:
                self.close()
        try:
            os.write(self.exit_fd, bytes([code & 0xFF]))
        except OSError:
            pass
        return code


def reattach(session):
    # A holder still running this session's turn from a closed window: relay to it instead
    try:
        names = os.listdir(RUN_DIR)
    except OSError:
        return False
    for name in names:
        if not name.endswith(".json"):
            continue
        try:
            with open(os.path.join(RUN_DIR, name)) as f:
                entry = json.load(f)
            if not alive(entry["pid"]):
                # A holder that was killed before it could clean up
                remove(os.path.join(RUN_DIR, name), entry.get("sock"))
                continue
            if entry.get("session") != session:
                continue
            conn = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            conn.connect(entry["sock"])
            reader = conn.makefile("rb")
            if reader.readline() != b"ok\n":
                conn.close()
                continue
        except (OSError, ValueError, KeyError):
            continue
        log(f"front attaching to holder {entry['pid']} for {session}")

        def upstream():
            try:
                for line in iter(sys.stdin.buffer.readline, b""):
                    conn.sendall(line)
            except OSError:
                pass
            # Extension host's gone: let the holder know straight away
            os._exit(0)
        threading.Thread(target=upstream, daemon=True).start()
        try:
            while data := reader.read1(65536):
                write_all(1, data)
        except OSError:
            pass
        return True
    return False


def main():
    argv = sys.argv[1:]
    if not argv:
        sys.exit("usage: claude-holder.py <claude> [args...]")
    try:
        check_run_dir()
    except OSError as e:
        log(f"{e}, running claude without a holder")
        os.execvp(argv[0], argv)
    session = resume_id(argv[1:])
    if session and reattach(session):
        os._exit(0)

    life_r, life_w = os.pipe()
    exit_r, exit_w = os.pipe()
    if os.fork() == 0:
        # Own session, so nothing aimed at the extension host's processes reaches it
        os.setsid()
        if os.fork():
            os._exit(0)
        os.close(life_w)
        os.close(exit_r)
        signal.signal(signal.SIGPIPE, signal.SIG_IGN)
        try:
            if os.path.getsize(LOG) > LOG_MAX:
                os.replace(LOG, LOG + ".1")
        except OSError:
            pass
        try:
            os._exit(Holder(argv, life_r, exit_w).run())
        except Exception as e:
            log(f"holder failed: {e!r}")
            os._exit(1)

    os.wait()
    os.close(life_r)
    os.close(exit_w)
    # The holder has these now; the front only waits, and exits like claude does
    devnull = os.open(os.devnull, os.O_RDWR)
    for fd in (0, 1, 2):
        os.dup2(devnull, fd)
    code = os.read(exit_r, 1)
    os._exit(code[0] if code else 1)


if __name__ == "__main__":
    main()
