//! The scene frame the headset draws from.
//!
//! This walks the machine the way `f1r3x::scene::build` does and produces the
//! same three populations with the same identifiers — rhobots, tuples,
//! listeners — and adds one thing to each: the structural tree of
//! [`crate::tree`]. The identifiers are deliberately identical, so a client
//! that has been reading `f1r3x`'s own `scene` command sees the same objects
//! here, with more on them.
//!
//! The walk is repeated rather than shared because `f1r3x::scene::Card` keeps
//! a label and not the term it came from, and the tree has to be written from
//! the term. Keeping the two in one pass is what guarantees a card's `id` and
//! its tree describe the same object.

use b3ll0ws::{Direct, Machine, Sequential};
use campf1r3_core::Keyed;
use f1r3x::scene::name_label;
use k1ndl1ng_campf1r3::Chan;
use k1ndl1ng_norm::{show_pretty, Hash32, Name, Norm, Subst, Val};

use crate::json::Buf;
use crate::tree::{self, Budget};

/// Write the scene as a JSON object.
pub fn write(out: &mut Buf, m: &Machine<Direct, Sequential>, tick: u64, budget: Budget) {
    let (data_n, waiting_n) = m.space().stats();
    let (_, comms) = m.counters();

    out.raw("{");
    out.field_num("tick", tick);
    out.comma();
    out.field_bool("quiescent", m.is_quiescent());
    out.comma();
    out.field_num("queued", m.queued() as u64);
    out.comma();
    out.field_num("data", data_n as u64);
    out.comma();
    out.field_num("waiting", waiting_n as u64);
    out.comma();
    out.field_num("comms", comms);

    // --- rhobots: components waiting to run. The things that move. ---------
    out.comma();
    out.key("rhobots");
    out.raw("[");
    for (i, j) in m.jobs().enumerate() {
        if i > 0 {
            out.comma();
        }
        card(out, "rhobot", &j.term.hash().hex(), &j.term, None, &[], budget);
    }
    out.raw("]");

    let chans = m.channels();

    // --- tuples: data at rest on a channel. The things that sit. ----------
    let mut tuples: Vec<(String, Norm, Chan)> = Vec::new();
    for c in &chans {
        for (at, d) in m.space().get_data(c).into_iter().enumerate() {
            // A datum is an argument list; show it as the send it came from.
            let term = Norm::send(c.name().clone(), false, d.args.clone());
            // The store keys a datum by its content alone, so the same tuple
            // on two channels has one hash. A card must be one drawable
            // object, so its id carries the channel and the row position too.
            let id = format!(
                "{}@{}#{}",
                hex(d.content_hash()),
                &hex(c.content_hash())[..8],
                at
            );
            tuples.push((id, term, c.clone()));
        }
    }
    tuples.sort_by(|a, b| a.0.cmp(&b.0));
    out.comma();
    out.key("tuples");
    out.raw("[");
    for (i, (id, term, c)) in tuples.iter().enumerate() {
        if i > 0 {
            out.comma();
        }
        card(out, "tuple", id, term, Some(c), &[], budget);
    }
    out.raw("]");

    // --- listeners: continuations at rest over a group. The things that wait.
    let mut listeners: Vec<(String, Norm, Chan, Vec<String>, String)> = Vec::new();
    for c in &chans {
        // Singleton groups, then every join this channel belongs to. The store
        // returns a join once per member, so take it only at its first.
        let mut groups: Vec<Vec<Chan>> = vec![vec![c.clone()]];
        for g in m.space().get_joins(c) {
            if g.first() == Some(c) && g.len() > 1 {
                groups.push(g);
            }
        }
        for g in groups {
            for (at, (pats, k, persist)) in m.space().get_waiting(&g).into_iter().enumerate() {
                // Show the body with its binders standing where the patterns
                // do, so it reads as one `for` rather than as a body full of
                // loose de Bruijn indices.
                let shown = if k.binders > 0 {
                    let slots: Vec<Val> = (0..k.binders as u32)
                        .map(|s| Val::Name(Name::Free(s)))
                        .collect();
                    k.body.substitute(&Subst::from_slots(slots))
                } else {
                    k.body.clone()
                };
                let id = format!(
                    "{}@{}#{}",
                    hex(k.content_hash()),
                    &hex(c.content_hash())[..8],
                    at
                );
                let group: Vec<String> = g.iter().map(|x| name_label(x.name())).collect();
                let mut label = format!(
                    "for({}){{ {} }}",
                    g.iter()
                        .zip(pats.iter())
                        .map(|(ch, p)| format!(
                            "{} {} {}",
                            p.pats.iter().map(name_label).collect::<Vec<_>>().join(", "),
                            p.kind.arrow(),
                            name_label(ch.name())
                        ))
                        .collect::<Vec<_>>()
                        .join(" & "),
                    show_pretty(&shown)
                );
                if persist {
                    label.push_str("  // persistent");
                }
                listeners.push((id, shown, c.clone(), group, label));
            }
        }
    }
    listeners.sort_by(|a, b| a.0.cmp(&b.0));
    out.comma();
    out.key("listeners");
    out.raw("[");
    for (i, (id, term, c, group, label)) in listeners.iter().enumerate() {
        if i > 0 {
            out.comma();
        }
        card_labelled(out, "listener", id, term, Some(c), group, label, budget);
    }
    out.raw("]");

    out.raw("}");
}

fn card(
    out: &mut Buf,
    kind: &str,
    id: &str,
    t: &Norm,
    chan: Option<&Chan>,
    group: &[String],
    budget: Budget,
) {
    let label = show_pretty(t);
    card_labelled(out, kind, id, t, chan, group, &label, budget);
}

#[allow(clippy::too_many_arguments)]
fn card_labelled(
    out: &mut Buf,
    kind: &str,
    id: &str,
    t: &Norm,
    chan: Option<&Chan>,
    group: &[String],
    label: &str,
    budget: Budget,
) {
    out.raw("{");
    out.field_str("id", id);
    out.comma();
    out.field_str("kind", kind);
    out.comma();
    out.field_str("label", label);
    out.comma();
    out.field_num("size", t.size() as u64);
    out.comma();
    out.field_num("depth", t.depth() as u64);
    out.comma();
    match chan {
        Some(c) => {
            out.field_str("chan", &name_label(c.name()));
            out.comma();
            // The 32 bytes the store keys this channel by. Two glyphs that
            // agree here are the witness that a communication is possible.
            out.field_str("chanFp", &hex(c.content_hash()));
        }
        None => {
            out.field_raw("chan", "null");
            out.comma();
            out.field_raw("chanFp", "null");
        }
    }
    if !group.is_empty() {
        out.comma();
        out.key("group");
        out.raw("[");
        for (i, g) in group.iter().enumerate() {
            if i > 0 {
                out.comma();
            }
            out.str(g);
        }
        out.raw("]");
    }
    out.comma();
    out.key("tree");
    tree::write(out, t, budget);
    out.raw("}");
}

fn hex(h: campf1r3_core::Hash32) -> String {
    Hash32(h.0).hex()
}
