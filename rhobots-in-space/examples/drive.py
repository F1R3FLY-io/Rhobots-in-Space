import sys, time; sys.path.insert(0,'/tmp')
from wsclient import WS

def latest(w, tag, secs=1.5):
    """Drain everything available and return the most recent frame for `tag`.
    The real client dispatches on the tag and keeps only the newest scene;
    a queue would hand back the empty world it drew on connect."""
    end = time.time() + secs
    w.s.settimeout(0.3)
    while time.time() < end:
        try: w.buf.append(w.recv())
        except Exception: break
    w.s.settimeout(5)
    hits = [f for f in w.buf if f.get("t") == tag]
    w.buf = [f for f in w.buf if f.get("t") != tag]
    return hits[-1] if hits else None

w = WS("127.0.0.1", 9099, "/s/smoke3")
r = w.until("ready")
print("ready      :", {k: r[k] for k in ("wire","session","resumed","coresMax")})
w.send("#1 deploy\n" + open('examples/replication.rho').read())
rep = w.until("reply")
print("deploy     : ok=%s seq=%s" % (rep["body"]["ok"], rep["seq"]))
w.send("#2 pacing lockstep"); w.until("reply")
w.send("#3 play"); w.until("reply")

for expect in (1,2,3,4,5,6):
    rd = w.until("round"); sc = rd["scene"]
    print("round %d    : steps=%-9s rhobots=%d tuples=%d listeners=%d inFlight=%d comms=%d"
          % (rd["round"], ",".join(s["kind"] for s in rd["steps"]),
             len(sc["rhobots"]), len(sc["tuples"]), len(sc["listeners"]),
             rd["inFlight"], sc["comms"]))
    assert rd["round"] == expect, rd["round"]
    if expect == 4:
        print("lockstep   : another round without ack?", "YES (bug)" if w.sees("round", 0.8) else "no -- withheld")
    w.send("ack %d" % rd["round"])

st = latest(w, "stats")
print("stats      :", {k: st[k] for k in ("pacing","cores","inFlight","buffer","admissionOpen","comms","commitsPerSec")})

w.send("#9 resend"); w.until("reply")
sc = latest(w, "scene")["scene"]
print("populations: rhobots=%d tuples=%d listeners=%d comms=%d"
      % (len(sc["rhobots"]), len(sc["tuples"]), len(sc["listeners"]), sc["comms"]))
for c in sc["listeners"][:1]:
    t = c["tree"]
    print("listener   :", c["label"][:58])
    print("  tree     : f=%s arrow=%s chan.g=%s body.f=%s" %
          (t["f"], t["binds"][0]["arrow"], t["binds"][0]["chan"]["g"], t["body"]["f"]))
    print("  fp match : card.chanFp == tree bind chan fp ->", c["chanFp"] == t["binds"][0]["chan"]["fp"])
for c in sc["tuples"][:1]:
    print("tuple      : %-44s tree.f=%s" % (c["label"][:44], c["tree"]["f"]))
for c in sc["rhobots"][:1]:
    print("rhobot     : %-44s tree.f=%s" % (c["label"][:44], c["tree"]["f"]))

# free-running back-pressure, live
w.send("pacing free"); w.until("reply")
w.send("buffer 3");    w.until("reply")
n = 0
while w.sees("round", 1.0): n += 1
print("free-run   : admitted %d rounds with no ack (bound 3) -> %s" % (n, "bounded" if n <= 3 else "UNBOUNDED (bug)"))
print("OK")
