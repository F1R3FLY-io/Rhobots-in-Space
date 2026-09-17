//  Layout.swift
//  The two orthogonal axes, and the function that places robots on them.
//
//  The whole spatial semantics is two rules that never interfere:
//
//    **Depth carries guardedness.** The plane nearest the programmer, z₀,
//    holds exactly the top-level parallel composition — the only robots that
//    can act. A receive's continuation stands one plane behind it. Depth
//    increases away from the viewer, which in RealityKit is −z, and the sign
//    is fixed once here and never inverted locally.
//
//    **Scale carries quotation.** A process inside a quote is drawn at σ times
//    its container's scale, always inside a capsule, so quotation is marked by
//    more than size. Below the legibility threshold it becomes an elision
//    glyph — which still carries the fingerprint, so an elided name stays
//    comparable.
//
//  Distant robots do subtend smaller visual angles, but that is perspective,
//  not quotation. The capsule is what tells the two apart.
//
//  This file computes placements only. Nothing here makes an entity; that is
//  Rhobots.swift, which walks what this produces.

import Foundation
import simd
import SwiftUI

// MARK: - Fingerprints

/// A name's visual identity, derived from the content hash of the process it
/// quotes — the same 32 bytes the store keys by.
///
/// Two names are equivalent exactly when their canonical forms coincide, so
/// matching fingerprints are the visual witness that a communication is
/// possible. Collisions are resolved by the evaluator, never by the eye: this
/// is a cue, not the test.
struct Fingerprint: Equatable, Hashable {
    let hex: String

    init(_ hex: String) { self.hex = hex }

    private var bytes: [UInt8] {
        var out: [UInt8] = []
        var i = hex.startIndex
        while i < hex.endIndex, out.count < 6 {
            let j = hex.index(i, offsetBy: 2, limitedBy: hex.endIndex) ?? hex.endIndex
            out.append(UInt8(hex[i..<j], radix: 16) ?? 0)
            i = j
        }
        while out.count < 6 { out.append(0) }
        return out
    }

    /// The colour ring. Hue takes the whole circle; saturation and brightness
    /// are kept in a narrow band so that every fingerprint is legible against
    /// the passthrough and none is muddy.
    var color: Color {
        let b = bytes
        let hue = Double(UInt16(b[0]) << 8 | UInt16(b[1])) / 65535.0
        let sat = 0.55 + Double(b[2]) / 255.0 * 0.35
        let bri = 0.72 + Double(b[3]) / 255.0 * 0.24
        return Color(hue: hue, saturation: sat, brightness: bri)
    }

    /// A second cue for people who cannot rely on hue, and a tiebreak for two
    /// fingerprints that land near each other on the wheel.
    var pattern: Int { Int(bytes[4] % 6) }

    /// Short form, for a label under a glyph.
    var short: String { String(hex.prefix(8)) }
}

// MARK: - What a placement is

/// One drawable element, already positioned.
///
/// The tree of placements mirrors the term, but every node carries the plane
/// and scale it ended up at, so the entity builder does no arithmetic and the
/// animator can ask where anything is.
indirect enum Placement {
    case inert(Node)
    case sender(Node, chan: GlyphPlacement, capsule: [Placement], persistent: Bool)
    case waiter(Node, binds: [BindPlacement], continuation: [Placement])
    /// `*y`: a full-size platform under a dashed silhouette, with the unshrink
    /// icon. Substitution fills it at the platform's own scale.
    case platform(Node, name: GlyphPlacement)
    /// A name variable standing where a process should be, before anything has
    /// filled it.
    case slot(Node, label: String)
    /// An enclosure on the plane; channels minted inside carry unforgeable
    /// fingerprints.
    case enclosure(Node, count: Int, body: [Placement])
    /// Too small to read: a capsule containing an ellipsis, with the
    /// fingerprint still on it.
    case elision(Node)

    struct Node {
        let fingerprint: Fingerprint
        /// Metres from the scene root, x across, y up, z toward the viewer.
        var position: SIMD3<Float>
        /// Absolute scale, after k levels of quotation: σᵏ.
        var scale: Float
        /// Which depth plane. Negative planes stand behind the programmer.
        var plane: Int
        /// Width this element occupies when packed along x with its siblings.
        var width: Float
        /// The term's own size and depth, for level-of-detail decisions.
        var termSize: Int
    }

    var node: Node {
        switch self {
        case .inert(let n), .sender(let n, _, _, _), .waiter(let n, _, _),
             .platform(let n, _), .slot(let n, _), .enclosure(let n, _, _),
             .elision(let n):
            return n
        }
    }
}

/// A glyph as placed on a hand or in a slot.
indirect enum GlyphPlacement {
    /// A capsule holding a process at σ times the hand's scale.
    case capsule(Fingerprint, contents: [Placement], scale: Float)
    /// A capsule whose contents are past legibility.
    case elided(Fingerprint, scale: Float)
    /// A small platform labelled by a binder, where a shrunken robot will
    /// stand once substitution fills it.
    case slot(Fingerprint, label: String, scale: Float)
    /// A name that quotes nothing.
    case unforgeable(Fingerprint, label: String, scale: Float)

    var fingerprint: Fingerprint {
        switch self {
        case .capsule(let f, _, _), .elided(let f, _),
             .slot(let f, _, _), .unforgeable(let f, _, _):
            return f
        }
    }
}

struct BindPlacement {
    let kind: Bind.Kind
    let arrow: String
    let chan: GlyphPlacement
    /// One binder platform per pattern, awaiting its payload.
    let binders: [GlyphPlacement]
}

// MARK: - The layout function

/// Sizes, in metres at scale 1. Starting points, to be tuned on device.
enum Metric {
    static let bodyWidth: Float = 0.18
    static let bodyHeight: Float = 0.26
    static let bodyDepth: Float = 0.12
    /// Clear space between two robots packed along x.
    static let gutter: Float = 0.10
    /// How far a hand reaches from the body.
    static let armReach: Float = 0.13
    static let glyphSize: Float = 0.07
    static let capsuleSize: Float = 0.09
    static let platformSize: Float = 0.10
    /// Eye height the live plane is centred on.
    static let planeHeight: Float = 1.35
    /// Past this many planes nothing is legible, so the chain stops being
    /// drawn. A term can nest receives far deeper than a room can show.
    static let maxPlanes = 12
}

struct Layout {
    let shrink: Float
    let legibility: Float
    let planeSpacing: Float

    init(panel: Panel) {
        shrink = Float(panel.shrink)
        legibility = Float(panel.legibility)
        planeSpacing = Float(panel.planeSpacing)
    }

    /// L(P, z, s) for one card, laid out from the live plane.
    func place(_ form: Form, plane: Int = 0, scale: Float = 1, x: Float = 0) -> [Placement] {
        lay(form, plane: plane, scale: scale, originX: x)
    }

    /// Everything on the live plane, packed along x. Left-to-right order
    /// carries no meaning — the robots wander, and the programmer may move
    /// them — but the executive's order is encoding order, so it is stable
    /// frame to frame and a robot keeps its place across a re-layout.
    func placeLivePlane(_ cards: [Card]) -> [(Card, [Placement])] {
        var out: [(Card, [Placement])] = []
        var cursor: Float = 0
        for card in cards {
            let ps = lay(card.tree, plane: 0, scale: 1, originX: cursor)
            let width = ps.map { $0.node.width }.reduce(0, +)
                      + Float(max(0, ps.count - 1)) * Metric.gutter
            cursor += width + Metric.gutter
            out.append((card, ps))
        }
        // Centre the plane on the programmer.
        let total = max(cursor - Metric.gutter, 0)
        let shift = -total / 2
        return out.map { (card, ps) in (card, ps.map { shifted($0, by: shift) }) }
    }

    // MARK: The clauses

    private func lay(_ form: Form, plane: Int, scale: Float, originX: Float) -> [Placement] {
        // Depth stops being drawn before it stops existing.
        guard plane <= Metric.maxPlanes else { return [] }

        let fp = Fingerprint(form.meta.fp)
        let pos = position(plane: plane, x: originX)

        switch form {

        // L(0, z, s)
        case .nilProc, .wild, .variable:
            return [.inert(node(fp, pos, scale, plane, Metric.bodyWidth * scale, form.meta.size))]

        // L(P | Q, z, s) = L(P,z,s) ⊎ L(Q,z,s), packed along x
        case .par(_, let parts):
            var out: [Placement] = []
            var cursor = originX
            for p in parts {
                let ps = lay(p, plane: plane, scale: scale, originX: cursor)
                let w = ps.map { $0.node.width }.reduce(0, +)
                      + Float(max(0, ps.count - 1)) * Metric.gutter * scale
                cursor += w + Metric.gutter * scale
                out.append(contentsOf: ps)
            }
            return out

        // L(x!(Q), z, s): a sender with glyph G(x,s) and capsule L(Q, 0, σs).
        // No continuation stands behind it.
        case .send(let m, let chan, let persistent, let args):
            let g = glyph(chan, scale: scale)
            // Depth inside a capsule is local: a receive in a payload lays out
            // its continuation behind it at the capsule's scale, starting from
            // the capsule's own plane zero.
            let inner = scale * shrink
            var capsule: [Placement] = []
            if inner >= legibility {
                var cursor = originX
                for a in args {
                    let ps = lay(a, plane: 0, scale: inner, originX: cursor)
                    cursor += (ps.map { $0.node.width }.reduce(0, +)) + Metric.gutter * inner
                    capsule.append(contentsOf: ps)
                }
            } else {
                capsule = [.elision(node(fp, pos, inner, plane, Metric.capsuleSize * inner, m.size))]
            }
            let n = node(fp, pos, scale, plane,
                         (Metric.bodyWidth + Metric.armReach) * scale, m.size)
            return [.sender(n, chan: g, capsule: capsule, persistent: persistent)]

        // L(for(y <- x)P, z, s): a waiter with glyph and binder platform,
        // ⊎ L(P, z+1, s) — the continuation, one plane behind, on a cable.
        case .receive(let m, let binds, let body):
            let placed = binds.map { b in
                BindPlacement(kind: b.kind,
                              arrow: b.arrow,
                              chan: glyph(b.chan, scale: scale),
                              binders: b.pats.map { glyph($0, scale: scale) })
            }
            let cont = lay(body, plane: plane + 1, scale: scale, originX: originX)
            let n = node(fp, pos, scale, plane,
                         (Metric.bodyWidth + Metric.armReach) * scale, m.size)
            return [.waiter(n, binds: placed, continuation: cont)]

        case .newScope(let m, let count, let body):
            let inner = lay(body, plane: plane, scale: scale, originX: originX)
            let w = inner.map { $0.node.width }.reduce(0, +) + Metric.gutter * scale
            return [.enclosure(node(fp, pos, scale, plane, w, m.size),
                               count: count, body: inner)]

        // L(*y, z, s): the platform with the unshrink icon.
        case .drop(let m, let name):
            let n = node(fp, pos, scale, plane, Metric.platformSize * scale, m.size)
            return [.platform(n, name: glyph(name, scale: scale))]

        case .elided(let m):
            return [.elision(node(fp, pos, scale, plane, Metric.capsuleSize * scale, m.size))]
        }
    }

    /// G(@P, s) = L(P, 0, σs), drawn as the elision glyph below s_min;
    /// G(y, s) is the glyph-slot platform.
    private func glyph(_ g: Glyph, scale: Float) -> GlyphPlacement {
        let inner = scale * shrink
        switch g {
        case .quote(let fp, let proc):
            let f = Fingerprint(fp)
            guard inner >= legibility, let proc else { return .elided(f, scale: inner) }
            return .capsule(f, contents: lay(proc, plane: 0, scale: inner, originX: 0),
                            scale: inner)
        case .elidedQuote(let fp, _, _):
            return .elided(Fingerprint(fp), scale: inner)
        case .slot(let fp, let label, _, _):
            return .slot(Fingerprint(fp), label: label, scale: scale)
        case .unforgeable(let fp, let label):
            return .unforgeable(Fingerprint(fp), label: label, scale: scale)
        }
    }

    // MARK: Helpers

    /// The one place the depth sign is fixed. Depth increases away from the
    /// viewer, which is −z; nothing else in the app may invert it.
    private func position(plane: Int, x: Float) -> SIMD3<Float> {
        SIMD3(x, Metric.planeHeight, -Float(plane) * planeSpacing)
    }

    private func node(_ fp: Fingerprint, _ pos: SIMD3<Float>, _ scale: Float,
                      _ plane: Int, _ width: Float, _ size: Int) -> Placement.Node {
        Placement.Node(fingerprint: fp, position: pos, scale: scale,
                       plane: plane, width: width, termSize: size)
    }

    private func shifted(_ p: Placement, by dx: Float) -> Placement {
        var n = p.node
        n.position.x += dx
        switch p {
        case .inert: return .inert(n)
        case .sender(_, let c, let cap, let per):
            return .sender(n, chan: c, capsule: cap.map { shifted($0, by: dx) },
                           persistent: per)
        case .waiter(_, let b, let cont):
            return .waiter(n, binds: b, continuation: cont.map { shifted($0, by: dx) })
        case .platform(_, let name): return .platform(n, name: name)
        case .slot(_, let l):        return .slot(n, label: l)
        case .enclosure(_, let c, let body):
            return .enclosure(n, count: c, body: body.map { shifted($0, by: dx) })
        case .elision: return .elision(n)
        }
    }
}
