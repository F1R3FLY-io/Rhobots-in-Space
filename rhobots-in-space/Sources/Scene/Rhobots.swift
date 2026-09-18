//  Rhobots.swift
//  The six elements of pure rho, as things you can look at.
//
//  | term            | rhobot                                                |
//  | --------------- | ----------------------------------------------------- |
//  | 0               | inert robot — grey, eyes closed, never acts           |
//  | x!(Q)           | sending robot — one hand, glyph x, capsule holding Q  |
//  | for(y <- x)P    | waiting robot — hand with glyph x, binder platform y, |
//  |                 | and P one plane behind on a tethering cable           |
//  | P | Q           | adjacency — side by side on one plane                 |
//  | @P              | capsule — P at scale σ, enclosed                      |
//  | *y              | platform + unshrink icon                              |
//
//  **Handedness is a fixed convention.** A waiting robot's hand points right
//  and a sending robot's points left, so a waiter and a sender placed left to
//  right face each other, glyph to glyph, with the binder platform and the
//  capsule in the gap between them. When a committed pair converges, the
//  waiter always takes the left-hand position, so a communication about to
//  happen reads as two robots arriving face to face.

import Foundation
import RealityKit
import SwiftUI
import simd

enum Palette {
    static let inert      = UIColor(white: 0.55, alpha: 1)
    static let body       = UIColor(red: 0.90, green: 0.90, blue: 0.93, alpha: 1)
    static let waiterBody = UIColor(red: 0.86, green: 0.89, blue: 0.94, alpha: 1)
    static let senderBody = UIColor(red: 0.94, green: 0.90, blue: 0.86, alpha: 1)
    static let capsule    = UIColor(white: 0.98, alpha: 0.35)
    static let platform   = UIColor(red: 0.82, green: 0.84, blue: 0.88, alpha: 1)
    static let cable      = UIColor(red: 0.45, green: 0.48, blue: 0.55, alpha: 1)
    static let enclosure  = UIColor(white: 0.95, alpha: 0.18)
}

/// Names given to entities so the animator can find them again. An entity is
/// found by the executive's own identifier, never by position in a list.
enum Tag {
    static func card(_ id: String) -> String { "card:" + id }
    static func glyph(_ fp: String) -> String { "glyph:" + fp }
    static func capsule(_ fp: String) -> String { "capsule:" + fp }
    static func binder(_ fp: String) -> String { "binder:" + fp }
    static func cable(_ fp: String) -> String { "cable:" + fp }
    static func body(_ fp: String) -> String { "body:" + fp }
}

@MainActor
struct RhobotFactory {

    let panel: Panel

    // MARK: A whole card

    /// One card of the scene: its robots, its continuations, and the cables
    /// that join them.
    func build(card: Card, placements: [Placement]) -> Entity {
        let root = Entity()
        root.name = Tag.card(card.id)
        for p in placements { root.addChild(entity(for: p)) }
        return root
    }

    // MARK: One placement

    func entity(for placement: Placement) -> Entity {
        switch placement {
        case .inert(let n):
            return inertRobot(n)

        case .sender(let n, let chan, let capsule, let persistent):
            return sendingRobot(n, chan: chan, capsule: capsule, persistent: persistent)

        case .waiter(let n, let binds, let continuation):
            return waitingRobot(n, binds: binds, continuation: continuation)

        case .platform(let n, let name):
            return dropPlatform(n, name: name)

        case .slot(let n, let label):
            return slotPlatform(n, label: label)

        case .enclosure(let n, let count, let body):
            return enclosure(n, count: count, body: body)

        case .elision(let n):
            return elisionGlyph(n)
        }
    }

    // MARK: The vocabulary

    /// Grey, eyes closed. Never acts, never wanders. Inert robots produced by
    /// reduction fade after the delay the panel sets, which is P | 0 ≡ P made
    /// visible — unless the programmer pins them, which is a viewing
    /// preference with no semantic effect.
    private func inertRobot(_ n: Placement.Node) -> Entity {
        let e = ModelEntity(
            mesh: .generateBox(size: SIMD3(Metric.bodyWidth, Metric.bodyHeight, Metric.bodyDepth),
                               cornerRadius: 0.03),
            materials: [SimpleMaterial(color: Palette.inert, roughness: 0.9, isMetallic: false)])
        e.name = Tag.body(n.fingerprint.hex)
        // Closed eyes: two flat slits rather than the spheres a live robot has.
        for side: Float in [-1, 1] {
            let eye = ModelEntity(
                mesh: .generateBox(size: SIMD3(0.025, 0.004, 0.005)),
                materials: [SimpleMaterial(color: .darkGray, roughness: 1, isMetallic: false)])
            eye.position = SIMD3(side * 0.04, 0.06, Metric.bodyDepth / 2)
            e.addChild(eye)
        }
        return place(e, n)
    }

    /// One appendage, pointing left, bearing the channel glyph and offering a
    /// capsule that holds the payload at scale σ. Nothing stands behind it: a
    /// send has no continuation.
    private func sendingRobot(_ n: Placement.Node,
                              chan: GlyphPlacement,
                              capsule: [Placement],
                              persistent: Bool) -> Entity {
        let root = Entity()
        let body = ModelEntity(
            mesh: .generateBox(size: SIMD3(Metric.bodyWidth, Metric.bodyHeight, Metric.bodyDepth),
                               cornerRadius: 0.03),
            materials: [SimpleMaterial(color: Palette.senderBody, roughness: 0.7, isMetallic: false)])
        body.name = Tag.body(n.fingerprint.hex)
        root.addChild(body)
        root.addChild(eyes(open: true))

        // The hand points left, so a sender to the right of a waiter faces it.
        let arm = armEntity(toward: -1)
        root.addChild(arm)

        let g = glyphEntity(chan)
        g.position = SIMD3(-Metric.armReach, 0.02, 0)
        root.addChild(g)

        // The capsule sits in the gap between the two hands, where it will
        // cross to the binder platform when the comm fires.
        let cap = capsuleEntity(fingerprint: n.fingerprint, contents: capsule)
        cap.position = SIMD3(-Metric.armReach, -0.06, 0)
        root.addChild(cap)

        if persistent {
            // `!!`: a send that re-issues itself. A second, fainter outline
            // behind the body says the robot will still be here afterwards.
            let ghost = ModelEntity(
                mesh: .generateBox(size: SIMD3(Metric.bodyWidth * 1.06,
                                               Metric.bodyHeight * 1.06,
                                               Metric.bodyDepth * 0.4),
                                   cornerRadius: 0.03),
                materials: [SimpleMaterial(color: Palette.capsule, roughness: 1, isMetallic: false)])
            ghost.position = SIMD3(0, 0, -Metric.bodyDepth * 0.5)
            root.addChild(ghost)
        }
        return place(root, n)
    }

    /// One appendage in two parts — a hand bearing the channel glyph, and a
    /// held platform labelled with the binder, awaiting its payload — with the
    /// continuation one plane behind, joined by a tethering cable.
    private func waitingRobot(_ n: Placement.Node,
                              binds: [BindPlacement],
                              continuation: [Placement]) -> Entity {
        let root = Entity()
        let body = ModelEntity(
            mesh: .generateBox(size: SIMD3(Metric.bodyWidth, Metric.bodyHeight, Metric.bodyDepth),
                               cornerRadius: 0.03),
            materials: [SimpleMaterial(color: Palette.waiterBody, roughness: 0.7, isMetallic: false)])
        body.name = Tag.body(n.fingerprint.hex)
        root.addChild(body)
        root.addChild(eyes(open: true))

        // One appendage per receipt. A join is one robot with several hands,
        // all of which must be served before it commits.
        for (i, bind) in binds.enumerated() {
            let y = Float(i) * -0.09
            // The hand points right, toward the sender it will face.
            let arm = armEntity(toward: 1)
            arm.position.y += y
            root.addChild(arm)

            let g = glyphEntity(bind.chan)
            g.position = SIMD3(Metric.armReach, 0.02 + y, 0)
            root.addChild(g)

            // Peek and persistent receipts are a different physical form, not
            // a different colour: a peek reads without consuming, so its hand
            // is open; a persistent receipt carries the same ghost outline as
            // a persistent send.
            if bind.kind != .linear {
                let mark = ModelEntity(
                    mesh: .generateBox(size: SIMD3(0.05, 0.008, 0.008)),
                    materials: [SimpleMaterial(color: Palette.cable, roughness: 1, isMetallic: false)])
                mark.position = SIMD3(Metric.armReach, 0.075 + y, 0.01)
                root.addChild(mark)
            }

            for (j, binder) in bind.binders.enumerated() {
                let b = glyphEntity(binder)
                b.name = Tag.binder(binder.fingerprint.hex)
                b.position = SIMD3(Metric.armReach, -0.06 + y - Float(j) * 0.05, 0)
                root.addChild(b)
            }
        }

        // The continuation stands one plane behind, and follows with a damped
        // lag, so a wandering robot visibly drags its future behind it.
        let cont = Entity()
        cont.name = "continuation"
        for p in continuation { cont.addChild(entity(for: p)) }
        root.addChild(cont)

        if !continuation.isEmpty {
            root.addChild(cableEntity(from: .zero,
                                      to: SIMD3(0, 0, -Float(panel.planeSpacing)),
                                      fingerprint: n.fingerprint))
        }
        return place(root, n)
    }

    /// A full-size platform under a dashed silhouette, marked with the
    /// unshrink icon: where the payload will land, at the platform's own
    /// scale. Unshrinking is relative — a `*y` inside a capsule restores its
    /// payload to the capsule's scale, not to the plane's.
    private func dropPlatform(_ n: Placement.Node, name: GlyphPlacement) -> Entity {
        let root = Entity()
        let plate = ModelEntity(
            mesh: .generateBox(size: SIMD3(Metric.platformSize, 0.012, Metric.platformSize),
                               cornerRadius: 0.004),
            materials: [SimpleMaterial(color: Palette.platform, roughness: 0.8, isMetallic: false)])
        root.addChild(plate)

        // The dashed silhouette: the shape of the robot that is not here yet.
        let ghost = ModelEntity(
            mesh: .generateBox(size: SIMD3(Metric.bodyWidth * 0.8,
                                           Metric.bodyHeight * 0.8,
                                           0.004),
                               cornerRadius: 0.02),
            materials: [SimpleMaterial(color: Palette.capsule, roughness: 1, isMetallic: false)])
        ghost.position = SIMD3(0, Metric.bodyHeight * 0.44, 0)
        root.addChild(ghost)

        // The unshrink icon: an outward chevron pair, which must never look
        // like the inspector's magnifier.
        for s: Float in [-1, 1] {
            let arrow = ModelEntity(
                mesh: .generateBox(size: SIMD3(0.018, 0.004, 0.004)),
                materials: [SimpleMaterial(color: name.fingerprint.uiColor,
                                           roughness: 1, isMetallic: false)])
            arrow.position = SIMD3(s * 0.022, 0.014, 0)
            root.addChild(arrow)
        }

        let label = glyphEntity(name)
        label.position = SIMD3(0, 0.03, Metric.platformSize / 2)
        root.addChild(label)
        return place(root, n)
    }

    private func slotPlatform(_ n: Placement.Node, label: String) -> Entity {
        let e = ModelEntity(
            mesh: .generateBox(size: SIMD3(Metric.glyphSize, 0.01, Metric.glyphSize),
                               cornerRadius: 0.004),
            materials: [SimpleMaterial(color: Palette.platform, roughness: 0.8, isMetallic: false)])
        e.addChild(text(label, size: 0.018, color: .darkGray, y: 0.012))
        return place(e, n)
    }

    /// `new`: an enclosure on the plane. Channels minted inside carry
    /// unforgeable fingerprints, which is why they are drawn ringed rather
    /// than capsuled — they quote nothing.
    private func enclosure(_ n: Placement.Node, count: Int, body: [Placement]) -> Entity {
        let root = Entity()
        let wall = ModelEntity(
            mesh: .generateBox(size: SIMD3(max(n.width, 0.3) + 0.12, Metric.bodyHeight + 0.14, 0.006),
                               cornerRadius: 0.02),
            materials: [SimpleMaterial(color: Palette.enclosure, roughness: 1, isMetallic: false)])
        wall.position = SIMD3(0, 0, -Metric.bodyDepth)
        root.addChild(wall)
        root.addChild(text("new ×\(count)", size: 0.02, color: .gray,
                           y: Metric.bodyHeight / 2 + 0.05))
        for p in body { root.addChild(entity(for: p)) }
        return place(root, n)
    }

    /// Below legibility a quoted subterm is a capsule containing an ellipsis.
    /// It carries the fingerprint, so an elided name is still comparable —
    /// and inspecting it opens a magnified inspector, which must never
    /// resemble the unquote animation.
    private func elisionGlyph(_ n: Placement.Node) -> Entity {
        let shell = ModelEntity(
            mesh: .generateBox(size: SIMD3(Metric.capsuleSize, Metric.capsuleSize * 0.7, 0.02),
                               cornerRadius: Metric.capsuleSize * 0.3),
            materials: [SimpleMaterial(color: Palette.capsule, roughness: 0.4, isMetallic: false)])
        shell.addChild(ring(n.fingerprint, radius: Metric.capsuleSize * 0.52))
        for i in -1...1 {
            let dot = ModelEntity(
                mesh: .generateSphere(radius: 0.004),
                materials: [SimpleMaterial(color: n.fingerprint.uiColor,
                                           roughness: 1, isMetallic: false)])
            dot.position = SIMD3(Float(i) * 0.012, 0, 0.012)
            shell.addChild(dot)
        }
        return place(shell, n)
    }

    // MARK: Parts

    private func glyphEntity(_ g: GlyphPlacement) -> Entity {
        let root = Entity()
        root.name = Tag.glyph(g.fingerprint.hex)
        switch g {
        case .capsule(let fp, let contents, let scale):
            let shell = capsuleShell(fp)
            for p in contents { shell.addChild(entity(for: p)) }
            shell.scale = SIMD3(repeating: max(scale, 0.02))
            root.addChild(shell)
        case .elided(let fp, let scale):
            let shell = capsuleShell(fp)
            for i in -1...1 {
                let dot = ModelEntity(
                    mesh: .generateSphere(radius: 0.004),
                    materials: [SimpleMaterial(color: fp.uiColor, roughness: 1, isMetallic: false)])
                dot.position = SIMD3(Float(i) * 0.012, 0, 0.012)
                shell.addChild(dot)
            }
            shell.scale = SIMD3(repeating: max(scale, 0.02))
            root.addChild(shell)
        case .slot(let fp, let label, _):
            let plate = ModelEntity(
                mesh: .generateBox(size: SIMD3(Metric.glyphSize, 0.01, Metric.glyphSize),
                                   cornerRadius: 0.004),
                materials: [SimpleMaterial(color: Palette.platform, roughness: 0.8, isMetallic: false)])
            plate.addChild(ring(fp, radius: Metric.glyphSize * 0.6))
            plate.addChild(text(label, size: 0.016, color: .darkGray, y: 0.012))
            root.addChild(plate)
        case .unforgeable(let fp, let label, _):
            let disc = ModelEntity(
                mesh: .generateCylinder(height: 0.008, radius: Metric.glyphSize * 0.5),
                materials: [SimpleMaterial(color: fp.uiColor, roughness: 0.5, isMetallic: true)])
            disc.addChild(text(label, size: 0.014, color: .white, y: 0.008))
            root.addChild(disc)
        }
        return root
    }

    private func capsuleShell(_ fp: Fingerprint) -> Entity {
        let shell = ModelEntity(
            mesh: .generateBox(size: SIMD3(Metric.capsuleSize,
                                           Metric.capsuleSize * 0.8,
                                           Metric.capsuleSize * 0.6),
                               cornerRadius: Metric.capsuleSize * 0.3),
            materials: [SimpleMaterial(color: Palette.capsule, roughness: 0.3, isMetallic: false)])
        shell.name = Tag.capsule(fp.hex)
        shell.addChild(ring(fp, radius: Metric.capsuleSize * 0.55))
        return shell
    }

    private func capsuleEntity(fingerprint: Fingerprint, contents: [Placement]) -> Entity {
        let shell = capsuleShell(fingerprint)
        for p in contents { shell.addChild(entity(for: p)) }
        return shell
    }

    /// The colour ring that carries a fingerprint. Drawn as a ring of beads
    /// rather than a tinted body, so the cue survives a body colour and reads
    /// at the edge of legibility.
    private func ring(_ fp: Fingerprint, radius: Float) -> Entity {
        let root = Entity()
        let beads = 6 + fp.pattern
        for i in 0..<beads {
            let a = Float(i) / Float(beads) * 2 * .pi
            let bead = ModelEntity(
                mesh: .generateSphere(radius: radius * 0.16),
                materials: [SimpleMaterial(color: fp.uiColor, roughness: 0.4, isMetallic: false)])
            bead.position = SIMD3(cos(a) * radius, sin(a) * radius, 0.006)
            root.addChild(bead)
        }
        return root
    }

    private func armEntity(toward side: Float) -> Entity {
        let arm = ModelEntity(
            mesh: .generateBox(size: SIMD3(Metric.armReach, 0.016, 0.016), cornerRadius: 0.008),
            materials: [SimpleMaterial(color: Palette.platform, roughness: 0.8, isMetallic: false)])
        arm.position = SIMD3(side * Metric.armReach / 2, 0.02, 0)
        return arm
    }

    private func eyes(open: Bool) -> Entity {
        let root = Entity()
        for side: Float in [-1, 1] {
            let eye = ModelEntity(
                mesh: open ? .generateSphere(radius: 0.012)
                           : .generateBox(size: SIMD3(0.025, 0.004, 0.005)),
                materials: [SimpleMaterial(color: .darkGray, roughness: 0.3, isMetallic: false)])
            eye.position = SIMD3(side * 0.04, 0.06, Metric.bodyDepth / 2)
            root.addChild(eye)
        }
        return root
    }

    /// A tethering cable: a chain of magnetically linked segments, visually
    /// distinct from the thin tinted binder tethers that appear only on gaze.
    /// It is drawn in segments because it has to be able to part at a crossing
    /// and snap back together.
    func cableEntity(from a: SIMD3<Float>, to b: SIMD3<Float>,
                     fingerprint: Fingerprint) -> Entity {
        let root = Entity()
        root.name = Tag.cable(fingerprint.hex)
        let segments = 8
        let step = (b - a) / Float(segments)
        for i in 0..<segments {
            let seg = ModelEntity(
                mesh: .generateBox(size: SIMD3(0.012, 0.012, length(step) * 0.8),
                                   cornerRadius: 0.006),
                materials: [SimpleMaterial(color: Palette.cable, roughness: 0.6, isMetallic: false)])
            seg.name = "segment\(i)"
            seg.position = a + step * (Float(i) + 0.5)
            root.addChild(seg)
        }
        return root
    }

    private func text(_ s: String, size: CGFloat, color: UIColor, y: Float) -> Entity {
        guard !s.isEmpty else { return Entity() }
        let mesh = MeshResource.generateText(
            s,
            extrusionDepth: 0.001,
            font: .systemFont(ofSize: size),
            containerFrame: .zero,
            alignment: .center,
            lineBreakMode: .byTruncatingTail)
        let e = ModelEntity(mesh: mesh,
                            materials: [UnlitMaterial(color: color)])
        e.position = SIMD3(0, y, 0)
        return e
    }

    private func place(_ e: Entity, _ n: Placement.Node) -> Entity {
        e.position = n.position
        e.scale = SIMD3(repeating: max(n.scale, 0.02))
        return e
    }
}

extension Fingerprint {
    var uiColor: UIColor { UIColor(color) }
}
