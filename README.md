# Rhobots on Vision Pro, against a rho executive on a Mac

Two pieces, to be dropped into what already exists.

1. **`f1r3x-serve`** — a new crate for the `Campf1r3` workspace. The
   communication harness and the session lifecycle, and nothing else.
2. **`RhobotsAVP`** — the visionOS client: the scene of Rhobots §3–§8 and the
   control panel of §9.

The executive itself is untouched. `f1r3x`, `b3ll0ws`, `campf1r3-core`,
`k1ndl1ng-*` are used exactly as they are.

---

## 1. `f1r3x-serve`

Copy `crates/f1r3x-serve/` into the `Campf1r3` workspace and add it to the
members list in the root `Cargo.toml`:

```toml
members = [
  # … the eight that are there …
  "crates/f1r3x-serve",
]
```

Then:

```
cargo build --release --offline
cargo test --offline
./target/release/f1r3x-serve --cores 8
```

It has no external dependencies, like everything else in the workspace, and
carries `#![forbid(unsafe_code)]`.

```
f1r3x-serve [--addr HOST:PORT] [--cores N] [--ttl SECONDS]
            [--no-bonjour] [--verbose] [--deploy FILE]
```

On start it prints the addresses a headset could reach it on, and advertises
`_f1r3x._tcp` through `dns-sd` so the address need not be read aloud. If
`dns-sd` is missing that is not fatal — the client has a host field.

### What it contains, and why each part is there

| file | what it is |
| --- | --- |
| `ws.rs` | WebSocket: SHA-1, base64, the frame codec. So the Swift side uses `URLSessionWebSocketTask` with nothing linked. |
| `session.rs` | One executive per named session, outliving the connection. |
| `driver.rs` | Admission control: the §9.2 pacing contract. |
| `proto.rs` | The harness verbs. Everything else goes to `f1r3x_ffi::dispatch` verbatim. |
| `tree.rs` | A term written out in the rhobot vocabulary. |
| `scene.rs` | The executive's three populations, each carrying its tree. |

**Why a session outlives its socket.** A Vision Pro is not a well-behaved
client: it is taken off, it sleeps, the app is backgrounded, and Wi-Fi roams
between access points. Every one of those drops the socket. If that destroyed
the executive, a person would lose a running world by putting the headset down
for a minute. Sessions are named by the client — the id lives in the app's
defaults, not in a cookie this server issued — so a resume works across a
process the system has restarted. One connection per session: a second attach
evicts the first and says so, because two headsets acknowledging into one
executive would each see pacing neither had asked for.

**Why the pacing is an acknowledgment.** In process, §9.2's lockstep is a
semaphore the scheduler holds until the scene signals. Over a socket it has to
be a message, and that is the only real change the network forces on the
design. The rest falls out: lockstep is free-running with a buffer of one, and
`Control::admission_open` is the single place either is decided.

**Why `tree.rs` exists.** `f1r3x`'s scene gives each drawable thing a *label* —
source-shaped text. That is right for a list and useless for a hand. Rhobots
draws a robot per constructor, so the client needs the shape of the term.
`tree.rs` writes the same `Norm` the executive is already holding out in the
§4 vocabulary. Every node carries `fp`, the content hash the store keys by, so
matching glyphs are the witness that a communication is possible and the client
never decides name equivalence itself. Elision happens server-side, against a
depth the client derives from σ and s_min, and an elided node *keeps its
fingerprint* — which is what §3 asks for, and also what stops one deep term
from sending megabytes to a headset that would draw three capsules of it.

### The wire

`ws://<host>:<port>/s/<session-id>`. One request per text frame; an optional
`#<n> ` prefix carries a correlation number, which comes back on the reply.

Requests are the `f1r3x-ffi` vocabulary — `deploy`, `tick`, `run`, `scene`,
`trace`, `log`, `cores`, `mode`, `status`, `show` — plus the harness verbs:

| verb | what it does |
| --- | --- |
| `play` / `pause` / `step` | the panel's run controls; `step` admits exactly one round in either mode |
| `pacing lockstep\|free` | §9.2 |
| `buffer <n>` | the playback buffer bound, in rounds |
| `ack <round>` | the scene has finished animating everything through this round |
| `treedepth <n>` | quotation levels the client can still draw |
| `resend` | push the scene as it stands, without advancing |
| `reset` | throw the world away, keep the session |
| `ping` / `bye` | |

Frames out are tagged objects: `ready`, `reply`, `round`, `scene`, `stats`,
`quiescent`, `error`. A `round` carries the commit log for that round *and* the
scene that results from it, so the client animates the log and then has the
world to apply — no diffing, and no state the executive already holds being
reconstructed on the device.

### One departure from the spec, stated plainly

§10 has worker threads racing on the device's physical cores, so that races are
settled by the hardware. `b3ll0ws` is a single-threaded deterministic machine,
and `cores` here is the **width of an admitted round**, not a count of racing
threads.

The slider still teaches what §9.3 says it teaches — up to `cores`
communications in progress at once, the number of converging pairs a picture of
concurrency, channel contention flattening the throughput readout — but the
plateau is a property of the term rather than of a lock. When the store grows a
threaded profile, `driver::run` is the one function that has to change. It is
documented there.

The upside is the property `Campf1r3`'s own README claims and this preserves:
the final scene is identical at width 1, at width 64, and free-running. A person
can slow the world to a single step and watch exactly the run they just saw at
speed. There is a test for it over the wire.

### Tests

33 new, 160 in the workspace, all passing. The socket tests are the ones worth
reading: lockstep withholding without an ack, the buffer bound, reconnect
resuming the same world, eviction, a bad term coming back as diagnostics rather
than a dropped socket, and the width-invariance property above.

```
cargo test --offline -p f1r3x-serve
```

---

## 2. `RhobotsAVP`

```
cd RhobotsAVP
xcodegen generate      # or make a visionOS app target by hand over Sources/
open RhobotsAVP.xcodeproj
```

visionOS 2.0, no package dependencies. The hand-tracking and world-sensing
usage descriptions are in `Resources/Info.plist` from the start, per the
bring-up carry-over — a missing one is a crash on first gesture, found on
device and never in the simulator. `NSAllowsLocalNetworking` and the Bonjour
service are there too, since `ws://` to a Mac on the same network is otherwise
refused.

| file | what it is |
| --- | --- |
| `Wire/Wire.swift` | the frames, the scene tree, and the client |
| `Model/Model.swift` | the panel's state, and the world with its playback buffer |
| `Scene/Layout.swift` | fingerprints, and L(P, z, s) |
| `Scene/Rhobots.swift` | the vocabulary as RealityKit entities |
| `Scene/Motion.swift` | wandering, approach, cable clearance |
| `Scene/Director.swift` | the four-phase communication animation |
| `UI/ImmersiveView.swift` | the space, and the frame loop |
| `UI/ControlPanel.swift` | the five groups of §9.1 |

### The two things it is most important to have got right

**The acknowledgment is sent when the animation finishes, and not before.**
Acknowledging on arrival would turn lockstep into free-running and
free-running into an unbounded queue, and the scene would silently fall behind
the evaluator while looking perfectly healthy. The scene therefore *pulls*
rounds — `World.beginNextRound()` — and `World.finish(_:)` is the only place an
ack is sent.

**Identity comes from the executive.** Entities are keyed by the card
identifiers `f1r3x` already issues, which are built from content hashes. A
robot that survives a round keeps its entity, and so its position, its wander
and its animation state. Nothing in the client invents a handle, which is what
lets a robot be followed across a frame — and across a reconnect.

### What is sketched rather than settled

The construction gestures. §8 of the spec calls them a sketch to be settled on
device, and they are: there is a grab that holds a robot still, which is the
foundation the rest sits on, and no palette, no pinch-to-quote, no
drop-behind-to-set-a-continuation. Those want a headset and a person, not a
guess.

### What has not been exercised

I could not compile the Swift — there is no toolchain here and no device. The
Rust is built, tested and smoke-tested against a real socket with an
independent client; the Swift is written against the wire format those tests
pin down, and it will want a first pass in Xcode. The places I would look
first are the RealityKit call sites in `Rhobots.swift` and the
`targetedToAnyEntity` drag in `ImmersiveView.swift`.

---

## Trying it without a headset

The harness is a plain WebSocket, so the whole protocol can be driven from a
terminal. `examples/replication.rho` is the spec's first acceptance test —
replication from reflection, §8 — and if it is legible in the scene, the
metaphor is sound.

```
./target/release/f1r3x-serve --cores 8 --deploy examples/replication.rho
```

then attach to `ws://127.0.0.1:9099/s/default`, send `play`, and acknowledge
each round.
