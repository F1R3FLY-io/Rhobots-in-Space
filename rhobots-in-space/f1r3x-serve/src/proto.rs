//! The request side.
//!
//! `f1r3x-ffi` already defines the vocabulary the headset speaks — `deploy`,
//! `tick`, `run`, `scene`, `trace`, `log`, `cores`, `mode`, `status`, `show` —
//! as a line protocol with JSON replies, designed for exactly this client.
//! None of it is reimplemented here. Anything this module does not recognise
//! goes to [`f1r3x_ffi::dispatch`] verbatim and its reply is passed through,
//! so the socket and the static library accept the same requests and a term
//! that runs under `f1r3x run` runs identically here.
//!
//! What is added is only what a socket needs and a linked library does not:
//! attaching to a session, the pacing controls whose gate is now an
//! acknowledgment rather than a semaphore, and the subscription that pushes
//! rounds instead of waiting to be polled.
//!
//! A request is one WebSocket text frame. It may begin with `#<n> ` to carry a
//! correlation number, which comes back on the reply; a client that only
//! listens for pushed frames can leave it off.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::driver;
use crate::json::{error_frame, frame, Buf};
use crate::session::{Pacing, Registry, Session};

/// What the connection thread should do after a request.
pub enum After {
    Continue,
    /// The client said goodbye, or asked for the session to be destroyed.
    Close,
}

/// Handle one request frame. `seq` correlation is stripped here.
pub fn handle(
    reg: &Arc<Registry>,
    session: &mut Option<Arc<Session>>,
    req: &str,
) -> (String, After) {
    let (seq, rest) = split_seq(req);
    let (body, after) = dispatch(reg, session, rest);
    (wrap(seq, &body), after)
}

fn wrap(seq: Option<u64>, body: &str) -> String {
    frame("reply", |b| {
        b.comma();
        match seq {
            Some(n) => b.field_num("seq", n),
            None => b.field_raw("seq", "null"),
        }
        b.comma();
        b.field_raw("body", body);
    })
}

/// `#12 cores 4` → `(Some(12), "cores 4")`
fn split_seq(req: &str) -> (Option<u64>, &str) {
    let Some(rest) = req.strip_prefix('#') else {
        return (None, req);
    };
    // The number runs to the first space; the request may then contain
    // newlines, as `deploy` does.
    match rest.find(' ') {
        Some(i) => match rest[..i].parse::<u64>() {
            Ok(n) => (Some(n), &rest[i + 1..]),
            Err(_) => (None, req),
        },
        None => (None, req),
    }
}

fn dispatch(
    reg: &Arc<Registry>,
    session: &mut Option<Arc<Session>>,
    req: &str,
) -> (String, After) {
    let (head, _body) = match req.find('\n') {
        Some(i) => (&req[..i], &req[i + 1..]),
        None => (req, ""),
    };
    let head = head.trim();
    let mut words = head.split_whitespace();
    let cmd = words.next().unwrap_or("");
    let args: Vec<&str> = words.collect();

    // `attach` is the only command that may arrive without a session.
    if cmd == "attach" {
        return (attach_reply(reg, session, args.first().copied()), After::Continue);
    }

    let Some(s) = session.clone() else {
        return (
            err("no-session", "attach to a session before anything else"),
            After::Continue,
        );
    };

    match cmd {
        // --- harness verbs ---------------------------------------------------
        "ping" => (ok("{\"pong\":true}"), After::Continue),

        "pacing" => {
            let p = match args.first().copied() {
                Some("free") | Some("free-running") => Pacing::FreeRunning,
                Some("lockstep") => Pacing::Lockstep,
                _ => {
                    return (
                        err("bad-argument", "pacing takes `lockstep` or `free`"),
                        After::Continue,
                    )
                }
            };
            {
                let mut c = s.ctl.lock().unwrap_or_else(|x| x.into_inner());
                // Switching from free-running to lockstep pauses admission
                // until the buffer drains, then continues in lockstep (§9.2).
                // Nothing extra is needed for that: in_flight is already the
                // fill, and lockstep's bound of one keeps admission closed
                // until the acks bring it to zero.
                c.pacing = p;
            }
            s.wake.notify_all();
            (ok(&pacing_state(&s)), After::Continue)
        }

        "buffer" => {
            let n: u64 = match args.first().and_then(|a| a.parse().ok()) {
                Some(n) if (1..=1024).contains(&n) => n,
                _ => {
                    return (
                        err("bad-argument", "buffer takes 1..1024 rounds"),
                        After::Continue,
                    )
                }
            };
            s.ctl.lock().unwrap_or_else(|x| x.into_inner()).buffer = n;
            s.wake.notify_all();
            (ok(&pacing_state(&s)), After::Continue)
        }

        // The scene has finished animating everything up to and including this
        // round. This is the acknowledgment lockstep waits for and the drain
        // free-running's buffer counts.
        "ack" => {
            let upto: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(0);
            s.ack(upto);
            (ok(&pacing_state(&s)), After::Continue)
        }

        "play" => {
            {
                let mut c = s.ctl.lock().unwrap_or_else(|x| x.into_inner());
                c.running = true;
            }
            s.wake.notify_all();
            (ok(&pacing_state(&s)), After::Continue)
        }

        "pause" => {
            {
                let mut c = s.ctl.lock().unwrap_or_else(|x| x.into_inner());
                c.running = false;
                c.steps_owed = 0;
            }
            s.wake.notify_all();
            (ok(&pacing_state(&s)), After::Continue)
        }

        // Single-step admits exactly one round, in either mode (§9.2).
        "step" => {
            {
                let mut c = s.ctl.lock().unwrap_or_else(|x| x.into_inner());
                c.steps_owed = c.steps_owed.saturating_add(1);
            }
            s.wake.notify_all();
            (ok(&pacing_state(&s)), After::Continue)
        }

        // How deep the client can still draw a quotation before it wants an
        // elision glyph. Derived on the device from σ and s_min.
        "treedepth" => {
            let d: u32 = args.first().and_then(|a| a.parse().ok()).unwrap_or(4);
            s.ctl.lock().unwrap_or_else(|x| x.into_inner()).quote_depth = d.min(16);
            (ok(&pacing_state(&s)), After::Continue)
        }

        // The scene as it stands, pushed rather than returned, so it arrives
        // by the same path as every other scene the client draws.
        "resend" => {
            driver::emit_scene(&s);
            driver::emit_stats(&s);
            (ok("{\"resent\":true}"), After::Continue)
        }

        // Throw the world away and keep the session. Cheaper for a person than
        // reconnecting, and it is what the panel's reset does.
        "reset" => {
            {
                let mut x = s.exec.lock().unwrap_or_else(|p| p.into_inner());
                let cores = x.settings().cores;
                *x = f1r3x::Executive::new(f1r3x::Settings {
                    mode: f1r3x::Mode::Lockstep,
                    cores,
                    ..f1r3x::Settings::default()
                });
            }
            {
                let mut c = s.ctl.lock().unwrap_or_else(|x| x.into_inner());
                c.running = false;
                c.steps_owed = 0;
                c.in_flight = 0;
                c.round = 0;
                c.comms_mark = 0;
                c.rate = 0.0;
            }
            s.wake.notify_all();
            driver::emit_scene(&s);
            (ok("{\"reset\":true}"), After::Continue)
        }

        "bye" => {
            reg.remove(&s.id);
            (ok("{\"bye\":true}"), After::Close)
        }

        // --- everything else is the executive's own vocabulary ---------------
        _ => {
            let reply = {
                let mut x = s.exec.lock().unwrap_or_else(|p| p.into_inner());
                f1r3x_ffi::dispatch(&mut x, req)
            };
            // Two of the executive's own commands change what is drawn, so the
            // scene follows them without the client having to ask.
            if matches!(cmd, "deploy" | "tick" | "run") && reply.starts_with("{\"ok\":true") {
                driver::emit_scene(&s);
                driver::emit_stats(&s);
            }
            if cmd == "cores" {
                driver::emit_stats(&s);
            }
            (reply, After::Continue)
        }
    }
}

fn attach_reply(
    reg: &Arc<Registry>,
    session: &mut Option<Arc<Session>>,
    id: Option<&str>,
) -> String {
    let Some(id) = id.filter(|s| !s.is_empty() && s.len() <= 128) else {
        return err(
            "bad-argument",
            "attach takes a session id of 1..128 characters",
        );
    };
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return err(
            "bad-argument",
            "a session id may use letters, digits, '-', '_' and '.'",
        );
    }
    // The connection thread has already attached the socket; this only names
    // the session the thread is bound to, which it set before the first
    // request. If they disagree the client is trying to move between sessions
    // on one socket, which the one-connection-per-session rule forbids.
    match session.as_ref() {
        Some(s) if s.id == id => {
            let c = s.ctl.lock().unwrap_or_else(|p| p.into_inner());
            let mut b = Buf::new();
            b.raw("{");
            b.field_str("session", &s.id);
            b.comma();
            b.field_num("coresMax", s.cores_max as u64);
            b.comma();
            b.field_num("round", c.round);
            b.comma();
            b.field_str("pacing", c.pacing.name());
            b.comma();
            b.field_bool("closing", s.closing.load(Ordering::SeqCst));
            b.raw("}");
            ok(&b.into_string())
        }
        Some(_) => err(
            "already-attached",
            "this connection is already attached to a different session; \
             open a new connection",
        ),
        None => {
            let (s, _) = reg.open(id);
            *session = Some(s);
            ok("{\"attached\":true}")
        }
    }
}

fn pacing_state(s: &Arc<Session>) -> String {
    let c = s.ctl.lock().unwrap_or_else(|p| p.into_inner());
    let bound = match c.pacing {
        Pacing::Lockstep => 1,
        Pacing::FreeRunning => c.buffer.max(1),
    };
    let mut b = Buf::new();
    b.raw("{");
    b.field_str("pacing", c.pacing.name());
    b.comma();
    b.field_bool("running", c.running);
    b.comma();
    b.field_num("round", c.round);
    b.comma();
    b.field_num("inFlight", c.in_flight);
    b.comma();
    b.field_num("buffer", bound);
    b.comma();
    b.field_bool("admissionOpen", c.admission_open());
    b.comma();
    b.field_num("quoteDepth", c.quote_depth as u64);
    b.raw("}");
    b.into_string()
}

/// The executive's success shape, so a client has one reply path.
fn ok(body: &str) -> String {
    format!("{{\"ok\":true,\"result\":{body}}}")
}

/// The executive's failure shape, likewise.
fn err(code: &str, message: &str) -> String {
    format!(
        "{{\"ok\":false,\"diags\":[{{\"lo\":0,\"hi\":0,\"code\":\"{}\",\"message\":\"{}\"}}]}}",
        crate::json::esc(code),
        crate::json::esc(message)
    )
}

/// An unsolicited error, outside any reply.
pub fn push_error(code: &str, message: &str) -> String {
    error_frame(code, message)
}
