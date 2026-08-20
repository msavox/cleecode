import json, os, pty, select, time
HERE=os.getcwd(); SNAP=os.path.join(HERE,"pyws.json"); FIGS=os.path.join(HERE,"pyfigs")
for f in (SNAP,):
    if os.path.exists(f): os.remove(f)
pid, fd = pty.fork()
if pid==0:
    os.environ.update(TERM="dumb", PYTHONSTARTUP=os.path.join(HERE,"pystartup2.py"),
                      PYTHONPATH=HERE, CLEECODE_PY_WS=SNAP, CLEECODE_PY_FIGS=FIGS)
    os.execvp(os.path.join(HERE,"pyenv","bin","python"), ["python"])
def pump(s):
    end=time.time()+s
    while time.time()<end:
        r,_,_=select.select([fd],[],[],0.02)
        if r:
            try:
                if not os.read(fd,65536): return
            except OSError: return
def send(l): os.write(fd,(l+"\n").encode()); 
def snap():
    with open(SNAP) as f: return json.load(f)
pump(1.5)
CMDS=[("a = 5",None),("import numpy as np",None),("arr = np.arange(100).reshape(10,10)",None),
      ("arr[0,0] = 999",None),("s = 'ciao'",None),("d = {'x':1,'y':2}",None),
      ("nn = np.array([1.0, np.nan, 5.0])",None),("z = np.array([3+4j, 1-1j])",None),
      ("lst = [3,1,4,1,5]",None),("del s",None),
      ("import matplotlib; matplotlib.use('Agg')",None),
      ("import matplotlib.pyplot as plt",None),
      ("fig, ax = plt.subplots(figsize=(8,6), dpi=96); _=ax.plot(np.arange(100))",None)]
prev=0
for cmd,_x in CMDS:
    send(cmd); pump(1.2 if "matplotlib" in cmd or "subplots" in cmd else 0.45)
d=snap()
print(f"seq={d['seq']} pid={d['pid']} lang={d['lang']}")
print(f"{'name':<8} {'class':<18} {'size':<10} {'min':>10} {'max':>10} {'mean':>10}  preview")
for v in d["vars"]:
    sz="x".join(map(str,v["size"]))
    fmt=lambda x: "-" if x is None else f"{x:.4g}"
    print(f"  {v['name']:<6} {v['class']:<18} {sz:<10} {fmt(v['min']):>10} {fmt(v['max']):>10} {fmt(v['mean']):>10}  {v['preview'][:32]}")
print("figures:", json.dumps(d["figures"], indent=1)[:420])
send("exit()"); pump(1.0)
try: os.close(fd)
except OSError: pass
os.waitpid(pid,0)
