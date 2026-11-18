//! Cameras: where a picture is taken from, and the views the engine offers.

use crate::RenderError;
use crate::plants;
use gamecraft_math::{DVec3, deg_to_rad, yaw_forward};
use gamecraft_world::{Entity, HeightField, TerrainSource, World};

/// Narrowest vertical field of view, degrees.
pub const FOV_MIN_DEG: f64 = 5.0;
/// Widest vertical field of view, degrees.
pub const FOV_MAX_DEG: f64 = 150.0;

/// Vertical field of view of [`Camera::framing`], degrees.
const FRAMING_FOV_DEG: f64 = 60.0;
/// How far above the horizon [`Camera::framing`] looks down from, degrees. Below half the
/// field of view, so a band of sky stays at the top of the picture.
const FRAMING_ELEVATION_DEG: f64 = 24.0;
/// Compass direction from the scene to [`Camera::framing`]: south-east.
const FRAMING_AZIMUTH_DEG: f64 = 135.0;
/// Share of the half frame the scene's corners may reach in [`Camera::framing`].
const FRAMING_MARGIN: f64 = 0.92;
/// Bisection steps of the framing distance.
const FRAMING_STEPS: u32 = 48;
/// Half extent of the box framed when the world holds nothing, metres.
const EMPTY_HALF_EXTENT_M: f64 = 10.0;
/// Smallest radius framed, metres.
const FRAMING_MIN_RADIUS_M: f64 = 1.0;
/// Vertical field of view of [`Camera::first_person`], degrees.
const FIRST_PERSON_FOV_DEG: f64 = 70.0;
/// How far ahead [`Camera::first_person`] puts its target, metres.
const LOOK_AHEAD_M: f64 = 10.0;
/// Vertical field of view of [`Camera::third_person`], degrees.
const THIRD_PERSON_FOV_DEG: f64 = 60.0;
/// How far above the entity's facing [`Camera::third_person`] looks down from, degrees.
const THIRD_PERSON_PITCH_DEG: f64 = 20.0;
/// The height an entity without a body is looked at, metres.
const DEFAULT_FOCUS_HEIGHT_M: f64 = 1.0;
/// Above this share of a full turn up or down, the view's up vector is north instead of +Y.
const STRAIGHT_UP_DOWN: f64 = 0.999;
/// The nearest the near plane comes, and the farthest it goes, metres.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const NEAR_MIN_M: f64 = 0.05;
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const NEAR_MAX_M: f64 = 2.0;
/// The near plane as a share of the distance to the target.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const NEAR_SHARE: f64 = 0.002;
/// The far plane: at least this far, metres, and this many times the distance to the target.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const FAR_MIN_M: f64 = 20_000.0;
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const FAR_SHARE: f64 = 50.0;

/// A perspective camera in world coordinates (metres, Y up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// The eye.
    pub position: [f64; 3],
    /// The point looked at; the picture's centre.
    pub target: [f64; 3],
    /// Vertical field of view, degrees, 5 to 150.
    pub fov_y_deg: f64,
}

impl Camera {
    /// A view that frames the whole world from above and to the south-east.
    ///
    /// The box framed holds the terrain (the height field when given, else the terrain
    /// recipe's square and height range) and every entity from its base to its top. The camera
    /// looks at the box's centre from the south-east, 24 degrees above the horizon, with a
    /// 60 degree field of view, as close as it can while every corner of the box stays inside
    /// a square frame (so any landscape picture shows all of it). An empty world frames 20 m
    /// round the origin.
    pub fn framing(world: &World, heights: Option<&HeightField>) -> Camera {
        let (lo, hi) = world_bounds(world, heights);
        let centre = (lo + hi) * 0.5;
        let radius = ((hi - lo).length() * 0.5).max(FRAMING_MIN_RADIUS_M);
        let (az, el) = (
            deg_to_rad(FRAMING_AZIMUTH_DEG),
            deg_to_rad(FRAMING_ELEVATION_DEG),
        );
        let towards_camera = DVec3::new(az.sin() * el.cos(), el.sin(), -az.cos() * el.cos());
        let corners: Vec<DVec3> = (0..8)
            .map(|k| {
                DVec3::new(
                    if k & 1 == 0 { lo.x } else { hi.x },
                    if k & 2 == 0 { lo.y } else { hi.y },
                    if k & 4 == 0 { lo.z } else { hi.z },
                )
            })
            .collect();
        let at = |distance: f64| Camera {
            position: (centre + towards_camera * distance).to_array(),
            target: centre.to_array(),
            fov_y_deg: FRAMING_FOV_DEG,
        };
        let fits = |camera: &Camera| {
            corners.iter().all(|c| {
                camera
                    .project(c.to_array(), 1.0)
                    .is_some_and(|[x, y]| x.abs() <= FRAMING_MARGIN && y.abs() <= FRAMING_MARGIN)
            })
        };
        // The bounding sphere fits at this distance (its angular radius is the margin's
        // angle), so the bisection keeps a distance that fits as its upper end.
        let half = deg_to_rad(FRAMING_FOV_DEG / 2.0);
        let angle = (FRAMING_MARGIN * half.tan()).atan();
        let mut far = radius / angle.sin() * 1.001;
        let mut near = 0.0;
        for _ in 0..FRAMING_STEPS {
            let mid = (near + far) / 2.0;
            if fits(&at(mid)) {
                far = mid;
            } else {
                near = mid;
            }
        }
        at(far)
    }

    /// From a character's eyes, looking along its facing (`yaw_forward` of its
    /// `rotation_y_deg`), level. `None` when the entity has no character component.
    pub fn first_person(entity: &Entity) -> Option<Camera> {
        let character = entity.character()?;
        let eye = DVec3::from_array(entity.position) + DVec3::Y * character.eye_height_m;
        let forward = yaw_forward(entity.rotation_y_deg);
        Some(Camera {
            position: eye.to_array(),
            target: (eye + forward * LOOK_AHEAD_M).to_array(),
            fov_y_deg: FIRST_PERSON_FOV_DEG,
        })
    }

    /// Behind and above an entity, looking at it: `distance_m` metres from the point looked at
    /// (a character's eyes, half a shape's height, else 1 m above the base), 20 degrees above
    /// its facing. A distance that is not finite and positive gives a camera that
    /// [`PictureRequest::validate`](crate::PictureRequest::validate) refuses.
    pub fn third_person(entity: &Entity, distance_m: f64) -> Camera {
        let focus_height = if let Some(c) = entity.character() {
            c.eye_height_m
        } else if let Some(s) = entity.shape() {
            s.size[1] * entity.scale / 2.0
        } else {
            DEFAULT_FOCUS_HEIGHT_M
        };
        let focus = DVec3::from_array(entity.position) + DVec3::Y * focus_height;
        let forward = yaw_forward(entity.rotation_y_deg);
        let pitch = deg_to_rad(THIRD_PERSON_PITCH_DEG);
        let back = -forward * pitch.cos() + DVec3::Y * pitch.sin();
        Camera {
            position: (focus + back * distance_m).to_array(),
            target: focus.to_array(),
            fov_y_deg: THIRD_PERSON_FOV_DEG,
        }
    }

    /// Checks that every number is finite, the field of view is within 5 to 150 degrees and
    /// the position is not the target. The error names the field and the value.
    pub fn validate(&self) -> Result<(), RenderError> {
        for (name, point) in [
            ("camera.position", self.position),
            ("camera.target", self.target),
        ] {
            for (axis, value) in point.iter().enumerate() {
                if !value.is_finite() {
                    return Err(RenderError::BadRequest(format!(
                        "{name}[{axis}] is {value}; it must be a finite number of metres"
                    )));
                }
            }
        }
        let fov = self.fov_y_deg;
        if !(fov.is_finite() && (FOV_MIN_DEG..=FOV_MAX_DEG).contains(&fov)) {
            return Err(RenderError::BadRequest(format!(
                "camera.fov_y_deg {fov} is outside {FOV_MIN_DEG} to {FOV_MAX_DEG} degrees; \
                 use for example 60"
            )));
        }
        let distance = (DVec3::from_array(self.target) - DVec3::from_array(self.position)).length();
        if !(distance.is_finite() && distance > 1e-6) {
            return Err(RenderError::BadRequest(format!(
                "camera.position {:?} equals camera.target; move the camera away from the \
                 point it looks at",
                self.position
            )));
        }
        Ok(())
    }

    /// Where `point` appears in a picture of width / height `aspect`: normalised device
    /// coordinates, x right and y up, -1 to 1 inside the frame. `None` when the point is not
    /// in front of the camera (or the camera is invalid).
    pub fn project(&self, point: [f64; 3], aspect: f64) -> Option<[f64; 2]> {
        let (eye, forward, right, up) = self.basis()?;
        let rel = DVec3::from_array(point) - eye;
        let depth = rel.dot(forward);
        if depth.is_nan() || depth <= 0.0 {
            return None;
        }
        let t = deg_to_rad(self.fov_y_deg / 2.0).tan();
        let ndc = [
            rel.dot(right) / (depth * t * aspect),
            rel.dot(up) / (depth * t),
        ];
        ndc.iter().all(|v| v.is_finite()).then_some(ndc)
    }

    /// The eye and the unit forward, right and up vectors of the view.
    fn basis(&self) -> Option<(DVec3, DVec3, DVec3, DVec3)> {
        let eye = DVec3::from_array(self.position);
        let forward = (DVec3::from_array(self.target) - eye).try_normalize()?;
        let world_up = view_up(forward);
        let right = forward.cross(world_up).try_normalize()?;
        let up = right.cross(forward);
        Some((eye, forward, right, up))
    }

    /// The view and projection matrices (depth 0 to 1) and the eye, for a picture of width /
    /// height `aspect`. The camera must be valid.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn matrices(&self, aspect: f64) -> (glam::Mat4, glam::Mat4, glam::Vec3) {
        let eye = DVec3::from_array(self.position);
        let target = DVec3::from_array(self.target);
        let forward = (target - eye).normalize_or(DVec3::NEG_Z);
        let view = glam::DMat4::look_at_rh(eye, target, view_up(forward));
        let distance = (target - eye).length();
        let near = (distance * NEAR_SHARE).clamp(NEAR_MIN_M, NEAR_MAX_M);
        let far = (distance * FAR_SHARE).max(FAR_MIN_M);
        let proj = glam::DMat4::perspective_rh(deg_to_rad(self.fov_y_deg), aspect, near, far);
        (view.as_mat4(), proj.as_mat4(), eye.as_vec3())
    }
}

/// The up vector of a view looking along `forward`: +Y, or north when looking straight up or
/// down.
fn view_up(forward: DVec3) -> DVec3 {
    if forward.y.abs() > STRAIGHT_UP_DOWN {
        DVec3::NEG_Z
    } else {
        DVec3::Y
    }
}

/// The box holding the terrain and every entity from its base to its top.
fn world_bounds(world: &World, heights: Option<&HeightField>) -> (DVec3, DVec3) {
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = DVec3::splat(f64::NEG_INFINITY);
    let mut add = |p: DVec3| {
        if p.is_finite() {
            lo = lo.min(p);
            hi = hi.max(p);
        }
    };
    if let Some(h) = heights {
        let ([x0, z0], [x1, z1]) = (h.min(), h.max());
        let (y0, y1) = h.height_range();
        add(DVec3::new(x0, f64::from(y0), z0));
        add(DVec3::new(x1, f64::from(y1), z1));
    } else if let Some(t) = &world.terrain {
        let half = t.size_m / 2.0;
        let (y0, y1) = match t.source {
            TerrainSource::Flat { height_m } => (height_m, height_m),
            TerrainSource::Noise { relief_m, .. } => (-relief_m / 2.0, relief_m / 2.0),
        };
        add(DVec3::new(-half, y0, -half));
        add(DVec3::new(half, y1, half));
    }
    for e in world.entities() {
        let base = DVec3::from_array(e.position);
        add(base);
        add(base + DVec3::Y * entity_height(e));
    }
    if lo.x > hi.x {
        let h = EMPTY_HALF_EXTENT_M;
        return (DVec3::new(-h, 0.0, -h), DVec3::new(h, 2.0, h));
    }
    (lo, hi)
}

/// How tall an entity stands, metres: its shape, its character, or its plant when mature.
fn entity_height(e: &Entity) -> f64 {
    let shape = e.shape().map_or(0.0, |s| s.size[1] * e.scale);
    let character = e.character().map_or(0.0, |c| c.height_m);
    let plant = e.plant().map_or(0.0, |p| {
        plants::mature_height_m(&p.species).unwrap_or(0.0) * p.growth * e.scale
    });
    shape.max(character).max(plant)
}
