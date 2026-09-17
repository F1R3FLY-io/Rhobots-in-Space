//  Model.swift
//  What the panel sets, and what the world currently is.
//
//  Two objects. `Panel` is the control panel's state: the things that tune the
//  environment rather than edit the program. Changing any of them never
//  changes the term being run — the layout ones re-lay the scene while it runs
//  because they change only the rendering function, and the pacing ones go to
//  the executive, which enforces them as admission control.
//
//  `World` is what has arrived: the latest scene, the rounds waiting to be
//  animated, and the readouts. It owns the one rule that makes the pacing
//  contract real on this side — **a round is acknowledged when its animation
//  has finished, and not before**. Acknowledging on arrival would turn
//  lockstep into free-running and free-running into an unbounded queue, and
//  the scene would silently fall behind the evaluator.

import Foundation
import Observation

// MARK: - The control panel

@Observable
final class Panel {

    // Execution (§9.2)
    var running = false
    var lockstep = true

    // Concurrency (§9.3)
    /// Round width: up to this many communications are in progress at once.
    var cores = 1
    var coresMax = 1

    // Playback (§9.2)
    /// Seconds for match, hand-off and advance. The approach takes as long as
    /// the distance requires at the approach speed.
    var commDuration: Double = 1.2
    /// The playback buffer bound, in rounds. Ignored in lockstep, whose bound
    /// is one by definition.
    var bufferBound = 128

    // Motion (§7.1, §7.2)
    /// Drift speed of top-level robots. Zero stills the plane.
    var wanderSpeed: Double = 0.10
    /// Speed at which a committed pair converges.
    var approachSpeed: Double = 0.40
    var breakFreely = false

    // Rendering (§3)
    /// Scale of a quoted process relative to its container.
    var shrink: Double = 0.30
    /// Scale below which a quote is drawn as the elision glyph.
    var legibility: Double = 0.05
    /// Distance between successive depth planes, in metres.
    var planeSpacing: Double = 0.40
    /// How long inert robots remain before fading. Zero fades at once;
    /// `inertFadeNever` keeps them.
    var inertFade: Double = 2.0

    static let inertFadeNever: Double = 999

    // Connection
    var host = "127.0.0.1"
    var port = 9099

    /// How many quotation levels are still legible at the current settings,
    /// which is what the executive is asked to send. Past it a subterm comes
    /// back as an elision glyph, with its fingerprint intact so the name stays
    /// comparable.
    ///
    /// σ^k ≥ s_min, so k = ⌊log(s_min) / log(σ)⌋.
    var quoteDepth: Int {
        guard shrink > 0, shrink < 1, legibility > 0 else { return 4 }
        let k = Int((log(legibility) / log(shrink)).rounded(.down))
        return max(1, min(k, 12))
    }

    // MARK: Persistence
    //
    // The panel persists its settings between sessions (§9), so a person picks
    // up with the world tuned the way they left it.

    private static let key = "rhobots.panel"

    func save() {
        let d: [String: Any] = [
            "lockstep": lockstep, "cores": cores,
            "commDuration": commDuration, "bufferBound": bufferBound,
            "wanderSpeed": wanderSpeed, "approachSpeed": approachSpeed,
            "breakFreely": breakFreely, "shrink": shrink,
            "legibility": legibility, "planeSpacing": planeSpacing,
            "inertFade": inertFade, "host": host, "port": port,
        ]
        UserDefaults.standard.set(d, forKey: Panel.key)
    }

    func load() {
        guard let d = UserDefaults.standard.dictionary(forKey: Panel.key) else { return }
        lockstep      = d["lockstep"]      as? Bool   ?? lockstep
        cores         = d["cores"]         as? Int    ?? cores
        commDuration  = d["commDuration"]  as? Double ?? commDuration
        bufferBound   = d["bufferBound"]   as? Int    ?? bufferBound
        wanderSpeed   = d["wanderSpeed"]   as? Double ?? wanderSpeed
        approachSpeed = d["approachSpeed"] as? Double ?? approachSpeed
        breakFreely   = d["breakFreely"]   as? Bool   ?? breakFreely
        shrink        = d["shrink"]        as? Double ?? shrink
        legibility    = d["legibility"]    as? Double ?? legibility
        planeSpacing  = d["planeSpacing"]  as? Double ?? planeSpacing
        inertFade     = d["inertFade"]     as? Double ?? inertFade
        host          = d["host"]          as? String ?? host
        port          = d["port"]          as? Int    ?? port
    }
}

// MARK: - The world

@MainActor
@Observable
final class World {

    let client = ExecutiveClient()
    let panel = Panel()

    /// The world as it should be drawn now.
    private(set) var scene = SceneFrame.empty
    /// The rounds that have arrived and not yet been animated. Bounded on the
    /// executive's side, not here: when this is as long as the buffer bound,
    /// admission pauses there.
    private(set) var queue: [Round] = []
    /// The round being animated, if any.
    private(set) var playing: Round?

    private(set) var stats: Stats?
    private(set) var connection: ExecutiveClient.State = .idle
    private(set) var diagnostics: [Diagnostic] = []
    private(set) var notice: String?

    var source: String = Self.startingTerm

    /// Replication from reflection: the spec's first acceptance test. If this
    /// is legible in the scene, the metaphor is sound.
    static let startingTerm = """
    new x in {
        for(y <- x){ x!(*y) | *y }
      | x!( for(y <- x){ x!(*y) | *y } | stdout!(Nil) )
    }
    """

    init() {
        panel.load()
        client.onState = { [weak self] s in
            guard let self else { return }
            self.connection = s
            if case .connected(_, let resumed) = s {
                // A reconnect finds the world where it was left, so the panel
                // is pushed again and the scene asked for afresh.
                self.pushPanel()
                self.client.resend()
                self.notice = resumed ? "resumed the session on the Mac"
                                      : "connected"
            }
        }
        client.onFrame = { [weak self] f in self?.receive(f) }
    }

    func connect() {
        client.connect(host: panel.host,
                       port: panel.port,
                       sessionID: ExecutiveClient.persistentSessionID)
    }

    private func receive(_ frame: Frame) {
        switch frame {
        case .scene(_, let s):
            // A scene that arrives while a round is being animated would pull
            // the world out from under it. The animation ends by applying the
            // round's own scene, so this is only for the unprompted ones.
            if playing == nil { scene = s }
        case .round(let r):
            queue.append(r)
        case .stats(let s):
            stats = s
            if panel.coresMax != s.coresMax { panel.coresMax = s.coresMax }
            if panel.running != s.running { panel.running = s.running }
        case .quiescent:
            notice = "quiescent — nothing left to reduce"
        case .error(let code, let message):
            notice = "\(code): \(message)"
        case .reply(_, let ok, _, let diags):
            diagnostics = ok ? [] : diags
        case .ready, .unknown:
            break
        }
    }

    // MARK: Playback
    //
    // The scene pulls rounds; nothing pushes them at it. That is what keeps
    // the acknowledgment honest.

    /// Take the next round to animate, if the scene is free to start one.
    func beginNextRound() -> Round? {
        guard playing == nil, !queue.isEmpty else { return nil }
        let r = queue.removeFirst()
        playing = r
        return r
    }

    /// The animation for `round` has finished. Apply its scene and release
    /// admission — in lockstep this is the gate the scheduler is waiting on,
    /// in free-running it is the drain that keeps the buffer moving.
    func finish(_ round: Round) {
        scene = round.scene
        playing = nil
        client.ack(round.round)
    }

    // MARK: Driving the executive

    func deploy() {
        diagnostics = []
        notice = nil
        client.deploy(source) { [weak self] ok, diags in
            guard let self else { return }
            self.diagnostics = diags
            self.notice = ok ? "deployed" : "the term did not parse"
        }
    }

    func reset() {
        queue.removeAll()
        playing = nil
        scene = .empty
        diagnostics = []
        client.reset()
    }

    func playPause() {
        panel.running.toggle()
        panel.running ? client.play() : client.pause()
    }

    func stepOnce() {
        panel.running = false
        client.step()
    }

    /// Push everything the executive needs to know about. Called on connect
    /// and whenever a pacing control moves; the rendering controls never
    /// leave this device, except for the quotation depth, which decides how
    /// much of a term is worth sending at all.
    func pushPanel() {
        client.setCores(panel.cores, max: max(panel.coresMax, panel.cores))
        client.setPacing(lockstep: panel.lockstep)
        client.setBuffer(panel.bufferBound)
        client.setTreeDepth(panel.quoteDepth)
        panel.save()
    }

    /// Whether the evaluator is currently held back by the scene. Shown on the
    /// panel so that a pause reads as back-pressure and not as a stall.
    var admissionPaused: Bool {
        guard let s = stats else { return false }
        return !s.admissionOpen
    }
}
