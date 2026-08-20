#!/usr/bin/env python3
"""End-to-end: octave interattivo in PTY, hook installato, si guarda il file JSON."""
import json, os, pty, select, sys, time

HERE = os.getcwd()
SNAP = os.path.join(HERE, "ws.json")
for f in (SNAP,):
    if os.path.exists(f):
        os.remove(f)

pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "dumb"
    os.execvp("octave-cli", ["octave-cli", "--no-init-file", "--interactive"])

transcript = []
def pump(seconds):
    end = time.time() + seconds
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try:
                d = os.read(fd, 65536)
            except OSError:
                return
            if not d:
                return
            transcript.append(d.decode("utf8", "replace"))

def send(l): os.write(fd, (l + "\n").encode())

def snap():
    if not os.path.exists(SNAP):
        return None
    with open(SNAP) as f:
        return json.load(f)

def show(label):
    d = snap()
    if d is None:
        print(f"[{label}] nessuno snapshot"); return
    print(f"\n[{label}] seq={d['seq']} pid={d['pid']} vars={len(d['vars'])}")
    for v in d["vars"]:
        sz = "x".join(str(x) for x in v["size"])
        st = ""
        if v["min"] is not None:
            st = f"min={v['min']:.4g} max={v['max']:.4g} mean={v['mean']:.4g}"
            if v["nans"]:
                st += f" nan={v['nans']}"
        print(f"   {v['name']:<10} {v['class']:<12} {sz:<10} {v['attr']:<3} {st:<44} {v['preview']}")

pump(1.5)
send(f"addpath('{HERE}');")
send(f"cleecode_ws('{SNAP}');")
pump(1.0); show("appena installato")

send("a = 1:10; b = randn(3,3)*100; s = 'ciao mondo'; c = {1,2,3};")
send("st.alpha = 1; st.beta = 2; z = [3+4i 1-1i]; L = logical([1 0 1]);")
send("nn = [1 NaN 5]; i8 = int8([-5 100]); f = @(x) x.^2; e = [];")
pump(1.5); show("dopo le assegnazioni")

send("a(1) = 999;")          # in-place: metadata identici
pump(1.0); show("dopo a(1)=999 (modifica in place)")

send("clear b c;")
pump(1.0); show("dopo clear b c")

send("big = rand(1500);")    # 2.25e6 elementi: oltre STAT_LIMIT
pump(2.0); show("dopo big = rand(1500)")

send("cleecode_ws('off');")
send("exit"); pump(1.5)
os.close(fd); os.waitpid(pid, 0)

print("\n=== quello che l'utente ha visto nel terminale ===")
txt = "".join(transcript)
print(txt[txt.index("cleecode_ws"):][:900] if "cleecode_ws" in txt else txt[-900:])
