//! The structural form of a term, for the renderer.
//!
//! `f1r3x`'s scene gives each drawable thing a *label*: source-shaped text,
//! which is right for a list and useless for a hand. Rhobots draws a robot per
//! constructor — a sender's hand bears a glyph and offers a capsule, a waiter
//! has a binder platform and a continuation one plane behind it — so the
//! client needs the shape of the term, not a rendering of it. That is all this
//! module adds: the same `Norm` the executive is already holding, written out
//! as the vocabulary of Rhobots §4, so that the layout function L(P, z, s) of
//! §4.1 can be evaluated on the device.
//!
//! Three properties are carried deliberately.
//!
//! **Every node carries its fingerprint.** `fp` is the content hash of the
//! subterm, hex. It is the same 32 bytes the store keys by, so two glyphs that
//! agree are the visual witness that a communication is possible (§3.1), and
//! the client never decides name equivalence itself.
//!
//! **Elision happens here, not on the device.** Scale carries quotation, so a
//! quote nested k deep is drawn at σ^k, and below s_min it is an elision
//! glyph. The client says how deep it can still draw; anything past that comes
//! back as `elided` *with its fingerprint intact*, which is what §3 asks for —
//! an elided name stays comparable. Without this, one deep term sends
//! megabytes to a headset that would draw three capsules of it.
//!
//! **The walk does not recurse.** K1ndl1ng Req. 4.5 forbids recursion on term
//! structure anywhere, on the grounds that a gesture interface nests quotes by
//! accident. This walks an explicit stack, like every other traversal in the
//! workspace. Each node builds its output in reading order and pushes it
//! reversed, so the text is written left to right by a loop that never nests.

use k1ndl1ng_norm::{BindKind, Name, Node, Norm};

use crate::json::Buf;

/// How much of a term is written out before it becomes an elision glyph.
#[derive(Copy, Clone, Debug)]
pub struct Budget {
    /// Quotation levels drawn. This counts the exponent k in σ^k, not
    /// parallel nesting, so it corresponds directly to the panel's legibility
    /// threshold.
    pub quote_depth: u32,
    /// Nodes written for one term, whatever the depth: a backstop against a
    /// wide term rather than a deep one.
    pub nodes: u32,
}

impl Default for Budget {
    fn default() -> Self {
        // Four quotation levels at σ = 0.3 is a scale of 0.008, under every
        // legibility threshold the panel offers; 4096 nodes is far more than a
        // person can have arranged by hand.
        Budget {
            quote_depth: 4,
            nodes: 4096,
        }
    }
}

impl Budget {
    pub fn with_quote_depth(d: u32) -> Budget {
        Budget {
            quote_depth: d.min(16),
            ..Budget::default()
        }
    }
}

/// Write `t` as a JSON tree in the rhobot vocabulary.
pub fn write(out: &mut Buf, t: &Norm, budget: Budget) {
    let mut w = Walk {
        out,
        left: budget.nodes,
        max_q: budget.quote_depth,
    };
    let mut stack: Vec<Task> = vec![Task::Proc(t.clone(), 0)];
    while let Some(task) = stack.pop() {
        match task {
            Task::Lit(s) => w.out.raw(s),
            Task::Owned(s) => w.out.raw(&s),
            Task::Proc(p, q) => w.proc(&p, q, &mut stack),
            Task::Name(n, q) => w.name(&n, q, &mut stack),
        }
    }
}

/// Convenience for a single term.
pub fn to_string(t: &Norm, budget: Budget) -> String {
    let mut b = Buf::new();
    write(&mut b, t, budget);
    b.into_string()
}

/// One item of work. The `u32` is the quotation depth the item sits at.
enum Task {
    Proc(Norm, u32),
    Name(Name, u32),
    Lit(&'static str),
    Owned(String),
}

/// Push a sequence written in reading order so that it pops in that order.
fn schedule(stack: &mut Vec<Task>, seq: Vec<Task>) {
    stack.extend(seq.into_iter().rev());
}

/// Append `items` with commas between and none after — the shape every list in
/// this encoding needs. Each item is itself a sequence, because a bind is
/// several tasks long.
fn commas(seq: &mut Vec<Task>, items: Vec<Vec<Task>>) {
    let n = items.len();
    for (i, item) in items.into_iter().enumerate() {
        seq.extend(item);
        if i + 1 < n {
            seq.push(Task::Lit(","));
        }
    }
}

struct Walk<'a> {
    out: &'a mut Buf,
    left: u32,
    max_q: u32,
}

impl Walk<'_> {
    /// True when this item must be replaced by an elision glyph.
    fn spent(&mut self, q: u32) -> bool {
        if self.left == 0 || q > self.max_q {
            return true;
        }
        self.left -= 1;
        false
    }

    /// The head every process node shares: what it is, its fingerprint, and
    /// the metrics the client sizes it by.
    fn head(&mut self, p: &Norm, form: &str) {
        self.out.raw("{\"f\":\"");
        self.out.raw(form);
        self.out.raw("\",\"fp\":\"");
        self.out.raw(&p.hash().hex());
        self.out.raw("\",\"size\":");
        self.out.num(p.size() as u64);
        self.out.raw(",\"depth\":");
        self.out.num(p.depth() as u64);
    }

    fn proc(&mut self, p: &Norm, q: u32, stack: &mut Vec<Task>) {
        if self.spent(q) {
            self.head(p, "elided");
            self.out.raw("}");
            return;
        }
        match p.node() {
            Node::Nil => {
                // The inert robot: grey, eyes closed, never acts.
                self.head(p, "nil");
                self.out.raw("}");
            }

            Node::Par(parts) => {
                // Adjacency on one plane. Left-to-right order carries no
                // meaning (§4), but the executive's order is encoding order,
                // so it is stable frame to frame and the client can keep a
                // robot's identity across a re-layout.
                self.head(p, "par");
                self.out.raw(",\"parts\":[");
                let mut seq = Vec::new();
                commas(
                    &mut seq,
                    parts.iter().map(|c| vec![Task::Proc(c.clone(), q)]).collect(),
                );
                seq.push(Task::Lit("]}"));
                schedule(stack, seq);
            }

            Node::Send {
                chan,
                persistent,
                args,
            } => {
                // A sending robot: one hand bearing the channel glyph, and a
                // capsule holding the payload at scale σ. No continuation
                // stands behind it.
                self.head(p, "send");
                self.out.raw(",\"persistent\":");
                self.out.raw(if *persistent { "true" } else { "false" });
                self.out.raw(",\"chan\":");
                let mut seq = vec![Task::Name(chan.clone(), q), Task::Lit(",\"args\":[")];
                commas(
                    &mut seq,
                    args.iter()
                        // The capsule is a quotation: its contents are drawn
                        // one scale level in.
                        .map(|a| vec![Task::Proc(a.clone(), q + 1)])
                        .collect(),
                );
                seq.push(Task::Lit("]}"));
                schedule(stack, seq);
            }

            Node::Receive { binds, body } => {
                // A waiting robot: a hand bearing the channel glyph, a binder
                // platform per pattern, and the continuation one plane behind,
                // joined by a tethering cable. At K0 there is exactly one bind
                // with one pattern; the rest is what Rhobots §11 reserves for
                // joins, and the client may draw it as several appendages.
                self.head(p, "receive");
                self.out.raw(",\"binds\":[");
                let mut seq = Vec::new();
                let binds: Vec<Vec<Task>> = binds
                    .iter()
                    .map(|b| {
                        let mut one = vec![
                            Task::Owned(format!(
                                "{{\"kind\":\"{}\",\"arrow\":\"{}\",\"binders\":{},\"chan\":",
                                kind_name(b.kind),
                                b.kind.arrow(),
                                b.binders
                            )),
                            Task::Name(b.chan.clone(), q),
                            Task::Lit(",\"pats\":["),
                        ];
                        commas(
                            &mut one,
                            b.pats.iter().map(|x| vec![Task::Name(x.clone(), q)]).collect(),
                        );
                        one.push(Task::Lit("]}"));
                        one
                    })
                    .collect();
                commas(&mut seq, binds);
                seq.push(Task::Lit("],\"body\":"));
                seq.push(Task::Proc(body.clone(), q));
                seq.push(Task::Lit("}"));
                schedule(stack, seq);
            }

            Node::New { count, body } => {
                // Reserved vocabulary: an enclosure on the plane, whose minted
                // channels carry unforgeable fingerprints (§11).
                self.head(p, "new");
                self.out.raw(",\"count\":");
                self.out.num(*count as u64);
                self.out.raw(",\"body\":");
                schedule(stack, vec![Task::Proc(body.clone(), q), Task::Lit("}")]);
            }

            Node::Eval(n) => {
                // `*x` at a process position: the full-size platform under a
                // dashed silhouette, marked with the unshrink icon (§4). When
                // substitution reaches it the payload unshrinks into it at the
                // platform's own scale (§7.2).
                self.head(p, "drop");
                self.out.raw(",\"name\":");
                schedule(stack, vec![Task::Name(n.clone(), q), Task::Lit("}")]);
            }

            Node::BoundVar(i) => {
                self.head(p, "var");
                self.out.raw(",\"bound\":true,\"index\":");
                self.out.num(*i as u64);
                self.out.raw("}");
            }

            Node::FreeVar(l) => {
                self.head(p, "var");
                self.out.raw(",\"bound\":false,\"index\":");
                self.out.num(*l as u64);
                self.out.raw("}");
            }

            Node::Wild => {
                self.head(p, "wild");
                self.out.raw("}");
            }
        }
    }

    fn name(&mut self, n: &Name, q: u32, stack: &mut Vec<Task>) {
        let fp = n.content_hash().hex();
        match n {
            Name::Quote(p) => {
                // A capsule. Quotation is what scale carries, so its contents
                // go one level in; past the budget it is the elision glyph,
                // which keeps the fingerprint so the name stays comparable.
                if self.spent(q) {
                    self.out.raw("{\"g\":\"quote\",\"elided\":true,\"fp\":\"");
                    self.out.raw(&fp);
                    self.out.raw("\",\"size\":");
                    self.out.num(p.size() as u64);
                    self.out.raw(",\"depth\":");
                    self.out.num(p.depth() as u64);
                    self.out.raw("}");
                    return;
                }
                self.out.raw("{\"g\":\"quote\",\"elided\":false,\"fp\":\"");
                self.out.raw(&fp);
                self.out.raw("\",\"proc\":");
                schedule(stack, vec![Task::Proc(p.clone(), q + 1), Task::Lit("}")]);
            }
            Name::Bound(i) => self.slot(&fp, true, *i),
            Name::Free(l) => self.slot(&fp, false, *l),
            Name::Unforgeable(b) => {
                // Named as the executive's printer names it, so a channel that
                // reads `u_3375c662` in a card's label reads the same here.
                self.out.raw("{\"g\":\"unforgeable\",\"fp\":\"");
                self.out.raw(&fp);
                self.out.raw("\",\"label\":\"");
                self.out
                    .raw(&format!("u_{:02x}{:02x}{:02x}{:02x}", b[0], b[1], b[2], b[3]));
                self.out.raw("\"}");
            }
        }
    }

    /// A glyph slot: a small platform labelled by its binder, where a shrunken
    /// robot will stand once substitution fills it (§4, §7.2 name position).
    fn slot(&mut self, fp: &str, bound: bool, index: u32) {
        self.out.raw("{\"g\":\"slot\",\"bound\":");
        self.out.raw(if bound { "true" } else { "false" });
        self.out.raw(",\"index\":");
        self.out.num(index as u64);
        self.out.raw(",\"fp\":\"");
        self.out.raw(fp);
        self.out.raw("\",\"label\":\"");
        self.out.raw(if bound { "n" } else { "f" });
        self.out.num(index as u64);
        self.out.raw("\"}");
    }
}

fn kind_name(k: BindKind) -> &'static str {
    match k {
        BindKind::Linear => "linear",
        BindKind::Persistent => "persistent",
        BindKind::Peek => "peek",
    }
}
