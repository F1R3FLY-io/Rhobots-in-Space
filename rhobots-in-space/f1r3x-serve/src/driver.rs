//! The driver: one thread per session, turning ticks into rounds.
//!
//! This is the whole of what the harness adds to the executive. `f1r3x`
//! already decides *what* reduces — the queue is FIFO over encoding-ordered
//! components, the store is in its deterministic profile, and neither the
//! cores knob nor the pacing mode changes the order of reduction. What was
//! missing for a networked client is *when* a decided commit is released to
//! the scene, which Rhobots §9.2 makes the programmer's choice:
//!
//! - **Lockstep.** Admit a round of at most `cores` commits, then withhold
//!   admission of the next until the scene acknowledges that every animation
//!   in the round has finished.
//! - **Free-running.** Admit without waiting. Rounds feed a bounded buffer
//!   that the scene drains; when it is full, admission pauses until an entry
//!   is drained, so the evaluator never runs further ahead of the scene than
//!   the bound, and *every commit is animated*.
//!
//! The two are one rule with different bounds — lockstep is free-running with
//! a buffer of one — and [`Control::admission_open`] is where it lives. In
//! process this was a semaphore; over a socket it is an acknowledgment, which
//! is the only real change the network forces on the design.
//!
//! One consequence is worth stating plainly, because it is a departure from
//! the spec's §10 and is forced by the user's arrangement rather than chosen.
//! The spec has worker threads racing on the device's physical cores, so that
//! races are settled by the hardware. This executive is a single-threaded
//! deterministic machine, and `cores` is the width of an admitted round, not a
//! count of racing threads. The slider still does what §9.3 says it teaches —
//! up to `cores` communications are in progress at once, the number of
//! converging pairs is a picture of concurrency, and channel contention still
//! plateaus the throughput readout — but the plateau is a property of the
//! term, not of a lock. When the store grows a threaded profile, this is the
//! one function that has to change.
//!
//! [`Control::admission_open`]: crate::session::Control::admission_open

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::json::{frame, Buf};
use crate::scene;
use crate::session::{Pacing, Session};
use crate::tree::Budget;

/// Run the session's driver until the session closes.
pub fn run(s: Arc<Session>) {
    let mut last_stats = Instant::now();
    loop {
        if s.closing.load(Ordering::SeqCst) {
            return;
        }

        // Wait until there is something to do. A round nobody can see is a
        // round that would never be acknowledged, so a detached session does
        // not advance — which is also what makes a resume pick up mid-run
        // exactly where it stopped.
        {
            let guard = s.ctl.lock().unwrap_or_else(|p| p.into_inner());
            let (guard, timeout) = s
                .wake
                .wait_timeout_while(guard, Duration::from_millis(250), |c| {
                    !c.wants_to_advance()
                })
                .unwrap_or_else(|p| p.into_inner());
            let idle = timeout.timed_out() || !guard.wants_to_advance();
            drop(guard);
            if s.closing.load(Ordering::SeqCst) {
                return;
            }
            if !s.attached() {
                continue;
            }
            if idle {
                // Nothing admitted this pass; keep the panel's readouts live.
                if last_stats.elapsed() >= Duration::from_millis(250) {
                    emit_stats(&s);
                    last_stats = Instant::now();
                }
                continue;
            }
        }

        if !s.attached() {
            continue;
        }

        // --- admit one round ------------------------------------------------
        let (round, steps_json, quiescent, scene_json, cores, pacing, in_flight, buffer) = {
            let mut x = s.exec.lock().unwrap_or_else(|p| p.into_inner());
            let (cores, quote_depth) = {
                let c = s.ctl.lock().unwrap_or_else(|p| p.into_inner());
                (x.settings().cores, c.quote_depth)
            };

            let steps = match x.tick() {
                Ok(v) => v,
                Err(e) => {
                    s.emit(&crate::json::error_frame("store", &format!("{e:?}")));
                    let mut c = s.ctl.lock().unwrap_or_else(|p| p.into_inner());
                    c.running = false;
                    c.steps_owed = 0;
                    continue;
                }
            };
            let quiescent = x.is_quiescent();
            let steps_json = f1r3x::json::steps(&steps);

            // The scene is rebuilt per round rather than diffed. A round is at
            // most `cores` commits and a scene is the machine's queue plus
            // what is at rest in the store, so this is small; a diff would be
            // an optimisation with a correctness cost, since the client would
            // have to reconstruct state the executive already holds.
            let mut sb = Buf::with_capacity(8192);
            scene::write(
                &mut sb,
                x.machine(),
                x.tick_count(),
                Budget::with_quote_depth(quote_depth),
            );
            let scene_json = sb.into_string();

            let mut c = s.ctl.lock().unwrap_or_else(|p| p.into_inner());
            c.round += 1;
            c.in_flight += 1;
            if c.steps_owed > 0 {
                c.steps_owed -= 1;
            }
            if quiescent {
                // Nothing left to advance. Stop asking.
                c.running = false;
                c.steps_owed = 0;
            }
            // Throughput, for the readout beside the cores slider (§9.3).
            let (_, comms) = x.machine().counters();
            let dt = c.comms_at.elapsed().as_secs_f64();
            if dt >= 0.5 {
                c.rate = (comms.saturating_sub(c.comms_mark)) as f64 / dt;
                c.comms_mark = comms;
                c.comms_at = Instant::now();
            }
            (
                c.round,
                steps_json,
                quiescent,
                scene_json,
                cores,
                c.pacing,
                c.in_flight,
                c.buffer,
            )
        };

        // --- release it to the scene -----------------------------------------
        s.emit(&frame("round", |b| {
            b.comma();
            b.field_num("round", round);
            b.comma();
            b.field_num("cores", cores as u64);
            b.comma();
            b.field_str("pacing", pacing.name());
            b.comma();
            b.field_raw("steps", &steps_json);
            b.comma();
            b.field_bool("quiescent", quiescent);
            b.comma();
            // What the client must send back to free admission. In lockstep
            // this is the gate; in free-running it is the drain.
            b.field_bool("ackRequired", true);
            b.comma();
            b.field_num("inFlight", in_flight);
            b.comma();
            b.field_num("buffer", if pacing == Pacing::Lockstep { 1 } else { buffer });
            b.comma();
            b.field_raw("scene", &scene_json);
        }));

        if quiescent {
            s.emit(&frame("quiescent", |b| {
                b.comma();
                b.field_num("round", round);
            }));
        }

        emit_stats(&s);
        last_stats = Instant::now();
    }
}

/// The panel's readouts: buffer fill, whether admission is open or paused by
/// back-pressure, and commits per second (§9.1 playback group, §9.3).
pub fn emit_stats(s: &Arc<Session>) {
    let (cores, tick, comms, steps, queued, data, waiting, deploys, quiescent) = {
        let x = s.exec.lock().unwrap_or_else(|p| p.into_inner());
        let (st, comms) = x.machine().counters();
        let (data, waiting) = x.machine().space().stats();
        (
            x.settings().cores,
            x.tick_count(),
            comms,
            st,
            x.machine().queued(),
            data,
            waiting,
            x.deploys().len(),
            x.is_quiescent(),
        )
    };
    let c = s.ctl.lock().unwrap_or_else(|p| p.into_inner());
    let bound = match c.pacing {
        Pacing::Lockstep => 1,
        Pacing::FreeRunning => c.buffer.max(1),
    };
    let open = c.admission_open();
    let f = frame("stats", |b| {
        b.comma();
        b.field_num("round", c.round);
        b.comma();
        b.field_num("tick", tick);
        b.comma();
        b.field_str("pacing", c.pacing.name());
        b.comma();
        b.field_bool("running", c.running);
        b.comma();
        b.field_num("cores", cores as u64);
        b.comma();
        b.field_num("coresMax", s.cores_max as u64);
        b.comma();
        // Buffer fill and admission status, which is the back-pressure the
        // panel shows: `admissionOpen: false` is the evaluator waiting for the
        // scene, not a stall.
        b.field_num("inFlight", c.in_flight);
        b.comma();
        b.field_num("buffer", bound);
        b.comma();
        b.field_bool("admissionOpen", open);
        b.comma();
        b.field_num("commitsPerSec", c.rate.round() as u64);
        b.comma();
        b.field_num("comms", comms);
        b.comma();
        b.field_num("steps", steps);
        b.comma();
        b.field_num("queued", queued as u64);
        b.comma();
        b.field_num("data", data as u64);
        b.comma();
        b.field_num("waiting", waiting as u64);
        b.comma();
        b.field_num("deploys", deploys as u64);
        b.comma();
        b.field_bool("quiescent", quiescent);
    });
    drop(c);
    s.emit(&f);
}

/// Send the scene as it stands, without advancing anything. Used on attach and
/// after a deploy, so a client that has just connected draws the world before
/// the first round.
pub fn emit_scene(s: &Arc<Session>) {
    let (scene_json, round) = {
        let x = s.exec.lock().unwrap_or_else(|p| p.into_inner());
        let quote_depth = s
            .ctl
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .quote_depth;
        let mut b = Buf::with_capacity(8192);
        scene::write(
            &mut b,
            x.machine(),
            x.tick_count(),
            Budget::with_quote_depth(quote_depth),
        );
        let round = s.ctl.lock().unwrap_or_else(|p| p.into_inner()).round;
        (b.into_string(), round)
    };
    s.emit(&frame("scene", |b| {
        b.comma();
        b.field_num("round", round);
        b.comma();
        b.field_raw("scene", &scene_json);
    }));
}
