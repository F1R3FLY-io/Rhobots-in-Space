//  Director.swift
//  What the scene does with a round.
//
//  A committed comm plays in four phases:
//
//    1. **Approach.** The waiter and the sender leave their wander and
//       converge on a meeting point, arriving face to face, waiter on the
//       left. This lasts as long as the distance requires at the approach
//       speed — it is the one phase whose duration the panel does not set.
//    2. **Match.** The two glyphs pulse in their shared fingerprint colour.
//    3. **Hand-off.** The capsule crosses the gap between the facing hands,
//       from the sender's hand to the waiter's binder platform, and both
//       robots fade: a send has no continuation, and a receipt is consumed.
//    4. **Advance and substitute.** Everything behind the waiter moves
//       forward one plane, so its first plane becomes part of the live plane,
//       and every platform labelled y fills.
//
//  Substitution is where the two axes finally meet. At a **process** position
//  — a `*y` platform — the payload unshrinks to the platform's own scale, and
//  the platform and its unshrink icon disappear, leaving the robot. That is
//  relative: a `*y` inside a capsule restores its payload to the capsule's
//  scale, not to the plane's. At a **name** position the shrunken payload
//  takes the slot and stays shrunk, and its fingerprint appears, so the
//  robot's channel becomes comparable with others.
//
//  The round is acknowledged when all of that has finished, and not before.

import Foundation
import RealityKit
import SwiftUI
import simd

@MainActor
final class SceneDirector {

    let root = Entity()
    private let world: World
    private var panel: Panel { world.panel }

    private let motion = MotionPlanner()
    private var cardEntities: [String: Entity] = [:]
    private var lastScene = SceneFrame.empty
    private var lastFrameTime = Date()

    /// Robots consumed by a round being animated. They keep wandering until
    /// their entry is released, and are never shown approaching anyone other
    /// than their committed partner.
    private var busy: Set<String> = []

    init(world: World) {
        self.world = world
        root.name = "rhobots.root"
    }

    // MARK: The frame loop

    /// Called every frame by the immersive view.
    func update() {
        let now = Date()
        let dt = Float(min(now.timeIntervalSince(lastFrameTime), 1.0 / 20.0))
        lastFrameTime = now

        motion.wanderSpeed = Float(panel.wanderSpeed)
        motion.approachSpeed = Float(panel.approachSpeed)
        motion.breakFreely = panel.breakFreely
        motion.advance(dt: dt)

        // Carry the planner's positions onto the entities.
        for (id, w) in motion.robots {
            guard let e = cardEntities[id] else { continue }
            e.position.x = w.position.x
            e.position.z = w.position.z
            if let cont = e.findEntity(named: "continuation") {
                // The continuation drags behind on its cable.
                cont.position.x = w.continuationLag.x - w.position.x
                cont.position.z = w.continuationLag.z - w.position.z - Float(panel.planeSpacing)
            }
        }

        // Start the next round if the scene is free. Nothing pushes rounds at
        // the scene; it pulls them, which is what keeps the acknowledgment
        // honest and the buffer meaningful.
        if let round = world.beginNextRound() {
            Task { await self.play(round) }
        }
    }

    // MARK: Applying a scene

    /// Rebuild what has changed. Entities are keyed by the executive's own
    /// identifiers, so a robot that survives a round keeps its entity, and
    /// therefore its position, its wander and its animation state.
    func apply(_ scene: SceneFrame, unshrinkNew: Bool) {
        let layout = Layout(panel: panel)
        let placed = layout.placeLivePlane(scene.livePlane)
        let factory = RhobotFactory(panel: panel)

        var wanted: Set<String> = []
        var fresh: [String] = []

        for (card, placements) in placed {
            wanted.insert(card.id)
            if cardEntities[card.id] != nil { continue }
            let e = factory.build(card: card, placements: placements)
            cardEntities[card.id] = e
            root.addChild(e)
            fresh.append(card.id)
        }

        // What has gone, goes. Inert robots linger for the delay the panel
        // sets — P | 0 ≡ P, made visible — unless it is set to never.
        for (id, e) in cardEntities where !wanted.contains(id) {
            cardEntities.removeValue(forKey: id)
            fade(e, over: 0.25) { e.removeFromParent() }
        }

        motion.sync(ids: Array(wanted),
                    positions: positions(of: placed),
                    inert: inertIDs(scene))

        if unshrinkNew {
            // Arrivals unshrink into place: the visual signature of
            // substitution at a process position.
            for id in fresh {
                guard let e = cardEntities[id] else { continue }
                let target = e.scale
                e.scale = target * Float(panel.shrink)
                var t = e.transform
                t.scale = target
                e.move(to: t, relativeTo: e.parent,
                       duration: panel.commDuration * 0.35,
                       timingFunction: .easeOut)
            }
        }
        lastScene = scene
    }

    private func positions(of placed: [(Card, [Placement])]) -> [String: SIMD3<Float>] {
        var out: [String: SIMD3<Float>] = [:]
        for (card, ps) in placed { out[card.id] = ps.first?.node.position ?? .zero }
        return out
    }

    /// An inert robot never acts and never wanders.
    private func inertIDs(_ scene: SceneFrame) -> Set<String> {
        var out: Set<String> = []
        for c in scene.livePlane {
            if case .nilProc = c.tree { out.insert(c.id) }
        }
        return out
    }

    // MARK: Playing a round

    private func play(_ round: Round) async {
        let comms = round.steps.filter { $0.isComm }

        // Up to `cores` pairs approach at once, so the number of converging
        // pairs is itself a picture of concurrency.
        await withTaskGroup(of: Void.self) { group in
            for step in comms {
                guard let pair = partners(for: step) else { continue }
                group.addTask { @MainActor in await self.playComm(pair) }
            }
        }

        // The losing partners of a race show their unmatched glyphs briefly,
        // so a race is visible after the fact.
        showLosers(of: comms)

        // Advance and substitute: the round's own scene is the result.
        apply(round.scene, unshrinkNew: true)
        try? await Task.sleep(nanoseconds: UInt64(panel.commDuration * 0.35 * 1e9))

        busy.removeAll()
        world.finish(round)
    }

    private struct Pair {
        let waiter: Card
        let sender: Card
        let fingerprint: Fingerprint
    }

    /// Find the two robots a commit consumed.
    ///
    /// The commit log names them by content hash, which is exactly what the
    /// card identifiers are built from, so nothing has to be guessed: a
    /// listener's id begins with its continuation's hash, and a tuple sits on
    /// the channel the commit names.
    private func partners(for step: Step) -> Pair? {
        guard let contHash = step.cont, let chanHash = step.chan else { return nil }
        let waiter = lastScene.listeners.first { $0.id.hasPrefix(contHash) }
        let sender = lastScene.tuples.first { $0.chanFp == chanHash }
        guard let waiter, let sender else { return nil }
        busy.insert(waiter.id)
        busy.insert(sender.id)
        return Pair(waiter: waiter, sender: sender, fingerprint: Fingerprint(chanHash))
    }

    private func playComm(_ pair: Pair) async {
        let d = panel.commDuration

        // 1. Approach. Released, not predicted: the evaluator has already
        //    decided this, and the approach is the scene's first rendering of
        //    that decision.
        _ = motion.converge(waiter: pair.waiter.id, sender: pair.sender.id)
        let travel = approachTime(pair)
        try? await Task.sleep(nanoseconds: UInt64(travel * 1e9))

        // 2. Match. The two glyphs pulse in their shared fingerprint colour.
        pulse(glyphOf: pair.waiter, pair.fingerprint)
        pulse(glyphOf: pair.sender, pair.fingerprint)
        try? await Task.sleep(nanoseconds: UInt64(d * 0.25 * 1e9))

        // 3. Hand-off. The capsule crosses the gap between the facing hands,
        //    and both robots go: a send has no continuation, a receipt is
        //    consumed.
        handOff(pair, over: d * 0.4)
        try? await Task.sleep(nanoseconds: UInt64(d * 0.4 * 1e9))
        if let e = cardEntities[pair.sender.id] { fade(e, over: d * 0.2) {} }
        if let e = cardEntities[pair.waiter.id] { fade(e, over: d * 0.2) {} }

        motion.releaseApproach([pair.waiter.id, pair.sender.id])
    }

    /// The approach lasts as long as the distance requires at the approach
    /// speed — unlike the other three phases, which take the communication
    /// duration the panel sets.
    private func approachTime(_ pair: Pair) -> Double {
        guard let a = motion.robots[pair.waiter.id]?.position,
              let b = motion.robots[pair.sender.id]?.position,
              panel.approachSpeed > 0
        else { return 0.2 }
        return min(Double(distance(a, b)) / panel.approachSpeed, 3.0)
    }

    private func pulse(glyphOf card: Card, _ fp: Fingerprint) {
        guard let e = cardEntities[card.id],
              let glyph = e.findEntity(named: Tag.glyph(fp.hex)) else { return }
        let big = Transform(scale: glyph.scale * 1.35,
                            rotation: glyph.orientation,
                            translation: glyph.position)
        let back = glyph.transform
        glyph.move(to: big, relativeTo: glyph.parent, duration: 0.12)
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: 120_000_000)
            glyph.move(to: back, relativeTo: glyph.parent, duration: 0.12)
        }
    }

    private func handOff(_ pair: Pair, over duration: Double) {
        guard let senderEntity = cardEntities[pair.sender.id],
              let capsule = senderEntity.findEntity(named: Tag.capsule(pair.fingerprint.hex))
                         ?? firstCapsule(in: senderEntity),
              let waiterEntity = cardEntities[pair.waiter.id]
        else { return }

        // Reparent to the root so the capsule can travel between two robots
        // that are themselves still moving.
        let worldTransform = capsule.transformMatrix(relativeTo: root)
        capsule.removeFromParent()
        root.addChild(capsule)
        capsule.setTransformMatrix(worldTransform, relativeTo: root)

        var target = waiterEntity.transform
        target.translation = waiterEntity.position + SIMD3(Metric.armReach, -0.06, 0)
        target.scale = capsule.scale
        capsule.move(to: target, relativeTo: root,
                     duration: duration, timingFunction: .easeInOut)
    }

    private func firstCapsule(in e: Entity) -> Entity? {
        for child in e.children {
            if child.name.hasPrefix("capsule:") { return child }
            if let found = firstCapsule(in: child) { return found }
        }
        return nil
    }

    /// When a committed communication consumes a robot that other redexes
    /// also wanted, the losing partners briefly show their unmatched glyphs.
    private func showLosers(of comms: [Step]) {
        let taken = Set(comms.compactMap { $0.chan })
        for card in lastScene.livePlane {
            guard let fp = card.chanFp, taken.contains(fp), !busy.contains(card.id),
                  let e = cardEntities[card.id],
                  let glyph = e.findEntity(named: Tag.glyph(fp))
            else { continue }
            let dim = Transform(scale: glyph.scale * 0.8,
                                rotation: glyph.orientation,
                                translation: glyph.position)
            let back = glyph.transform
            glyph.move(to: dim, relativeTo: glyph.parent, duration: 0.15)
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: 300_000_000)
                glyph.move(to: back, relativeTo: glyph.parent, duration: 0.15)
            }
        }
    }

    private func fade(_ e: Entity, over duration: Double, then: @escaping () -> Void) {
        var t = e.transform
        t.scale = .init(repeating: 0.001)
        e.move(to: t, relativeTo: e.parent, duration: duration, timingFunction: .easeIn)
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: UInt64(duration * 1e9))
            then()
        }
    }

    /// The programmer has taken hold of a robot, or let it go. A held robot
    /// holds still; if it is part of a released commit, its partner waits at
    /// the meeting point.
    func hold(_ cardID: String, _ held: Bool) {
        motion.hold(cardID, held)
    }
}
