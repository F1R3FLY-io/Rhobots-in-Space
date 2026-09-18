//  Wire.swift
//  The protocol the executive speaks, and the client that speaks it.
//
//  Two halves. `Wire` is the frames and the scene tree, decoded exactly as
//  `f1r3x-serve` writes them. `ExecutiveClient` is a connection that survives
//  the life a headset actually has — sleeping, backgrounding, roaming between
//  access points — by reconnecting to a session named in the app's defaults
//  rather than to one the server issued.
//
//  Nothing here draws anything. The scene tree is handed to Layout, which
//  evaluates L(P, z, s) on it.

import Foundation

// MARK: - The scene tree

/// A glyph on a hand or in a slot: what a name looks like.
///
/// Every glyph carries `fp`, the 32-byte content hash the store keys by.
/// Two glyphs with the same fingerprint are the visual witness that a
/// communication is possible — but only the witness. The store decides.
enum Glyph: Decodable {
    /// A capsule: the quoted process, drawn one scale level in.
    case quote(fp: String, proc: Form?)
    /// A capsule too deep to draw, standing in for its contents. It keeps the
    /// fingerprint, so an elided name stays comparable.
    case elidedQuote(fp: String, size: Int, depth: Int)
    /// A small platform labelled by its binder, where a shrunken robot will
    /// stand once substitution fills it.
    case slot(fp: String, label: String, bound: Bool, index: Int)
    /// A name minted by `new`, which quotes nothing.
    case unforgeable(fp: String, label: String)

    var fingerprint: String {
        switch self {
        case .quote(let fp, _), .elidedQuote(let fp, _, _),
             .slot(let fp, _, _, _), .unforgeable(let fp, _):
            return fp
        }
    }

    /// What to write under the glyph when there is room.
    var label: String {
        switch self {
        case .quote, .elidedQuote: return ""
        case .slot(_, let l, _, _), .unforgeable(_, let l, _): return l
        }
    }

    private enum Keys: String, CodingKey {
        case g, fp, proc, label, bound, index, elided, size, depth
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        let fp = try c.decode(String.self, forKey: .fp)
        switch try c.decode(String.self, forKey: .g) {
        case "quote":
            if try c.decodeIfPresent(Bool.self, forKey: .elided) == true {
                self = .elidedQuote(fp: fp,
                                    size: try c.decodeIfPresent(Int.self, forKey: .size) ?? 0,
                                    depth: try c.decodeIfPresent(Int.self, forKey: .depth) ?? 0)
            } else {
                self = .quote(fp: fp, proc: try c.decodeIfPresent(Form.self, forKey: .proc))
            }
        case "slot":
            self = .slot(fp: fp,
                         label: try c.decodeIfPresent(String.self, forKey: .label) ?? "",
                         bound: try c.decodeIfPresent(Bool.self, forKey: .bound) ?? false,
                         index: try c.decodeIfPresent(Int.self, forKey: .index) ?? 0)
        default:
            self = .unforgeable(fp: fp,
                                label: try c.decodeIfPresent(String.self, forKey: .label) ?? "u_")
        }
    }
}

/// One bind of a receipt: a hand, its patterns, and how it consumes.
struct Bind: Decodable {
    enum Kind: String, Decodable { case linear, persistent, peek }
    let kind: Kind
    /// `<-`, `<=` or `<<-`, to write beside the hand.
    let arrow: String
    let binders: Int
    let chan: Glyph
    let pats: [Glyph]
}

/// A process, in the rhobot vocabulary of the spec's §4.
///
/// The cases are the six elements of pure rho plus what K1 adds and what the
/// budget elides. `fp`, `size` and `depth` ride on every one of them: the
/// first so the renderer can follow an object across frames without the
/// executive inventing handles, the last two so it can decide how large to
/// draw it.
indirect enum Form: Decodable {
    case nilProc(Meta)
    case par(Meta, [Form])
    case send(Meta, chan: Glyph, persistent: Bool, args: [Form])
    case receive(Meta, binds: [Bind], body: Form)
    case newScope(Meta, count: Int, body: Form)
    /// `*x`: the full-size platform under a dashed silhouette, with the
    /// unshrink icon. Substitution fills it at the platform's own scale.
    case drop(Meta, name: Glyph)
    case variable(Meta, bound: Bool, index: Int)
    case wild(Meta)
    /// Past the quotation budget: an ellipsis capsule carrying a fingerprint.
    case elided(Meta)

    struct Meta {
        let fp: String
        let size: Int
        let depth: Int
    }

    var meta: Meta {
        switch self {
        case .nilProc(let m), .par(let m, _), .send(let m, _, _, _),
             .receive(let m, _, _), .newScope(let m, _, _), .drop(let m, _),
             .variable(let m, _, _), .wild(let m), .elided(let m):
            return m
        }
    }

    private enum Keys: String, CodingKey {
        case f, fp, size, depth, parts, chan, persistent, args
        case binds, body, count, name, bound, index
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        let m = Meta(fp: try c.decode(String.self, forKey: .fp),
                     size: try c.decodeIfPresent(Int.self, forKey: .size) ?? 1,
                     depth: try c.decodeIfPresent(Int.self, forKey: .depth) ?? 1)
        switch try c.decode(String.self, forKey: .f) {
        case "nil":
            self = .nilProc(m)
        case "par":
            self = .par(m, try c.decodeIfPresent([Form].self, forKey: .parts) ?? [])
        case "send":
            self = .send(m,
                         chan: try c.decode(Glyph.self, forKey: .chan),
                         persistent: try c.decodeIfPresent(Bool.self, forKey: .persistent) ?? false,
                         args: try c.decodeIfPresent([Form].self, forKey: .args) ?? [])
        case "receive":
            self = .receive(m,
                            binds: try c.decodeIfPresent([Bind].self, forKey: .binds) ?? [],
                            body: try c.decode(Form.self, forKey: .body))
        case "new":
            self = .newScope(m,
                             count: try c.decodeIfPresent(Int.self, forKey: .count) ?? 0,
                             body: try c.decode(Form.self, forKey: .body))
        case "drop":
            self = .drop(m, name: try c.decode(Glyph.self, forKey: .name))
        case "var":
            self = .variable(m,
                             bound: try c.decodeIfPresent(Bool.self, forKey: .bound) ?? false,
                             index: try c.decodeIfPresent(Int.self, forKey: .index) ?? 0)
        case "wild":
            self = .wild(m)
        default:
            self = .elided(m)
        }
    }
}

// MARK: - Frames

/// One drawable thing, as the executive names it.
///
/// `id` is the executive's own identifier, not one this client made up, so an
/// object keeps its entity — and therefore its position and its animation
/// state — across frames.
struct Card: Decodable, Identifiable {
    enum Kind: String, Decodable {
        /// A component waiting to run: the things that move.
        case rhobot
        /// Data at rest on a channel: the things that sit.
        case tuple
        /// A continuation at rest over a group: the things that wait.
        case listener
    }
    let id: String
    let kind: Kind
    let label: String
    let size: Int
    let depth: Int
    let chan: String?
    let chanFp: String?
    let group: [String]?
    let tree: Form
}

struct SceneFrame: Decodable {
    let tick: Int
    let quiescent: Bool
    let queued: Int
    let data: Int
    let waiting: Int
    let comms: Int
    let rhobots: [Card]
    let tuples: [Card]
    let listeners: [Card]

    /// Everything standing on the live plane. Depth carries guardedness, and
    /// these are exactly the things that can act.
    var livePlane: [Card] { tuples + listeners + rhobots }

    static let empty = SceneFrame(tick: 0, quiescent: true, queued: 0, data: 0,
                                  waiting: 0, comms: 0, rhobots: [], tuples: [],
                                  listeners: [])
}

/// One entry of the commit log. The scene animates these in log order.
struct Step: Decodable {
    let kind: String
    let chan: String?
    let cont: String?
    let datum: String?
    let term: String?
    let group: [String]?
    let spawned: Int?

    /// A communication: a send and a receipt met.
    var isComm: Bool { kind == "comm" }
}

struct Stats: Decodable {
    let round: Int
    let tick: Int
    let pacing: String
    let running: Bool
    let cores: Int
    let coresMax: Int
    /// Rounds admitted but not yet acknowledged: the buffer's fill.
    let inFlight: Int
    /// The bound. Lockstep's is one, by definition.
    let buffer: Int
    /// False means the evaluator is waiting for the scene — back-pressure,
    /// not a stall.
    let admissionOpen: Bool
    let commitsPerSec: Int
    let comms: Int
    let steps: Int
    let queued: Int
    let data: Int
    let waiting: Int
    let deploys: Int
    let quiescent: Bool
}

struct Round: Decodable {
    let round: Int
    let cores: Int
    let pacing: String
    let steps: [Step]
    let quiescent: Bool
    let inFlight: Int
    let buffer: Int
    let scene: SceneFrame
}

struct Diagnostic: Decodable {
    let lo: Int
    let hi: Int
    let code: String
    let message: String
}

/// Everything the executive can say.
enum Frame {
    case ready(session: String, resumed: Bool, coresMax: Int, wire: Int)
    case reply(seq: Int?, ok: Bool, result: Data?, diags: [Diagnostic])
    case round(Round)
    case scene(round: Int, scene: SceneFrame)
    case stats(Stats)
    case quiescent(round: Int)
    case error(code: String, message: String)
    case unknown(tag: String)
}

enum Wire {
    /// The host protocol this client is written against. `f1r3x-host` sends
    /// its own on every `ready`; a mismatch is the one thing worth checking
    /// before anything else, because the two repositories version separately.
    static let expectedVersion = 2

    static func decode(_ text: String) -> Frame? {
        guard let data = text.data(using: .utf8),
              let top = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let tag = top["t"] as? String
        else { return nil }
        let d = JSONDecoder()

        func sub<T: Decodable>(_ key: String, _ type: T.Type) -> T? {
            guard let v = top[key],
                  let raw = try? JSONSerialization.data(withJSONObject: v)
            else { return nil }
            return try? d.decode(type, from: raw)
        }

        switch tag {
        case "ready":
            return .ready(session: top["session"] as? String ?? "",
                          resumed: top["resumed"] as? Bool ?? false,
                          coresMax: top["coresMax"] as? Int ?? 1,
                          wire: top["wire"] as? Int ?? 0)
        case "round":
            // A round frame is a `Round` with a tag on it; the extra keys are
            // ignored by the decoder.
            guard let r = try? d.decode(Round.self, from: data) else { return .unknown(tag: tag) }
            return .round(r)
        case "scene":
            guard let s = sub("scene", SceneFrame.self) else { return .unknown(tag: tag) }
            return .scene(round: top["round"] as? Int ?? 0, scene: s)
        case "stats":
            guard let s = try? d.decode(Stats.self, from: data) else { return .unknown(tag: tag) }
            return .stats(s)
        case "quiescent":
            return .quiescent(round: top["round"] as? Int ?? 0)
        case "error":
            return .error(code: top["code"] as? String ?? "error",
                          message: top["message"] as? String ?? "")
        case "reply":
            let body = top["body"] as? [String: Any] ?? [:]
            let ok = body["ok"] as? Bool ?? false
            var result: Data?
            if let r = body["result"] {
                result = try? JSONSerialization.data(withJSONObject: r, options: .fragmentsAllowed)
            }
            var diags: [Diagnostic] = []
            if let raw = body["diags"],
               let dd = try? JSONSerialization.data(withJSONObject: raw) {
                diags = (try? d.decode([Diagnostic].self, from: dd)) ?? []
            }
            return .reply(seq: top["seq"] as? Int, ok: ok, result: result, diags: diags)
        default:
            return .unknown(tag: tag)
        }
    }
}

// MARK: - The client

/// A connection to a rho executive.
///
/// Frames arrive out of order with respect to requests: the driver pushes
/// rounds, scenes and stats on its own schedule, so a pushed frame routinely
/// arrives before the reply to the request that caused it. Everything is
/// therefore dispatched on its tag, and a reply is matched by the `#n`
/// correlation number rather than by position.
@MainActor
final class ExecutiveClient: NSObject {

    enum State: Equatable {
        case idle
        case connecting
        case connected(session: String, resumed: Bool)
        case failed(String)
    }

    private(set) var state: State = .idle
    var onFrame: ((Frame) -> Void)?
    var onState: ((State) -> Void)?

    private var task: URLSessionWebSocketTask?
    private var session: URLSession!
    private var seq = 0
    private var pending: [Int: (Bool, Data?, [Diagnostic]) -> Void] = [:]

    private var host = ""
    private var port = 9099
    private var sessionID = ""
    private var wantsConnection = false
    private var backoff: TimeInterval = 0.5

    override init() {
        super.init()
        session = URLSession(configuration: .default, delegate: nil, delegateQueue: .main)
    }

    /// The session id lives in the app's defaults, not in anything the server
    /// issued, so a resume works across a process the system has restarted.
    static var persistentSessionID: String {
        let key = "rhobots.sessionID"
        if let existing = UserDefaults.standard.string(forKey: key) { return existing }
        let fresh = "avp-" + UUID().uuidString.prefix(8).lowercased()
        UserDefaults.standard.set(fresh, forKey: key)
        return fresh
    }

    func connect(host: String, port: Int, sessionID: String) {
        self.host = host
        self.port = port
        self.sessionID = sessionID
        self.wantsConnection = true
        self.backoff = 0.5
        open()
    }

    func disconnect() {
        wantsConnection = false
        task?.cancel(with: .goingAway, reason: nil)
        task = nil
        set(.idle)
    }

    private func open() {
        guard wantsConnection else { return }
        guard var comps = URLComponents(string: "ws://\(host):\(port)") else {
            set(.failed("that address cannot be read as a host and port"))
            return
        }
        comps.path = "/s/\(sessionID)"
        guard let url = comps.url else {
            set(.failed("that address cannot be read as a host and port"))
            return
        }
        set(.connecting)
        let t = session.webSocketTask(with: url)
        task = t
        t.resume()
        receive()
    }

    private func set(_ s: State) {
        state = s
        onState?(s)
    }

    private func receive() {
        task?.receive { [weak self] result in
            Task { @MainActor in
                guard let self else { return }
                switch result {
                case .failure:
                    // Sleeping, backgrounding and roaming all land here. The
                    // world is still on the Mac; reconnecting resumes it.
                    self.scheduleReconnect()
                case .success(let message):
                    if case .string(let text) = message {
                        self.handle(text)
                    }
                    self.receive()
                }
            }
        }
    }

    private func handle(_ text: String) {
        guard let frame = Wire.decode(text) else { return }
        if case .ready(let s, let resumed, _, _) = frame {
            backoff = 0.5
            set(.connected(session: s, resumed: resumed))
        }
        if case .reply(let n, let ok, let result, let diags) = frame, let n {
            if let cont = pending.removeValue(forKey: n) { cont(ok, result, diags) }
        }
        onFrame?(frame)
    }

    private func scheduleReconnect() {
        task = nil
        guard wantsConnection else { return }
        set(.connecting)
        let wait = backoff
        backoff = min(backoff * 2, 8)
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: UInt64(wait * 1_000_000_000))
            self.open()
        }
    }

    // MARK: Requests

    /// Fire and forget. Used for acks and panel changes, where a reply would
    /// only be noise.
    func send(_ command: String) {
        task?.send(.string(command)) { _ in }
    }

    /// Ask, and hear back. The correlation number is what matches the reply,
    /// since a pushed frame may well arrive first.
    @discardableResult
    func request(_ command: String,
                 then: ((Bool, Data?, [Diagnostic]) -> Void)? = nil) -> Int {
        seq += 1
        let n = seq
        if let then { pending[n] = then }
        task?.send(.string("#\(n) \(command)")) { [weak self] error in
            guard error != nil else { return }
            Task { @MainActor in
                self?.pending.removeValue(forKey: n)?(false, nil, [
                    Diagnostic(lo: 0, hi: 0, code: "transport",
                               message: "the request never reached the executive")
                ])
            }
        }
        return n
    }

    // MARK: The vocabulary
    //
    // Everything below `deploy` is the executive's own command surface, which
    // the harness passes through unchanged; `play` and below are the harness
    // verbs. They are spelled out here so that a reader of this file can see
    // the whole protocol in one place.

    func deploy(_ source: String,
                then: @escaping (Bool, [Diagnostic]) -> Void) {
        request("deploy\n\(source)") { ok, _, diags in then(ok, diags) }
    }

    func setCores(_ n: Int, max: Int) { send("cores \(n) \(max)") }
    func play()                       { send("play") }
    func pause()                      { send("pause") }
    func step()                       { send("step") }
    func setPacing(lockstep: Bool)    { send("pacing \(lockstep ? "lockstep" : "free")") }
    func setBuffer(_ n: Int)          { send("buffer \(n)") }
    func setTreeDepth(_ n: Int)       { send("treedepth \(n)") }
    func resend()                     { send("resend") }
    func reset()                      { send("reset") }

    /// The scene has finished animating everything through this round. In
    /// lockstep this is the gate the scheduler waits on; in free-running it is
    /// the drain that keeps the buffer moving.
    func ack(_ round: Int) { send("ack \(round)") }
}
