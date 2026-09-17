//! Tests for the harness, and for nothing below it.
//!
//! The executive's own behaviour is tested in its own crates. What is tested
//! here is what the harness adds: the tree encoding the renderer reads, the
//! session lifecycle a headset forces, and the admission contract of Rhobots
//! §9.2 — which is the one piece of real logic in this crate and the one most
//! worth pinning down, because it is a distributed protocol and its failure
//! mode is a scene that quietly skips commits.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use crate::session::{Pacing, Registry};
use crate::tree::Budget;

use f1r3x::{Executive, Settings};
use k1ndl1ng_norm::{normalise, Options as NOpts};
use k1ndl1ng_parse::{parse, Options as POpts};

// ---------------------------------------------------------------------------
// The tree encoding

fn term(src: &str) -> k1ndl1ng_norm::Norm {
    let p = parse(src, &POpts::default());
    assert!(p.ok(), "parse failed for `{src}`");
    let n = normalise(&p.tree, &NOpts::default()).expect("normalise");
    let free: Vec<String> = n.free.iter().map(|f| f.ident.clone()).collect();
    n.term.ground_free_by(&free)
}

/// Balanced braces and brackets is a cheap total check that the explicit-stack
/// walk pushes its closers in the right order — exactly the mistake that kind
/// of walk invites.
fn balanced(s: &str) -> bool {
    let mut stack = Vec::new();
    let mut in_str = false;
    let mut esc = false;
    for c in s.chars() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' | '[' => stack.push(c),
            '}' => {
                if stack.pop() != Some('{') {
                    return false;
                }
            }
            ']' => {
                if stack.pop() != Some('[') {
                    return false;
                }
            }
            _ => {}
        }
    }
    stack.is_empty() && !in_str
}

#[test]
fn every_form_encodes_to_balanced_json() {
    for src in [
        "Nil",
        "@{Nil}!(Nil)",
        "for(y <- @{Nil}){ *y }",
        "for(y <- @{Nil}){ *y } | @{Nil}!(Nil)",
        "@{Nil}!(@{Nil}!(Nil))",
        "new x in { x!(Nil) }",
        "for(y <= @{Nil}){ Nil }",
        "for(y <<- @{Nil}){ Nil }",
        "for(a <- @{Nil} & b <- @{@{Nil}!(Nil)}){ *a | *b }",
        "for(y <- @{Nil}){ for(z <- y){ *z } } | @{Nil}!(@{Nil}!(Nil))",
    ] {
        let t = term(src);
        let j = tree::to_string(&t, Budget::default());
        assert!(balanced(&j), "unbalanced tree for `{src}`:\n{j}");
        assert!(j.starts_with("{\"f\":\""), "no form tag for `{src}`: {j}");
    }
}

#[test]
fn a_send_carries_its_channel_and_its_capsule() {
    let j = tree::to_string(&term("@{Nil}!(Nil)"), Budget::default());
    assert!(j.contains("\"f\":\"send\""), "{j}");
    assert!(j.contains("\"chan\":{\"g\":\"quote\""), "{j}");
    assert!(j.contains("\"args\":["), "{j}");
}

#[test]
fn a_receive_carries_its_binds_and_its_continuation() {
    let j = tree::to_string(&term("for(y <- @{Nil}){ *y }"), Budget::default());
    assert!(j.contains("\"f\":\"receive\""), "{j}");
    assert!(j.contains("\"kind\":\"linear\""), "{j}");
    assert!(j.contains("\"arrow\":\"<-\""), "{j}");
    assert!(j.contains("\"body\":"), "{j}");
    // `*y` is the platform with the unshrink icon, not a bare variable.
    assert!(j.contains("\"f\":\"drop\""), "{j}");
}

#[test]
fn the_three_bind_kinds_are_distinguished() {
    for (src, kind, arrow) in [
        ("for(y <- @{Nil}){ Nil }", "linear", "<-"),
        ("for(y <= @{Nil}){ Nil }", "persistent", "<="),
        ("for(y <<- @{Nil}){ Nil }", "peek", "<<-"),
    ] {
        let j = tree::to_string(&term(src), Budget::default());
        assert!(j.contains(&format!("\"kind\":\"{kind}\"")), "{src}: {j}");
        assert!(j.contains(&format!("\"arrow\":\"{arrow}\"")), "{src}: {j}");
    }
}

#[test]
fn a_quotation_budget_elides_and_keeps_the_fingerprint() {
    let t = term("@{Nil}!(@{Nil}!(@{Nil}!(@{Nil}!(Nil))))");
    let deep = tree::to_string(&t, Budget::with_quote_depth(10));
    let shallow = tree::to_string(&t, Budget::with_quote_depth(1));
    assert!(!deep.contains("\"elided\":true"), "nothing should elide deep");
    assert!(shallow.contains("\"elided\""), "should elide at depth 1");
    assert!(shallow.len() < deep.len());
    assert!(balanced(&shallow), "{shallow}");
    // The point of the elision glyph: the name stays comparable.
    assert!(
        shallow.matches("\"fp\":\"").count() > 0,
        "an elided node must still carry a fingerprint"
    );
}

#[test]
fn a_node_budget_bounds_a_wide_term() {
    let wide = term("@{Nil}!(Nil) | @{@{Nil}!(Nil)}!(Nil) | @{Nil}!(@{Nil}!(Nil)) | for(y <- @{Nil}){ *y }");
    let j = tree::to_string(
        &wide,
        Budget {
            quote_depth: 8,
            nodes: 3,
        },
    );
    assert!(balanced(&j), "{j}");
    assert!(j.contains("\"f\":\"elided\""), "{j}");
}

#[test]
fn congruent_terms_encode_to_the_same_tree() {
    // Order in a parallel composition carries no meaning and the normaliser
    // has already absorbed it; the tree inherits that, or the renderer would
    // redraw the world every time two robots swapped places.
    let a = tree::to_string(
        &term("@{Nil}!(Nil) | for(y <- @{Nil}){ *y }"),
        Budget::default(),
    );
    let b = tree::to_string(
        &term("for(y <- @{Nil}){ *y } | @{Nil}!(Nil)"),
        Budget::default(),
    );
    assert_eq!(a, b);
}

#[test]
fn matching_channels_give_matching_fingerprints() {
    // The visual witness that a communication is possible (§3.1).
    let s = tree::to_string(&term("@{Nil}!(Nil)"), Budget::default());
    let r = tree::to_string(&term("for(y <- @{Nil}){ Nil }"), Budget::default());
    let fp = |j: &str| {
        let i = j.find("\"chan\":{").expect("a chan");
        let k = j[i..].find("\"fp\":\"").expect("a fingerprint") + i + 6;
        j[k..k + 64].to_string()
    };
    assert_eq!(fp(&s), fp(&r));
}

// ---------------------------------------------------------------------------
// The scene frame

#[test]
fn a_scene_names_the_three_populations() {
    let mut x = Executive::new(Settings::default());
    x.deploy("for(y <- @{Nil}){ *y } | @{Nil}!(@{Nil}!(Nil))")
        .expect("deploy");
    let mut b = json::Buf::new();
    scene::write(&mut b, x.machine(), 0, Budget::default());
    let j = b.into_string();
    assert!(balanced(&j), "unbalanced scene:\n{j}");
    for key in ["\"rhobots\":[", "\"tuples\":[", "\"listeners\":[", "\"tick\":"] {
        assert!(j.contains(key), "scene missing {key}");
    }
    // Every card carries a tree, which is the whole reason this frame exists.
    assert_eq!(j.matches("\"tree\":").count(), j.matches("\"id\":").count());
}

#[test]
fn a_card_id_here_is_the_id_f1r3x_gives_it() {
    // The harness must not invent handles: a client reading `f1r3x`'s own
    // `scene` command and one reading this frame see the same objects.
    let mut x = Executive::new(Settings::default());
    x.deploy("for(y <- @{Nil}){ *y } | @{@{Nil}!(Nil)}!(Nil)")
        .expect("deploy");
    let _ = x.tick();
    let _ = x.tick();
    let theirs = f1r3x::json::scene(&x.scene());
    let mut b = json::Buf::new();
    scene::write(&mut b, x.machine(), x.tick_count(), Budget::default());
    let ours = b.into_string();
    let ids = ids_of(&theirs);
    assert!(!ids.is_empty(), "the reference scene had no cards");
    for id in ids {
        assert!(
            ours.contains(&format!("\"id\":\"{id}\"")),
            "card {id} is in f1r3x's scene but not in the harness frame"
        );
    }
}

fn ids_of(j: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = j;
    while let Some(i) = rest.find("\"id\":\"") {
        rest = &rest[i + 6..];
        if let Some(k) = rest.find('"') {
            out.push(rest[..k].to_string());
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Admission: the contract of §9.2

fn admission(pacing: Pacing, buffer: u64, in_flight: u64) -> bool {
    let reg = Registry::new(4, Duration::from_secs(60));
    let (sess, _) = reg.open("t");
    let mut c = sess.ctl.lock().unwrap();
    c.pacing = pacing;
    c.buffer = buffer;
    c.in_flight = in_flight;
    c.admission_open()
}

#[test]
fn lockstep_is_free_running_with_a_buffer_of_one() {
    assert!(admission(Pacing::Lockstep, 128, 0));
    assert!(!admission(Pacing::Lockstep, 128, 1));
    assert!(admission(Pacing::FreeRunning, 1, 0));
    assert!(!admission(Pacing::FreeRunning, 1, 1));
}

#[test]
fn free_running_admits_up_to_the_buffer_bound_and_no_further() {
    assert!(admission(Pacing::FreeRunning, 4, 3));
    assert!(!admission(Pacing::FreeRunning, 4, 4));
    assert!(!admission(Pacing::FreeRunning, 4, 9));
}

#[test]
fn an_ack_frees_exactly_the_rounds_it_names() {
    let reg = Registry::new(4, Duration::from_secs(60));
    let (s, _) = reg.open("t");
    {
        let mut c = s.ctl.lock().unwrap();
        c.pacing = Pacing::FreeRunning;
        c.buffer = 8;
        c.round = 5;
        c.in_flight = 5;
    }
    // "I have finished animating everything through round 3."
    s.ack(3);
    assert_eq!(s.ctl.lock().unwrap().in_flight, 2);
    s.ack(5);
    assert_eq!(s.ctl.lock().unwrap().in_flight, 0);
    // A duplicate ack after a reconnect cannot push it below zero.
    s.ack(3);
    assert_eq!(s.ctl.lock().unwrap().in_flight, 0);
}

#[test]
fn a_detached_session_is_not_advanced() {
    // A round nobody can see would never be acknowledged, so the driver must
    // not admit one. This is what makes a resume pick up where it stopped.
    let reg = Registry::new(4, Duration::from_secs(60));
    let (s, _) = reg.open("t");
    s.ctl.lock().unwrap().running = true;
    assert!(!s.attached());
}

// ---------------------------------------------------------------------------
// Sessions

#[test]
fn a_session_is_resumed_by_name_not_reissued() {
    let reg = Registry::new(4, Duration::from_secs(60));
    let (a, existed) = reg.open("headset-1");
    assert!(!existed);
    a.ctl.lock().unwrap().round = 17;
    let (b, existed) = reg.open("headset-1");
    assert!(existed);
    assert_eq!(b.ctl.lock().unwrap().round, 17);
    assert!(Arc::ptr_eq(&a, &b));
}

#[test]
fn a_detached_session_expires() {
    let reg = Registry::new(4, Duration::from_millis(1));
    let (_s, _) = reg.open("gone");
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(reg.reap(), vec!["gone".to_string()]);
    assert!(reg.is_empty());
}

#[test]
fn bye_destroys_the_session() {
    let reg = Arc::new(Registry::new(4, Duration::from_secs(60)));
    let (s, _) = reg.open("t");
    let mut sess = Some(s);
    let (_, after) = proto::handle(&reg, &mut sess, "bye");
    assert!(matches!(after, proto::After::Close));
    assert!(reg.get("t").is_none());
}

#[test]
fn commands_need_a_session_first() {
    let reg = Arc::new(Registry::new(4, Duration::from_secs(60)));
    let mut sess = None;
    let (reply, _) = proto::handle(&reg, &mut sess, "status");
    assert!(reply.contains("no-session"), "{reply}");
}

#[test]
fn an_unknown_command_reaches_the_executive_and_is_refused_there() {
    // The point is that it is refused by `f1r3x-ffi` in its own words rather
    // than swallowed here: the two surfaces stay one vocabulary.
    let reg = Arc::new(Registry::new(4, Duration::from_secs(60)));
    let (s, _) = reg.open("t");
    let mut sess = Some(s);
    let (reply, _) = proto::handle(&reg, &mut sess, "wobble");
    assert!(reply.contains("unknown-command"), "{reply}");
}

#[test]
fn a_correlation_number_comes_back_on_the_reply() {
    let reg = Arc::new(Registry::new(4, Duration::from_secs(60)));
    let (s, _) = reg.open("t");
    let mut sess = Some(s);
    let (reply, _) = proto::handle(&reg, &mut sess, "#42 status");
    assert!(reply.contains("\"seq\":42"), "{reply}");
    let (reply, _) = proto::handle(&reg, &mut sess, "status");
    assert!(reply.contains("\"seq\":null"), "{reply}");
}

#[test]
fn a_deploy_with_a_payload_survives_the_seq_prefix() {
    let reg = Arc::new(Registry::new(4, Duration::from_secs(60)));
    let (s, _) = reg.open("t");
    let mut sess = Some(s.clone());
    let (reply, _) = proto::handle(&reg, &mut sess, "#7 deploy\n@{Nil}!(Nil)");
    assert!(reply.contains("\"seq\":7"), "{reply}");
    assert!(reply.contains("\"ok\":true"), "{reply}");
    assert_eq!(s.exec.lock().unwrap().deploys().len(), 1);
}

#[test]
fn reset_keeps_the_session_and_clears_the_world() {
    let reg = Arc::new(Registry::new(4, Duration::from_secs(60)));
    let (s, _) = reg.open("t");
    let mut sess = Some(s.clone());
    proto::handle(&reg, &mut sess, "deploy\n@{Nil}!(Nil)");
    assert_eq!(s.exec.lock().unwrap().deploys().len(), 1);
    proto::handle(&reg, &mut sess, "reset");
    assert_eq!(s.exec.lock().unwrap().deploys().len(), 0);
    assert!(reg.get("t").is_some());
}

#[test]
fn the_panel_verbs_move_the_control_state() {
    let reg = Arc::new(Registry::new(4, Duration::from_secs(60)));
    let (s, _) = reg.open("t");
    let mut sess = Some(s.clone());
    proto::handle(&reg, &mut sess, "pacing free");
    assert_eq!(s.ctl.lock().unwrap().pacing, Pacing::FreeRunning);
    proto::handle(&reg, &mut sess, "buffer 32");
    assert_eq!(s.ctl.lock().unwrap().buffer, 32);
    proto::handle(&reg, &mut sess, "play");
    assert!(s.ctl.lock().unwrap().running);
    proto::handle(&reg, &mut sess, "pause");
    assert!(!s.ctl.lock().unwrap().running);
    proto::handle(&reg, &mut sess, "step");
    assert_eq!(s.ctl.lock().unwrap().steps_owed, 1);
    proto::handle(&reg, &mut sess, "treedepth 2");
    assert_eq!(s.ctl.lock().unwrap().quote_depth, 2);
    // Out-of-range values are refused rather than clamped silently.
    let (reply, _) = proto::handle(&reg, &mut sess, "buffer 0");
    assert!(reply.contains("bad-argument"), "{reply}");
    assert_eq!(s.ctl.lock().unwrap().buffer, 32);
}

// ---------------------------------------------------------------------------
// WebSocket

#[test]
fn the_handshake_accept_value_is_the_one_rfc_6455_gives() {
    // The worked example from RFC 6455 §1.3.
    let key = "dGhlIHNhbXBsZSBub25jZQ==";
    let d = ws::sha1(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes());
    assert_eq!(ws::b64(&d), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
}

#[test]
fn sha1_and_base64_agree_with_their_published_vectors() {
    let hex = |d: [u8; 20]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(
        hex(ws::sha1(b"abc")),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
    assert_eq!(
        hex(ws::sha1(b"")),
        "da39a3ee5e6b4b0d3255bfef95601890afd80709"
    );
    assert_eq!(ws::b64(b"f"), "Zg==");
    assert_eq!(ws::b64(b"fo"), "Zm8=");
    assert_eq!(ws::b64(b"foo"), "Zm9v");
    assert_eq!(ws::b64(b"foobar"), "Zm9vYmFy");
}

// ---------------------------------------------------------------------------
// End to end, over a real socket

/// A client just complete enough to drive the server: handshake, masked text
/// frames out, frames in.
struct Client {
    sock: TcpStream,
    /// Frames read while looking for a different one. The server pushes
    /// rounds, scenes and stats on its own schedule, so a pushed frame
    /// routinely arrives before the reply to the request that caused it. A
    /// client that discarded those would lose them; the real Swift client
    /// dispatches on the tag for the same reason.
    pending: Vec<String>,
}

impl Client {
    fn connect(addr: std::net::SocketAddr, session: &str) -> Client {
        let mut sock = TcpStream::connect(addr).expect("connect");
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(
            sock,
            "GET /s/{session} HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Version: 13\r\n\r\n"
        )
        .unwrap();
        let mut head = Vec::new();
        let mut b = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            assert_eq!(sock.read(&mut b).unwrap(), 1, "handshake truncated");
            head.push(b[0]);
        }
        let head = String::from_utf8_lossy(&head).to_string();
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        assert!(head.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="), "{head}");
        Client {
            sock,
            pending: Vec::new(),
        }
    }

    fn send(&mut self, s: &str) {
        let payload = s.as_bytes();
        let mut f = vec![0x81];
        let mask = [0x37u8, 0xfa, 0x21, 0x3d];
        let n = payload.len();
        if n < 126 {
            f.push(0x80 | n as u8);
        } else {
            f.push(0x80 | 126);
            f.extend_from_slice(&(n as u16).to_be_bytes());
        }
        f.extend_from_slice(&mask);
        for (i, b) in payload.iter().enumerate() {
            f.push(b ^ mask[i & 3]);
        }
        self.sock.write_all(&f).unwrap();
    }

    /// `None` on a read timeout, which several tests rely on to show that
    /// nothing arrived.
    fn try_recv(&mut self) -> Option<String> {
        let mut h = [0u8; 2];
        if self.sock.read_exact(&mut h).is_err() {
            return None;
        }
        assert_eq!(h[0] & 0x0F, 0x1, "expected a text frame");
        assert_eq!(h[1] & 0x80, 0, "a server frame must not be masked");
        let mut len = (h[1] & 0x7F) as usize;
        if len == 126 {
            let mut e = [0u8; 2];
            self.sock.read_exact(&mut e).ok()?;
            len = u16::from_be_bytes(e) as usize;
        } else if len == 127 {
            let mut e = [0u8; 8];
            self.sock.read_exact(&mut e).ok()?;
            len = u64::from_be_bytes(e) as usize;
        }
        let mut p = vec![0u8; len];
        self.sock.read_exact(&mut p).ok()?;
        Some(String::from_utf8(p).unwrap())
    }

    /// Take an already-read frame carrying `tag`, if there is one.
    fn take_pending(&mut self, needle: &str) -> Option<String> {
        let i = self.pending.iter().position(|f| f.contains(needle))?;
        Some(self.pending.remove(i))
    }

    /// Read frames until one carries `tag`, or time runs out. Frames that do
    /// not match are kept, not dropped.
    fn until(&mut self, tag: &str) -> String {
        let needle = format!("\"t\":\"{tag}\"");
        if let Some(f) = self.take_pending(&needle) {
            return f;
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match self.try_recv() {
                Some(f) if f.contains(&needle) => return f,
                Some(f) => self.pending.push(f),
                None => break,
            }
        }
        panic!("no `{tag}` frame arrived");
    }

    /// True if a frame carrying `tag` has arrived, or arrives within `window`.
    fn sees(&mut self, tag: &str, window: Duration) -> bool {
        let needle = format!("\"t\":\"{tag}\"");
        if self.take_pending(&needle).is_some() {
            return true;
        }
        self.sock.set_read_timeout(Some(window)).unwrap();
        let deadline = Instant::now() + window;
        let mut seen = false;
        while Instant::now() < deadline {
            match self.try_recv() {
                Some(f) if f.contains(&needle) => {
                    seen = true;
                    break;
                }
                Some(f) => self.pending.push(f),
                None => break,
            }
        }
        self.sock
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        seen
    }
}

fn server() -> (Arc<Server>, std::net::SocketAddr) {
    let cfg = Config {
        addr: "127.0.0.1:0".to_string(),
        cores_max: 4,
        session_ttl: Duration::from_secs(60),
        verbose: false,
    };
    let s = Arc::new(Server::bind(&cfg).expect("bind"));
    let addr = s.local_addr().unwrap();
    let t = s.clone();
    std::thread::spawn(move || {
        let _ = t.serve();
    });
    (s, addr)
}

/// A term with several rounds in it: one comm, then the released continuation.
const RELAY: &str = "deploy\nfor(y <- @{Nil}){ *y } | @{Nil}!(@{Nil}!(Nil))";

#[test]
fn a_client_attaches_and_is_told_what_it_attached_to() {
    let (srv, addr) = server();
    let mut c = Client::connect(addr, "e2e-hello");
    let ready = c.until("ready");
    assert!(ready.contains("\"session\":\"e2e-hello\""), "{ready}");
    assert!(ready.contains("\"resumed\":false"), "{ready}");
    assert!(ready.contains("\"coresMax\":4"), "{ready}");
    // The world is drawn before anything is asked for.
    assert!(c.until("scene").contains("\"rhobots\":["));
    srv.shutdown();
}

#[test]
fn lockstep_withholds_the_next_round_until_the_scene_acknowledges() {
    // The heart of §9.2, and the reason this crate exists.
    let (srv, addr) = server();
    let mut c = Client::connect(addr, "e2e-lockstep");
    c.until("ready");
    c.send(RELAY);
    assert!(c.until("reply").contains("\"ok\":true"));
    c.send("pacing lockstep");
    c.until("reply");
    c.send("play");
    c.until("reply");

    let first = c.until("round");
    assert!(first.contains("\"round\":1"), "{first}");
    assert!(first.contains("\"pacing\":\"lockstep\""), "{first}");
    assert!(first.contains("\"inFlight\":1"), "{first}");
    assert!(first.contains("\"scene\":{"), "a round carries the scene");

    // Without an ack no second round may be admitted. Watch for long enough
    // that a driver ignoring admission would certainly have produced one.
    assert!(
        !c.sees("round", Duration::from_millis(700)),
        "a second round was admitted without an acknowledgment"
    );

    // Acknowledge, and it comes.
    c.send("ack 1");
    assert!(c.until("round").contains("\"round\":2"));
    srv.shutdown();
}

#[test]
fn free_running_runs_ahead_but_only_as_far_as_the_buffer() {
    let (srv, addr) = server();
    let mut c = Client::connect(addr, "e2e-free");
    c.until("ready");
    c.send(RELAY);
    c.until("reply");
    c.send("pacing free");
    c.until("reply");
    c.send("buffer 2");
    c.until("reply");
    c.send("play");
    c.until("reply");

    // Two rounds arrive with no acknowledgment at all, which lockstep would
    // not have allowed.
    assert!(c.until("round").contains("\"round\":1"));
    let r2 = c.until("round");
    assert!(r2.contains("\"round\":2"), "{r2}");
    assert!(r2.contains("\"inFlight\":2"), "{r2}");

    // And then back-pressure: the buffer is full, so admission pauses.
    assert!(
        !c.sees("round", Duration::from_millis(700)),
        "the buffer bound was not respected"
    );
    srv.shutdown();
}

#[test]
fn a_paused_session_admits_nothing() {
    let (srv, addr) = server();
    let mut c = Client::connect(addr, "e2e-pause");
    c.until("ready");
    c.send(RELAY);
    c.until("reply");
    assert!(
        !c.sees("round", Duration::from_millis(500)),
        "a round was admitted without `play`"
    );
    // A single step admits exactly one round, in either mode.
    c.send("step");
    c.until("reply");
    assert!(c.until("round").contains("\"round\":1"));
    c.send("ack 1");
    c.until("reply");
    assert!(
        !c.sees("round", Duration::from_millis(500)),
        "`step` admitted more than one round"
    );
    srv.shutdown();
}

#[test]
fn a_reconnect_resumes_the_same_world() {
    // The lifecycle a headset actually has: the socket drops, and the world
    // must still be there.
    let (srv, addr) = server();
    let mut c = Client::connect(addr, "e2e-resume");
    c.until("ready");
    c.send("deploy\n@{Nil}!(@{Nil}!(Nil))");
    c.until("reply");
    c.send("step");
    c.until("reply");
    c.until("round");
    drop(c);

    std::thread::sleep(Duration::from_millis(150));
    let mut c2 = Client::connect(addr, "e2e-resume");
    let ready = c2.until("ready");
    assert!(ready.contains("\"resumed\":true"), "{ready}");
    c2.send("status");
    assert!(c2.until("reply").contains("\"deploys\":1"));
    srv.shutdown();
}

#[test]
fn a_second_connection_evicts_the_first_and_says_so() {
    let (srv, addr) = server();
    let mut a = Client::connect(addr, "e2e-evict");
    a.until("ready");
    let mut b = Client::connect(addr, "e2e-evict");
    b.until("ready");
    assert!(
        a.sees("error", Duration::from_secs(2)),
        "the evicted client was not told"
    );
    srv.shutdown();
}

#[test]
fn a_bad_term_comes_back_as_diagnostics_not_a_dropped_socket() {
    let (srv, addr) = server();
    let mut c = Client::connect(addr, "e2e-bad");
    c.until("ready");
    c.send("deploy\nfor(y <- ){ *y");
    let reply = c.until("reply");
    assert!(reply.contains("\"ok\":false"), "{reply}");
    assert!(reply.contains("\"diags\":["), "{reply}");
    // And the connection is still usable afterwards.
    c.send("ping");
    assert!(c.until("reply").contains("pong"));
    srv.shutdown();
}

#[test]
fn the_cores_knob_does_not_change_what_happens() {
    // The property the demo rests on, stated over the wire: a person can slow
    // the world to one step and see exactly the run they watched at speed.
    let run_at = |cores: u32, session: &str| -> String {
        let (srv, addr) = server();
        let mut c = Client::connect(addr, session);
        c.until("ready");
        c.send(&format!("cores {cores} 8"));
        c.until("reply");
        c.send(RELAY);
        c.until("reply");
        c.send("pacing free");
        c.until("reply");
        c.send("buffer 64");
        c.until("reply");
        c.send("play");
        c.until("reply");
        let last;
        loop {
            let f = c.until("round");
            let round: u64 = f
                .split("\"round\":")
                .nth(1)
                .and_then(|s| s.split(',').next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            c.send(&format!("ack {round}"));
            if f.contains("\"quiescent\":true") {
                last = f;
                break;
            }
        }
        srv.shutdown();
        // The final scene, less the tick counter: how many ticks the run took
        // is exactly what the knob is allowed to change.
        let scene = last.split("\"scene\":").nth(1).unwrap_or("").to_string();
        let tail = scene
            .split_once(",\"quiescent\"")
            .map(|(_, t)| t.to_string())
            .unwrap_or(scene);
        tail
    };
    assert_eq!(run_at(1, "e2e-c1"), run_at(4, "e2e-c4"));
}
