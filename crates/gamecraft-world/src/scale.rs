//! An entity's scale: one number for every axis, or one along each of its own axes.
//!
//! In a file a scale is one number (`"scale": 2`) when it is the same on every axis, as it
//! always was, and `[x, y, z]` when it is not. So a world that never scales per axis has the
//! text and the state hash it had. What a scale means where depends on what is scaled:
//! shapes, models and the offsets of a prefab's children take it along their own axes
//! ([`Scale::times`]); what has one size (the width of a cylinder or a sphere, a plant) takes
//! the middle of the three ([`Scale::middle`]), so that stretching something along one axis
//! leaves it as it was.

use crate::error::{Ctx, WorldError};
use crate::limits::{SCALE_MAX, SCALE_MIN};
use gamecraft_math::wrap_deg;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser::SerializeTuple};
use std::fmt;

/// An entity's scale along its own x, y and z axes, each 0.001 to 1000 (checked where the
/// entity is: [`Scale::validate`]). The same on every axis is one number in a file.
#[derive(Clone, Copy, PartialEq)]
pub struct Scale {
    /// Along the entity's own x axis (its right).
    pub x: f64,
    /// Along y (up).
    pub y: f64,
    /// Along the entity's own z axis (its back; its forward is -Z).
    pub z: f64,
}

impl Default for Scale {
    /// [`Scale::ONE`].
    fn default() -> Self {
        Scale::ONE
    }
}

impl fmt::Debug for Scale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_uniform() {
            write!(f, "Scale({})", self.x)
        } else {
            write!(f, "Scale[{}, {}, {}]", self.x, self.y, self.z)
        }
    }
}

/// The one of three that is neither the largest nor the smallest.
fn median(a: f64, b: f64, c: f64) -> f64 {
    if (a <= b && b <= c) || (c <= b && b <= a) {
        b
    } else if (b <= a && a <= c) || (c <= a && a <= b) {
        a
    } else {
        c
    }
}

/// A turn is a right angle (or a multiple) this close, in degrees.
const RIGHT_ANGLE_TOLERANCE_DEG: f64 = 1e-9;

impl Scale {
    /// The scale 1 on every axis.
    pub const ONE: Scale = Scale {
        x: 1.0,
        y: 1.0,
        z: 1.0,
    };

    /// The same scale on every axis.
    pub const fn uniform(scale: f64) -> Scale {
        Scale {
            x: scale,
            y: scale,
            z: scale,
        }
    }

    /// A scale per axis.
    pub const fn per_axis(x: f64, y: f64, z: f64) -> Scale {
        Scale { x, y, z }
    }

    /// `[x, y, z]`.
    pub const fn get(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }

    /// Whether every axis has the same scale, to the bit (then a file writes one number).
    pub fn is_uniform(self) -> bool {
        self.x.to_bits() == self.y.to_bits() && self.x.to_bits() == self.z.to_bits()
    }

    /// The scale for what has one size: the middle of the three. Stretching an entity along
    /// one axis leaves it as it was; scaling it on two, or on all three, takes it along.
    pub fn middle(self) -> f64 {
        median(self.x, self.y, self.z)
    }

    /// The largest of the three: the most anything of the entity grew.
    pub fn largest(self) -> f64 {
        self.x.max(self.y).max(self.z)
    }

    /// A size along the entity's own axes, scaled: `size` times the scale, axis by axis.
    pub fn times(self, size: [f64; 3]) -> [f64; 3] {
        [size[0] * self.x, size[1] * self.y, size[2] * self.z]
    }

    /// This scale divided by `other`, axis by axis: how much something grew.
    pub(crate) fn over(self, other: Scale) -> Scale {
        Scale::per_axis(self.x / other.x, self.y / other.y, self.z / other.z)
    }

    /// This scale times `other`, axis by axis.
    pub(crate) fn by(self, other: Scale) -> Scale {
        Scale::per_axis(self.x * other.x, self.y * other.y, self.z * other.z)
    }

    /// This scale as it is along the axes of something turned by `turn_deg` relative to the
    /// frame the scale is given in: the scale of a parent, in a child's own frame. A child
    /// turned a right angle (or any multiple) has its x along the parent's z and its z along
    /// the parent's x, so those two swap; turned by any other angle it is not a box that
    /// scales per axis, and the child keeps the scale along its own axes as it is.
    pub(crate) fn in_turned_frame(self, turn_deg: f64) -> Scale {
        let turn = wrap_deg(turn_deg);
        let quarters = (turn / 90.0).round();
        if (turn - quarters * 90.0).abs() > RIGHT_ANGLE_TOLERANCE_DEG {
            return self;
        }
        // 0 and 180: as they are; 90 and 270: x and z swap.
        if quarters.rem_euclid(2.0) == 0.0 {
            self
        } else {
            Scale::per_axis(self.z, self.y, self.x)
        }
    }

    /// Each axis finite and from [`SCALE_MIN`] to [`SCALE_MAX`]; the error names the axis
    /// (`scale[1]`).
    pub(crate) fn validate(self, ctx: Ctx) -> Result<(), WorldError> {
        for (axis, value) in self.get().iter().enumerate() {
            ctx.range(format!("scale[{axis}]"), *value, SCALE_MIN, SCALE_MAX, "")?;
        }
        Ok(())
    }
}

/// How a scale is written: one number when it is the same on every axis, else `[x, y, z]`.
impl Serialize for Scale {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.is_uniform() {
            serializer.serialize_f64(self.x)
        } else {
            let mut tuple = serializer.serialize_tuple(3)?;
            tuple.serialize_element(&self.x)?;
            tuple.serialize_element(&self.y)?;
            tuple.serialize_element(&self.z)?;
            tuple.end()
        }
    }
}

/// How a scale is read: a number, or `[x, y, z]`.
impl<'de> Deserialize<'de> for Scale {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ScaleVisitor;
        impl<'de> de::Visitor<'de> for ScaleVisitor {
            type Value = Scale;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a scale: a number, or [x, y, z]")
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Scale, E> {
                Ok(Scale::uniform(value))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Scale, E> {
                Ok(Scale::uniform(value as f64))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Scale, E> {
                Ok(Scale::uniform(value as f64))
            }

            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Scale, A::Error> {
                let mut axes = [1.0; 3];
                for (i, slot) in axes.iter_mut().enumerate() {
                    // An element that is not a number would be reported as "expected f64":
                    // say what it is part of.
                    let next = seq.next_element::<f64>().map_err(|e| {
                        de::Error::custom(format!(
                            "a scale is a number, or [x, y, z] of numbers: {e}"
                        ))
                    })?;
                    match next {
                        Some(value) => *slot = value,
                        None => {
                            return Err(de::Error::invalid_length(i, &"three numbers, [x, y, z]"));
                        }
                    }
                }
                if seq.next_element::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::invalid_length(4, &"three numbers, [x, y, z]"));
                }
                Ok(Scale::per_axis(axes[0], axes[1], axes[2]))
            }
        }
        deserializer.deserialize_any(ScaleVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uniform_scale_writes_as_one_number_and_reads_from_either() {
        assert_eq!(serde_json::to_string(&Scale::uniform(2.5)).unwrap(), "2.5");
        assert_eq!(serde_json::to_string(&Scale::ONE).unwrap(), "1.0");
        assert_eq!(
            serde_json::to_string(&Scale::per_axis(1.0, 2.0, 3.0)).unwrap(),
            "[1.0,2.0,3.0]"
        );
        assert_eq!(
            serde_json::from_str::<Scale>("2").unwrap(),
            Scale::uniform(2.0)
        );
        assert_eq!(
            serde_json::from_str::<Scale>("2.0").unwrap(),
            Scale::uniform(2.0)
        );
        assert_eq!(
            serde_json::from_str::<Scale>("[1, 2.0, 3]").unwrap(),
            Scale::per_axis(1.0, 2.0, 3.0)
        );
        assert_eq!(
            serde_json::from_str::<Scale>("[2, 2.0, 2]").unwrap(),
            Scale::uniform(2.0)
        );
        for bad in [
            "\"big\"",
            "[1, 2]",
            "[1, 2, 3, 4]",
            "[1, null, 3]",
            "null",
            "[]",
            "{}",
            "[1, [2], 3]",
        ] {
            assert!(serde_json::from_str::<Scale>(bad).is_err(), "{bad}");
        }
        assert_eq!(format!("{:?}", Scale::uniform(2.0)), "Scale(2)");
        assert_eq!(
            format!("{:?}", Scale::per_axis(1.0, 2.0, 3.0)),
            "Scale[1, 2, 3]"
        );
    }

    #[test]
    fn a_parents_scale_in_a_turned_childs_frame() {
        let s = Scale::per_axis(1.0, 2.0, 10.0);
        for turn in [0.0, 180.0, -180.0, 360.0, 720.0] {
            assert_eq!(s.in_turned_frame(turn), s, "{turn}");
        }
        for turn in [90.0, 270.0, -90.0, -270.0, 450.0] {
            assert_eq!(
                s.in_turned_frame(turn),
                Scale::per_axis(10.0, 2.0, 1.0),
                "{turn}"
            );
        }
        for turn in [45.0, 30.0, 89.999, 90.001, 17.3] {
            assert_eq!(s.in_turned_frame(turn), s, "{turn}: not a right angle");
        }
    }
}
