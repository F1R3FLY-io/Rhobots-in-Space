//! `f1r3x-serve` — the communication harness.
//!
//! A rho executive runs on a Mac; the Rhobots client runs on a Vision Pro;
//! this is the wire between them and nothing else. It contains no parser, no
//! normaliser, no store, no interpreter and no scheduling policy — all of that
//! is `f1r3x` and the crates under it, unchanged. What is here is a
//! WebSocket, a session that outlives a connection, and the admission control
//! that Rhobots §9.2 specifies, which in process was a semaphore and over a
//! socket has to be an acknowledgment.
//!
//! ```text
//!   Vision Pro                    Mac
//!   ┌──────────────┐   WebSocket  ┌──────────────────────────────┐
//!   │ RealityKit   │◀────────────▶│ f1r3x-serve                  │
//!   │ scene, panel │   rounds,    │  session · driver · tree     │
//!   └──────────────┘   acks       │        ↓                     │
//!                                 │  f1r3x (executive)           │
//!                                 │  b3ll0ws · campf1r3 · k1ndl1ng│
//!                                 └──────────────────────────────┘
//! ```
//!
//! The spec has version 1 linking the evaluator into the headset behind a C
//! ABI, to avoid the device-boundary transport failures met during the
//! F1R3Skein bring-up. This is the other arrangement, and the reason to want
//! it is development rather than deployment: a Mac rebuilds and restarts the
//! executive in seconds without a device build, and `f1r3x-ffi` remains
//! exactly as it is for the day the executive moves on-device. The two agree
//! by construction, because both speak the same request vocabulary —
//! [`f1r3x_ffi::dispatch`] is what answers everything except the handful of
//! verbs a socket needs.

#![forbid(unsafe_code)]

pub mod driver;
pub mod json;
pub mod proto;
pub mod scene;
pub mod session;
pub mod tree;
pub mod ws;

use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use session::{Registry, Session};

/// Bumped when a client must notice a change to the *harness* protocol. The
/// executive's own protocol version is reported separately, by its `status`.
pub const WIRE_VERSION: u32 = 1;

pub struct Config {
    pub addr: String,
    /// Upper bound of the panel's cores slider (§9.3): the parallelism visible
    /// to this process, which may be fewer than the machine's hardware cores.
    pub cores_max: u32,
    /// How long a detached session keeps its world.
    pub session_ttl: Duration,
    /// Log every request and frame.
    pub verbose: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            addr: "0.0.0.0:9099".to_string(),
            cores_max: available_parallelism(),
            session_ttl: Duration::from_secs(900),
            verbose: false,
        }
    }
}

pub fn available_parallelism() -> u32 {
    thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1)
        .max(1)
}

pub struct Server {
    pub reg: Arc<Registry>,
    pub stop: Arc<AtomicBool>,
    listener: TcpListener,
    verbose: bool,
}

impl Server {
    pub fn bind(cfg: &Config) -> io::Result<Server> {
        let listener = TcpListener::bind(&cfg.addr)?;
        Ok(Server {
            reg: Arc::new(Registry::new(cfg.cores_max, cfg.session_ttl)),
            stop: Arc::new(AtomicBool::new(false)),
            listener,
            verbose: cfg.verbose,
        })
    }

    pub fn local_addr(&self) -> io::Result<std::net::SocketAddr> {
        self.listener.local_addr()
    }

    /// Accept until stopped. The reaper runs alongside.
    pub fn serve(&self) -> io::Result<()> {
        let reg = self.reg.clone();
        let stop = self.stop.clone();
        let verbose = self.verbose;
        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_secs(15));
                for id in reg.reap() {
                    if verbose {
                        eprintln!("[reap] session {id} expired");
                    }
                }
            }
        });

        for incoming in self.listener.incoming() {
            if self.stop.load(Ordering::SeqCst) {
                break;
            }
            let sock = match incoming {
                Ok(s) => s,
                Err(e) => {
                    if self.verbose {
                        eprintln!("[accept] {e}");
                    }
                    continue;
                }
            };
            let reg = self.reg.clone();
            let verbose = self.verbose;
            thread::spawn(move || {
                if let Err(e) = connection(reg, sock, verbose) {
                    if verbose {
                        eprintln!("[conn] {e}");
                    }
                }
            });
        }
        Ok(())
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.reg.close_all();
        // Unblock `incoming` by connecting to ourselves.
        if let Ok(addr) = self.listener.local_addr() {
            let _ = TcpStream::connect(addr);
        }
    }
}

/// One connection: handshake, attach, then read requests until it closes.
fn connection(reg: Arc<Registry>, mut sock: TcpStream, verbose: bool) -> io::Result<()> {
    sock.set_nodelay(true)?;
    let target = ws::accept(&mut sock)?;
    if verbose {
        eprintln!("[conn] upgraded, target {target}");
    }

    // The session id is the request path: `/s/<id>`. Putting it in the URL
    // rather than in a first message means the socket is bound to a session
    // before any request is read, so a resumed session can be sent its scene
    // immediately and a client that only listens never has to speak first.
    let id = target
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty() && *s != "s")
        .unwrap_or("default")
        .to_string();
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        || id.len() > 128
    {
        let _ = ws::write_text(
            &mut sock,
            &proto::push_error("bad-session", "the session id in the path is not acceptable"),
        );
        let _ = ws::write_close(&mut sock);
        return Ok(());
    }

    let (s, resumed) = reg.open(&id);
    let write_half = sock.try_clone()?;
    let epoch = s.attach(write_half);

    // One driver per session, started on first attach. A resumed session
    // already has one; it was parked waiting for a socket.
    if !resumed {
        let ds = s.clone();
        thread::spawn(move || driver::run(ds));
    }

    s.emit(&json::frame("ready", |b| {
        b.comma();
        b.field_num("wire", WIRE_VERSION as u64);
        b.comma();
        b.field_str("session", &s.id);
        b.comma();
        b.field_bool("resumed", resumed);
        b.comma();
        b.field_num("coresMax", s.cores_max as u64);
    }));
    // A resumed client draws the world it left before it is told anything
    // else; a fresh one draws an empty plane, which is also correct.
    driver::emit_scene(&s);
    driver::emit_stats(&s);

    let mut session = Some(s.clone());
    let result = read_loop(&reg, &mut session, &mut sock, epoch, verbose);
    s.detach(epoch);
    if verbose {
        eprintln!("[conn] session {id} detached");
    }
    result
}

fn read_loop(
    reg: &Arc<Registry>,
    session: &mut Option<Arc<Session>>,
    sock: &mut TcpStream,
    epoch: u64,
    verbose: bool,
) -> io::Result<()> {
    loop {
        // If this connection has been evicted by a later one, stop reading.
        if let Some(s) = session.as_ref() {
            if s.epoch.load(Ordering::SeqCst) != epoch || s.closing.load(Ordering::SeqCst) {
                return Ok(());
            }
        }
        let msg = match ws::read(sock) {
            Ok(Some(m)) => m,
            Ok(None) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        match msg {
            ws::Msg::Close => return Ok(()),
            ws::Msg::Ping(p) => {
                if let Some(s) = session.as_ref() {
                    let mut g = s.out.lock().unwrap_or_else(|x| x.into_inner());
                    if let Some(sink) = g.as_mut() {
                        let _ = ws::write(&mut sink.sock, 0xA, &p);
                    }
                }
            }
            ws::Msg::Pong(_) => {}
            ws::Msg::Binary(_) => {
                if let Some(s) = session.as_ref() {
                    s.emit(&proto::push_error(
                        "binary",
                        "requests are text frames; this protocol has no binary form",
                    ));
                }
            }
            ws::Msg::Text(req) => {
                if verbose {
                    let head = req.lines().next().unwrap_or("");
                    eprintln!("[req] {head}");
                }
                let (reply, after) = proto::handle(reg, session, &req);
                if let Some(s) = session.as_ref() {
                    s.emit(&reply);
                }
                if matches!(after, proto::After::Close) {
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
