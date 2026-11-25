//! Spans: placing a deck between two points of the ground, the `span` parameter of
//! `entity.add` and `prefab.spawn`.
//!
//! A span gives two ends on the ground plane. What it places stands level at the midpoint,
//! its base at the higher end's ground plus an offset (it never sinks into the ground; how far
//! the other end is above its ground is in the answer, so a generator can place an abutment),
//! faces from `from` to `to`, and is scaled so that its length along its forward axis spans
//! the ends. Entities turn about the vertical axis only, so a span over sloped ground cannot
//! tilt: full orientation is M3 work.

use crate::error::{CommandError, bad, failed};
use crate::params::Params;
use crate::session::Edit;
use gamecraft_math::yaw_of;
use gamecraft_world::limits::{MAX_COORDINATE_M, SCALE_MAX, SCALE_MIN};
use gamecraft_world::{GroundBox, Scale};
use serde_json::{Value, json};

/// The keys of the `span` object, for the nearest-name hint.
const SPAN_KEYS: [&str; 4] = ["from", "to", "offset_m", "length_m"];

/// Two ends closer than this, metres, are one point.
const SAME_POINT_M: f64 = 1e-6;

/// What a span places, and what to report of it.
pub(crate) struct Span {
    /// Where the base stands.
    pub position: [f64; 3],
    /// Its turn: it faces from `from` to `to`.
    pub rotation_y_deg: f64,
    /// Its scale: 1 across and 1 high, so that only its length (along its own z) grows to
    /// span the ends.
    pub scale: Scale,
    /// The `span` object of the answer.
    pub report: Value,
}

/// What there is to measure of a thing a span places, in its own frame at scale 1: its length
/// along its forward axis (local z) and where its centre lies along that axis (0: on its
/// origin; positive: behind it).
pub(crate) struct Measured {
    length_m: f64,
    centre_m: f64,
}

impl Measured {
    /// Of the smallest box around the thing in its own frame ([`GroundBox`]).
    pub(crate) fn of_box(b: GroundBox) -> Measured {
        Measured {
            length_m: b.1[1] - b.0[1],
            centre_m: (b.0[1] + b.1[1]) / 2.0,
        }
    }
}

/// A number of metres as a sentence shows it: at most three decimals, no trailing zeros.
fn metres(v: f64) -> String {
    let text = format!("{v:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The field `at` (`span.from`) of the `span` object: `None` when left out.
fn field<'a>(span: &Params<'a>, at: &str) -> Option<&'a Value> {
    span.value(at.trim_start_matches("span."))
}

/// `[x, z]` of the field `at` of the `span` object; an error naming it when it is missing or
/// not a point.
fn point(span: &Params<'_>, at: &str) -> Result<[f64; 2], CommandError> {
    let value = field(span, at).ok_or_else(|| {
        bad(format!(
            "missing `{at}` ([x, z], a point of the ground plane)"
        ))
    })?;
    let wrong = || {
        bad(format!(
            "`{at}` must be [x, z] in metres, got {}",
            crate::spec::kind_of(value)
        ))
    };
    let items = value.as_array().ok_or_else(wrong)?;
    if items.len() != 2 {
        return Err(wrong());
    }
    let mut out = [0.0; 2];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = item.as_f64().filter(|x| x.is_finite()).ok_or_else(wrong)?;
    }
    Ok(out)
}

/// The finite number of the field `at` of the `span` object, `None` when left out; an error
/// naming it for anything else.
fn number(span: &Params<'_>, at: &str) -> Result<Option<f64>, CommandError> {
    match field(span, at) {
        None => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|x| x.is_finite())
            .map(Some)
            .ok_or_else(|| {
                bad(format!(
                    "`{at}` must be a number of metres, got {}",
                    crate::spec::kind_of(v)
                ))
            }),
    }
}

/// The span of the `span` parameter: where it places what it is given, turned and scaled.
///
/// `measured` is the extent of what is placed when it can be measured; when it cannot,
/// `what` names it ("the entity", "prefab \"deck\"") and `why` says what it lacks, and the
/// span asks for `length_m`.
pub(crate) fn span(
    edit: &Edit<'_>,
    p: &Params<'_>,
    measured: Option<Measured>,
    what: &str,
    why: &str,
) -> Result<Span, CommandError> {
    // The spec refused a `span` that is not an object, so this is always some.
    let object = p.value("span").and_then(Value::as_object);
    if let Some(key) = object
        .into_iter()
        .flat_map(|o| o.keys())
        .find(|k| !SPAN_KEYS.contains(&k.as_str()))
    {
        let hint = crate::spec::nearest(key, SPAN_KEYS.into_iter())
            .map(|n| format!("; did you mean `{n}`?"))
            .unwrap_or_default();
        return Err(bad(format!(
            "`span.{}` is not a key of a span; a span takes {}{hint}",
            crate::spec::excerpt(key),
            SPAN_KEYS.join(", ")
        )));
    }
    let span = Params::new(object);
    let from = point(&span, "span.from")?;
    let to = point(&span, "span.to")?;
    let (dx, dz) = (to[0] - from[0], to[1] - from[1]);
    let length_between = (dx * dx + dz * dz).sqrt();
    if !(length_between.is_finite() && length_between > SAME_POINT_M) {
        return Err(bad(format!(
            "`span.from` and `span.to` are the same point [{}, {}]; the span has no length",
            metres(from[0]),
            metres(from[1])
        )));
    }
    let ground = |x: f64, z: f64, at: &str| {
        edit.ground(x, z)
            .map_err(|e| failed(format!("{} ({at})", e.message())))
    };
    let ground_from = ground(from[0], from[1], "span.from")?;
    let ground_to = ground(to[0], to[1], "span.to")?;
    let offset = number(&span, "span.offset_m")?.unwrap_or(0.0);
    if offset.abs() > MAX_COORDINATE_M {
        return Err(bad(format!(
            "`span.offset_m` is {offset:?}; it must be a finite number of metres from \
             -{MAX_COORDINATE_M} to {MAX_COORDINATE_M}"
        )));
    }
    let length = match number(&span, "span.length_m")? {
        Some(given) => {
            if !(given > 0.0 && given <= MAX_COORDINATE_M) {
                return Err(bad(format!(
                    "`span.length_m` is {given:?}; it must be a number of metres above 0 and \
                     at most {MAX_COORDINATE_M}"
                )));
            }
            Measured {
                length_m: given,
                centre_m: 0.0,
            }
        }
        None => match measured {
            Some(m) => m,
            None => {
                return Err(bad(format!(
                    "`span` needs `length_m`: {what} {why}; give its length along its forward \
                     axis at scale 1, in metres"
                )));
            }
        },
    };
    let length_scale = length_between / length.length_m;
    if !(SCALE_MIN..=SCALE_MAX).contains(&length_scale) {
        return Err(bad(format!(
            "`span` is {} m between its ends and {what} is {} m long, which would scale it by \
             {}; an entity's scale is from {SCALE_MIN} to {SCALE_MAX}. Give `length_m`, or \
             place something else",
            metres(length_between),
            metres(length.length_m),
            metres(length_scale)
        )));
    }
    // Level, on the higher end's ground: a deck never sinks into the ground it spans.
    let y = ground_from.max(ground_to) + offset;
    // Its centre lies `centre_m` behind its origin (towards `from`); so the origin goes that
    // much past the midpoint, towards `to`, and the thing covers exactly the two ends.
    let centre = length.centre_m * length_scale;
    let (mx, mz) = ((from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0);
    let (fx, fz) = (dx / length_between, dz / length_between);
    let position = [mx + fx * centre, y, mz + fz * centre];
    let rotation_y_deg = yaw_of(dx, dz).ok_or_else(|| {
        bad(format!(
            "`span.from` and `span.to` are too far apart to measure: {dx} m and {dz} m"
        ))
    })?;
    let end = |x: f64, z: f64, ground: f64| json!({"point": [x, y, z], "ground_m": ground, "gap_m": y - ground});
    let report = json!({
        "from": from,
        "to": to,
        "length_m": length.length_m,
        "scale": Scale::per_axis(1.0, 1.0, length_scale),
        "ends": [
            end(from[0], from[1], ground_from),
            end(to[0], to[1], ground_to)
        ],
    });
    Ok(Span {
        position,
        rotation_y_deg,
        scale: Scale::per_axis(1.0, 1.0, length_scale),
        report,
    })
}
