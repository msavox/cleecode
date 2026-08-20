#!/usr/bin/env python3
"""Un comando alla volta, PTY sempre drenata: ogni comando deve produrre uno snapshot fresco."""
import json, os, pty, select, time
HERE = os.getcwd(); SNAP = os.path.join(HERE, "ws2.json")
if os.path.exists(SNAP): os.remove(SNAP)
pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "dumb"
    os.execvp("octave-cli", ["octave-cli", "--no-init-file", "--interactive"])
def pump(s):
    end = time.time()+s
    while time.time() < end:
        r,_,_ = select.select([fd],[],[],0.02)
        if r:
            try:
                if not os.read(fd, 65536): return
            except OSError: return
def send(l): os.write(fd, (l+"\n").encode())
def snap():
    try:
        with open(SNAP) as f: return json.load(f)
    except Exception: return None

pump(1.5); send(f"addpath('{HERE}');"); pump(0.5)
send(f"cleecode_ws('{SNAP}');"); pump(0.8)
base = snap()["seq"]

CMDS = [
  ("a = 1:10;",            lambda d: d["a"]["max"] == 10),
  ("a(1) = 999;",          lambda d: d["a"]["max"] == 999),
  ("s = 'ciao';",          lambda d: d["s"]["preview"] == "ciao"),
  ("z = [3+4i 1-1i];",     lambda d: d["z"]["attr"] == "c" and abs(d["z"]["max"]-5) < 1e-9),
  ("nn = [1 NaN 5];",      lambda d: d["nn"]["nans"] == 1 and d["nn"]["mean"] == 3),
  ("clear s;",             lambda d: "s" not in d),
  ("st.alpha = 1;",        lambda d: d["st"]["class"] == "struct"),
  ("global G; G = 7;",     lambda d: "g" in d["G"]["attr"]),
  ("a = a + 1;",           lambda d: d["a"]["max"] == 1000),
  ("a = a + 1;",           lambda d: d["a"]["max"] == 1001),   # comando ripetuto identico
  ("pause(1.5); k = 3;",   lambda d: d["k"]["max"] == 3),
]
ok = fail = 0
prev = base
for cmd, check in CMDS:
    send(cmd)
    pump(2.6 if "pause" in cmd else 0.9)
    d = snap()
    seqd = d["seq"]
    byname = {v["name"]: v for v in d["vars"]}
    fresh = seqd > prev
    try: passed = check(byname)
    except Exception as e: passed = False
    prev = seqd
    if fresh and passed:
        ok += 1; print(f"  ok    seq={seqd:<3} {cmd}")
    else:
        fail += 1; print(f"  FAIL  seq={seqd:<3} {cmd}   (fresh={fresh} check={passed})")
send("exit"); pump(1.0)
os.close(fd); os.waitpid(pid,0)
print(f"\n{ok} ok, {fail} fail")
