//  Motion.swift
//  Robots on the live plane are never parked.
//
//  **Wandering is a live picture of structural congruence.** Position on a
//  plane carries no meaning, so top-level robots drift; they may pass one
//  another, and two matching fingerprints may come close, without anything
//  happening. A person who has watched the plane for a minute has learnt that
//  adjacency is not a relationship, which is the one thing a static diagram of
//  P | Q can never teach.
//
//  **Approach means a committed communication.** The evaluator has already
//  decided it; the approach is the scene's first rendering of that decision.
//  Robots never approach speculatively. That is why the scene animates the
//  commit log rather than the redex candidates: a pair that heads for a
//  meeting point is a pair that has already met.
//
//  **Cables must never be seen knotted**, since a tangle would suggest a
//  relationship between processes that does not exist.

import Foundation
import simd

/// One wandering robot's state.
struct Wanderer {
    /// The executive's identifier for the thing that is moving.
    let id: String
    var position: SIMD3<Float>
    var heading: SIMD2<Float>
    /// Where its continuation is dragging behind it, if it has one.
    var continuationLag: SIMD3<Float>
    /// Inert robots do not wander; nor does one the programmer is holding.
    var still: Bool = false
    /// Set while a committed pair is converging.
    var approachingTo: SIMD3<Float>?

    static func heading(seed: Int) -> SIMD2<Float> {
        // A fingerprint is as good a seed as any, and it makes a robot's
        // wander reproducible across a reconnect.
        let a = Float(seed % 628) / 100.0
        return SIMD2(cos(a), sin(a))
    }
}

/// The plane's motion planner.
///
/// Keeps every robot inside the plane's band so depth keeps its meaning,
/// separates neighbours, and refuses any step that would cross two cables —
/// unless the crossing is one of the three that must never be delayed.
@MainActor
final class MotionPlanner {

    /// How close two cables may come before the planner steers away.
    let clearance: Float = 0.12
    /// Half-width of the band the live plane occupies.
    let bandHalfWidth: Float = 1.6
    let bandHalfHeight: Float = 0.45

    private(set) var robots: [String: Wanderer] = [:]
    /// Cables that parted this frame, so the renderer can draw the gap and
    /// snap them back. A parted cable is a rendering event only: the
    /// continuation stays bound to its prefix throughout.
    private(set) var partedCables: Set<String> = []

    var wanderSpeed: Float = 0.10
    var approachSpeed: Float = 0.40
    /// `false` is *avoid, then break*; `true` is *break freely*, where the
    /// planner ignores cables and every crossing is resolved by parting.
    var breakFreely = false

    func sync(ids: [String], positions: [String: SIMD3<Float>], inert: Set<String>) {
        // Add what is new, keep what is still here, drop what has gone. A
        // robot that survives a round keeps its position and its heading, so
        // the plane does not jump when the scene is re-applied.
        for id in ids where robots[id] == nil {
            robots[id] = Wanderer(id: id,
                                  position: positions[id] ?? .zero,
                                  heading: Wanderer.heading(seed: id.hashValue),
                                  continuationLag: positions[id] ?? .zero,
                                  still: inert.contains(id))
        }
        let live = Set(ids)
        robots = robots.filter { live.contains($0.key) }
        for id in ids { robots[id]?.still = inert.contains(id) }
    }

    /// Hold a robot still: one the programmer is grabbing or inspecting does
    /// not drift. If it is half of a released commit, its partner waits at the
    /// meeting point.
    func hold(_ id: String, _ held: Bool) {
        robots[id]?.still = held
    }

    /// Send a committed pair to a shared meeting point, clear of other
    /// robots, with the waiter on the left and the sender on the right.
    func converge(waiter: String, sender: String) -> SIMD3<Float>? {
        guard let a = robots[waiter]?.position, let b = robots[sender]?.position
        else { return nil }
        var mid = (a + b) / 2
        mid = clearOfOthers(mid, excluding: [waiter, sender])
        let gap = Metric.bodyWidth + Metric.armReach
        robots[waiter]?.approachingTo = mid - SIMD3(gap, 0, 0)
        robots[sender]?.approachingTo = mid + SIMD3(gap, 0, 0)
        return mid
    }

    func releaseApproach(_ ids: [String]) {
        for id in ids { robots[id]?.approachingTo = nil }
    }

    /// One frame of motion.
    func advance(dt: Float) {
        partedCables.removeAll()
        let ids = Array(robots.keys)

        for id in ids {
            guard var w = robots[id] else { continue }

            // A committed pair converges, and nothing stops it. This is one of
            // the crossings that must never be delayed.
            if let target = w.approachingTo {
                if w.still { robots[id] = w; continue }   // its partner waits
                let d = target - w.position
                let dist = length(d)
                if dist > 0.01 {
                    w.position += normalize(d) * min(approachSpeed * dt, dist)
                }
                w.continuationLag = damp(w.continuationLag, toward: w.position, dt: dt)
                robots[id] = w
                continue
            }

            guard !w.still, wanderSpeed > 0 else {
                robots[id] = w
                continue
            }

            // A smoothed random heading, with separation from neighbours.
            w.heading = normalize(w.heading + jitter(id: id) * dt * 2.2
                                            + separation(for: id) * dt * 3.0)
            var step = SIMD3(w.heading.x, 0, w.heading.y) * wanderSpeed * dt
            var next = w.position + step

            // Confined to the plane's band, so depth keeps its meaning.
            if abs(next.x) > bandHalfWidth { w.heading.x = -w.heading.x; step.x = -step.x }
            if abs(next.z - w.position.z) > bandHalfHeight { step.z = 0 }
            next = w.position + step

            // Avoidance: the planner treats every cable as an obstacle and
            // rejects a step that would bring two within the clearance. In
            // effect wandering robots keep to lanes and do not pass behind one
            // another.
            if !breakFreely, crossesACable(id: id, from: w.position, to: next) {
                // Pick a different heading rather than stopping; a robot that
                // froze would read as a robot that had committed.
                w.heading = SIMD2(-w.heading.y, w.heading.x)
                robots[id] = w
                continue
            }
            if breakFreely, crossesACable(id: id, from: w.position, to: next) {
                partedCables.insert(id)
            }

            w.position = next
            w.continuationLag = damp(w.continuationLag, toward: w.position, dt: dt)
            robots[id] = w
        }
    }

    // MARK: Helpers

    /// The continuation follows its waiting robot with a damped lag rather
    /// than rigidly, so a wandering robot visibly drags its future behind it.
    /// The lag is also what makes cables able to cross at all.
    private func damp(_ current: SIMD3<Float>, toward target: SIMD3<Float>,
                      dt: Float) -> SIMD3<Float> {
        current + (target - current) * min(1, dt * 2.5)
    }

    private func jitter(id: String) -> SIMD2<Float> {
        let t = Float(Date().timeIntervalSince1970)
        let phase = Float(abs(id.hashValue % 1000)) / 1000 * 6.283
        return SIMD2(sin(t * 0.7 + phase), cos(t * 0.53 + phase * 1.7)) * 0.4
    }

    private func separation(for id: String) -> SIMD2<Float> {
        guard let me = robots[id]?.position else { return .zero }
        var push = SIMD2<Float>.zero
        for (other, w) in robots where other != id {
            let d = me - w.position
            let dist = length(d)
            if dist > 0.0001, dist < 0.45 {
                push += SIMD2(d.x, d.z) / dist * (0.45 - dist)
            }
        }
        return push
    }

    private func clearOfOthers(_ p: SIMD3<Float>, excluding: Set<String>) -> SIMD3<Float> {
        var out = p
        for (id, w) in robots where !excluding.contains(id) {
            let d = out - w.position
            if length(d) < 0.4, length(d) > 0.0001 {
                out += normalize(d) * 0.4
            }
        }
        out.x = min(max(out.x, -bandHalfWidth), bandHalfWidth)
        return out
    }

    /// A cable runs from a waiting robot back to its continuation's hub. A
    /// candidate step is rejected if the robot's own cable, or its
    /// continuation's lagging path, would come within the clearance of
    /// another's.
    private func crossesACable(id: String, from: SIMD3<Float>, to: SIMD3<Float>) -> Bool {
        guard let mine = robots[id] else { return false }
        let myCable = (to, mine.continuationLag)
        for (other, w) in robots where other != id {
            let theirs = (w.position, w.continuationLag)
            if segmentsWithin(myCable, theirs, clearance) { return true }
        }
        return false
    }

    private func segmentsWithin(_ a: (SIMD3<Float>, SIMD3<Float>),
                                _ b: (SIMD3<Float>, SIMD3<Float>),
                                _ d: Float) -> Bool {
        // Sampled rather than solved: a cable is a chain of segments with a
        // damped lag, not a straight line, so an exact segment-segment
        // distance would be answering a question the geometry does not ask.
        let n = 4
        for i in 0...n {
            let t = Float(i) / Float(n)
            let pa = mix(a.0, a.1, t: t)
            for j in 0...n {
                let s = Float(j) / Float(n)
                let pb = mix(b.0, b.1, t: s)
                if distance(pa, pb) < d { return true }
            }
        }
        return false
    }

    private func mix(_ a: SIMD3<Float>, _ b: SIMD3<Float>, t: Float) -> SIMD3<Float> {
        a + (b - a) * t
    }
}
