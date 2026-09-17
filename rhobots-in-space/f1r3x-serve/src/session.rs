//! Sessions, and the lifecycle a headset actually has.
//!
//! A Vision Pro is not a well-behaved client. It is taken off, it sleeps, the
//! app is backgrounded while a person looks at something else, and the Wi-Fi
//! roams between access points. Every one of those drops the socket. If a
//! dropped socket destroyed the executive, a person would lose a running world
//! by putting the headset down for a minute, so the executive outlives the
//! connection:
//!
//! ```text
//!   attach <id>        socket opens, session created or resumed
//!   … commands, rounds …
//!   socket closes      session detaches: the driver stops, state stays
//!   attach <id>        same session, same world, same tick count
//!   bye | TTL expires  session destroyed
//! ```
//!
//! Sessions are named by the client, which is what lets a resume work across
//! a process the system has restarted: the id lives in the app's defaults, not
//! in a cookie this server issued. An unknown id is not an error — it creates
//! the session — so a first run and a resume are the same call.
//!
//! One rule keeps this honest: **a session has at most one connection**. If a
//! second socket attaches to a live session, the first is evicted rather than
//! both being served. Two headsets driving one executive would interleave
//! their acknowledgments and neither would see the pacing it asked for.

use std::collections::BTreeMap;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use f1r3x::{Executive, Mode, Settings};

use crate::json;
use crate::ws;

/// How the scene and the evaluator are paced against each other
/// (Rhobots §9.2).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Pacing {
    /// Admit a round of at most `cores` commits, then withhold admission of
    /// the next until the scene acknowledges that every animation in the round
    /// has finished.
    Lockstep,
    /// Admit without waiting. The rounds feed a bounded buffer that the scene
    /// drains; when it is full, admission pauses until an entry is drained, so
    /// the evaluator never runs further ahead of the scene than the bound and
    /// every commit is animated.
    FreeRunning,
}

impl Pacing {
    pub fn name(self) -> &'static str {
        match self {
            Pacing::Lockstep => "lockstep",
            Pacing::FreeRunning => "free-running",
        }
    }
}

/// What the control panel sets, held apart from the executive so the driver
/// can read it without taking the executive's lock.
pub struct Control {
    pub pacing: Pacing,
    /// Rounds admitted but not yet acknowledged by the scene.
    pub in_flight: u64,
    /// The playback buffer bound (§9.2), in rounds. Lockstep ignores it: its
    /// bound is one, by definition.
    pub buffer: u64,
    /// Whether the driver should be advancing at all — the panel's run/pause.
    pub running: bool,
    /// Rounds still owed after a `step`, for single-stepping in either mode.
    pub steps_owed: u64,
    /// Quotation depth the client can still draw; past it the tree elides.
    pub quote_depth: u32,
    /// Rounds admitted since the session began.
    pub round: u64,
    /// For the throughput readout: comms and when they were counted.
    pub comms_mark: u64,
    pub comms_at: Instant,
    pub rate: f64,
}

impl Control {
    fn new() -> Control {
        Control {
            pacing: Pacing::Lockstep,
            in_flight: 0,
            buffer: 128,
            running: false,
            steps_owed: 0,
            quote_depth: 4,
            round: 0,
            comms_mark: 0,
            comms_at: Instant::now(),
            rate: 0.0,
        }
    }

    /// Whether the scheduler may admit another round. Lockstep is
    /// free-running with a bound of one; writing it that way rather than as
    /// two code paths is what keeps the two modes honestly comparable.
    pub fn admission_open(&self) -> bool {
        let bound = match self.pacing {
            Pacing::Lockstep => 1,
            Pacing::FreeRunning => self.buffer.max(1),
        };
        self.in_flight < bound
    }

    pub fn wants_to_advance(&self) -> bool {
        (self.running || self.steps_owed > 0) && self.admission_open()
    }
}

/// The socket a session's frames go out on, if it has one.
pub struct Sink {
    pub sock: TcpStream,
    /// Bumped when a connection is evicted, so the old reader knows to stop.
    pub epoch: u64,
}

pub struct Session {
    pub id: String,
    pub exec: Mutex<Executive>,
    pub ctl: Mutex<Control>,
    /// Woken when the control state changes, when a round is acknowledged, or
    /// when the session is closing.
    pub wake: Condvar,
    /// `None` while detached. The driver will not advance without it: a round
    /// nobody can see is a round that would never be acknowledged.
    pub out: Mutex<Option<Sink>>,
    pub closing: AtomicBool,
    pub detached_since: Mutex<Option<Instant>>,
    pub epoch: AtomicU64,
    /// Cores visible to this process, which is what the panel's slider is
    /// bounded by (§9.3).
    pub cores_max: u32,
}

impl Session {
    fn new(id: &str, cores_max: u32) -> Session {
        Session {
            id: id.to_string(),
            exec: Mutex::new(Executive::new(Settings {
                // The harness owns pacing, so the executive is always asked
                // for one round at a time; `cores` is the round's width.
                mode: Mode::Lockstep,
                ..Settings::default()
            })),
            ctl: Mutex::new(Control::new()),
            wake: Condvar::new(),
            out: Mutex::new(None),
            closing: AtomicBool::new(false),
            detached_since: Mutex::new(Some(Instant::now())),
            epoch: AtomicU64::new(0),
            cores_max,
        }
    }

    /// Send one frame, if anyone is listening. A write failure is not an
    /// error here: it means the socket has gone, and the reader thread will
    /// notice and detach.
    pub fn emit(&self, frame: &str) {
        let mut guard = self.out.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(sink) = guard.as_mut() {
            if ws::write_text(&mut sink.sock, frame).is_err() {
                *guard = None;
                *self.detached_since.lock().unwrap_or_else(|p| p.into_inner()) =
                    Some(Instant::now());
            }
        }
    }

    pub fn attached(&self) -> bool {
        self.out
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
    }

    /// Take the socket over for a new connection, evicting any previous one.
    pub fn attach(&self, sock: TcpStream) -> u64 {
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let mut guard = self.out.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(old) = guard.take() {
            // Tell the evicted client why, then let its reader fail.
            let mut old = old;
            let _ = ws::write_text(
                &mut old.sock,
                &json::error_frame(
                    "evicted",
                    "another connection attached to this session; \
                     a session serves one client at a time",
                ),
            );
            let _ = ws::write_close(&mut old.sock);
            let _ = old.sock.shutdown(std::net::Shutdown::Both);
        }
        *guard = Some(Sink { sock, epoch });
        *self.detached_since.lock().unwrap_or_else(|p| p.into_inner()) = None;
        drop(guard);
        self.wake.notify_all();
        epoch
    }

    /// Give up the socket, if this connection still owns it. The executive
    /// stays exactly as it is.
    pub fn detach(&self, epoch: u64) {
        let mut guard = self.out.lock().unwrap_or_else(|p| p.into_inner());
        if guard.as_ref().map(|s| s.epoch) == Some(epoch) {
            *guard = None;
            *self.detached_since.lock().unwrap_or_else(|p| p.into_inner()) =
                Some(Instant::now());
        }
        drop(guard);
        self.wake.notify_all();
    }

    /// Mark a round acknowledged by the scene, releasing admission.
    pub fn ack(&self, upto: u64) {
        let mut c = self.ctl.lock().unwrap_or_else(|p| p.into_inner());
        // An ack names the highest round the scene has finished animating, so
        // a client that batches acks, or whose ack for round n-1 was lost in
        // a reconnect, still frees the right number of slots.
        let outstanding = c.round.saturating_sub(upto);
        c.in_flight = c.in_flight.min(outstanding);
        drop(c);
        self.wake.notify_all();
    }

    pub fn close(&self) {
        self.closing.store(true, Ordering::SeqCst);
        self.wake.notify_all();
        let mut guard = self.out.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(sink) = guard.as_mut() {
            let _ = ws::write_close(&mut sink.sock);
            let _ = sink.sock.shutdown(std::net::Shutdown::Both);
        }
        *guard = None;
    }
}

/// Every live session, by id.
pub struct Registry {
    sessions: Mutex<BTreeMap<String, Arc<Session>>>,
    cores_max: u32,
    ttl: Duration,
}

impl Registry {
    pub fn new(cores_max: u32, ttl: Duration) -> Registry {
        Registry {
            sessions: Mutex::new(BTreeMap::new()),
            cores_max,
            ttl,
        }
    }

    /// Find a session or make one. Returns the session and whether it already
    /// existed, so the client can be told it resumed rather than started.
    pub fn open(&self, id: &str) -> (Arc<Session>, bool) {
        let mut map = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(s) = map.get(id) {
            return (s.clone(), true);
        }
        let s = Arc::new(Session::new(id, self.cores_max));
        map.insert(id.to_string(), s.clone());
        (s, false)
    }

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
    }

    pub fn remove(&self, id: &str) -> Option<Arc<Session>> {
        let s = self
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
        if let Some(s) = &s {
            s.close();
        }
        s
    }

    pub fn ids(&self) -> Vec<String> {
        self.sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drop sessions that have been detached longer than the TTL. A headset
    /// that is put down for five minutes keeps its world; one abandoned for an
    /// hour does not hold memory for the rest of the day.
    pub fn reap(&self) -> Vec<String> {
        let now = Instant::now();
        let mut dead = Vec::new();
        {
            let map = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
            for (id, s) in map.iter() {
                let since = *s
                    .detached_since
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                if let Some(t) = since {
                    if now.duration_since(t) > self.ttl {
                        dead.push(id.clone());
                    }
                }
            }
        }
        for id in &dead {
            self.remove(id);
        }
        dead
    }

    pub fn close_all(&self) {
        let map = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        for s in map.values() {
            s.close();
        }
    }
}
