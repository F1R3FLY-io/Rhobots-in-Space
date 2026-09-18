# Rhobots in Space

Programming the pure rho calculus in immersive space on Apple Vision Pro.

This repository is the application: Swift, RealityKit and SwiftUI, and nothing
else. The executive it talks to is [`CampF1R3`], which knows nothing about
headsets — the same executive runs in a browser tab and behind a C ABI.

The two meet at one number, `WIRE_VERSION`, which arrives on the first frame.

## Running it

Start an executive on a Mac on the same network:

```
# in the CampF1R3 checkout
cargo build --release
./target/release/f1r3x-serve --cores 8
```

It prints the addresses it can be reached on and advertises `_f1r3x._tcp`, so
nobody has to read an IP address aloud.

Then:

```
xcodegen generate      # or make a visionOS app target by hand over Sources/
open RhobotsAVP.xcodeproj
```

visionOS 2.0, no package dependencies. Enter the host in the panel, deploy a
term, and enter the space.

`Resources/Info.plist` carries the hand-tracking and world-sensing usage
descriptions from the start, per the carry-over from the F1R3Skein bring-up — a
missing one is a crash on first gesture, found on device and never in the
simulator. `NSAllowsLocalNetworking` and the Bonjour service are there too,
since `ws://` to a Mac on the same network is otherwise refused.

## What is here

| file | what it is |
| --- | --- |
| `Sources/Wire/Wire.swift` | the frames, the scene tree, and the client |
| `Sources/Model/Model.swift` | the panel's state, and the world with its playback buffer |
| `Sources/Scene/Layout.swift` | fingerprints, and L(P, z, s) |
| `Sources/Scene/Rhobots.swift` | the vocabulary as RealityKit entities |
| `Sources/Scene/Motion.swift` | wandering, approach, cable clearance |
| `Sources/Scene/Director.swift` | the four-phase communication animation |
| `Sources/UI/ImmersiveView.swift` | the space, and the frame loop |
| `Sources/UI/ControlPanel.swift` | the five groups of the spec's §9.1 |
| `tools/` | a WebSocket client in standard-library Python, for driving an executive without a headset |

## The two things it is most important to have got right

**The acknowledgment is sent when the animation finishes, and not before.**
Acknowledging on arrival would turn lockstep into free-running and
free-running into an unbounded queue, and the scene would silently fall behind
the evaluator while looking perfectly healthy. The scene therefore *pulls*
rounds — `World.beginNextRound()` — and `World.finish(_:)` is the only place an
ack is sent.

**Identity comes from the executive.** Entities are keyed by the card
identifiers `f1r3x` issues, which are built from content hashes. A robot that
survives a round keeps its entity, and so its position, its wander and its
animation state. Nothing here invents a handle, which is what lets a robot be
followed across a frame — and across a reconnect.

## The metaphor

Two axes that never interfere. **Depth carries guardedness**: the plane nearest
the programmer holds exactly the top-level parallel composition, and a
receive's continuation stands one plane behind it. **Scale carries
quotation**: a process inside a quote is drawn at σ times its container's
scale, always inside a capsule, so quotation is marked by more than size.
Distant robots do subtend smaller visual angles, but that is perspective; the
capsule is what tells the two apart.

Robots on the live plane are never parked. **Wandering is a live picture of
structural congruence** — position carries no meaning, so two matching
fingerprints may come close without anything happening. **Approach means a
committed communication**: the evaluator has already decided it, and the
approach is the scene's first rendering of that decision. Robots never approach
speculatively, which is why the scene animates the commit log rather than the
redex candidates.

`tools/replication.rho` is replication from reflection, the spec's first
acceptance test. If it is legible in the scene, the metaphor is sound.

## What is sketched rather than settled

The construction gestures. §8 of the spec calls them a sketch to be settled on
device, and they are: there is a grab that holds a robot still, which is the
foundation the rest sits on, and no palette, no pinch-to-quote, no
drop-behind-to-set-a-continuation. Those want a headset and a person.

## Without a headset

`tools/` drives an executive from a terminal, which is useful for checking that
a term behaves before putting it in front of anyone.

```
python3 tools/drive.py
```

[`CampF1R3`]: https://github.com/F1R3FLY-io/Campf1r3
