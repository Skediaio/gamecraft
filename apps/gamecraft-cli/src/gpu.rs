//! Pictures for the host: `gamecraft-render` on a headless device made at the first picture
//! and kept for the next ones.

use gamecraft_automation::{CameraRequest, Pictures, RenderRequest, Rendered};
use gamecraft_render::{
    Camera, PictureRequest, RenderError, WorldRenderer, encode_png, headless_gpu,
};
use gamecraft_world::{EntityId, HeightField, World};
use serde_json::{Value, json};

/// The host's [`Pictures`]. Without an adapter every picture is an error that says so.
#[derive(Default)]
pub struct GpuPictures {
    gpu: Option<Result<Gpu, String>>,
    /// Asset files handed over before the device was opened, for the renderer it gets.
    pending: Vec<(String, Vec<u8>)>,
}

struct Gpu {
    renderer: WorldRenderer,
    /// The `view_revision` the renderer's scene was last synced to.
    synced: Option<u64>,
    /// The character the last picture's camera followed.
    followed: Option<EntityId>,
}

fn open_gpu() -> Result<Gpu, String> {
    match headless_gpu() {
        Ok((device, queue)) => Ok(Gpu {
            renderer: WorldRenderer::new(device, queue),
            synced: None,
            followed: None,
        }),
        Err(RenderError::NoGpu(why)) => Err(format!(
            "no GPU was found ({why}); pictures need a GPU adapter or a software Vulkan \
             adapter (mesa-vulkan-drivers), everything else works without one"
        )),
        Err(other) => Err(other.to_string()),
    }
}

impl Pictures for GpuPictures {
    fn provide_asset(&mut self, path: &str, bytes: Vec<u8>) {
        match &mut self.gpu {
            Some(Ok(gpu)) => gpu.renderer.provide_asset(path, bytes),
            _ => self.pending.push((path.to_string(), bytes)),
        }
    }

    fn render(
        &mut self,
        world: &World,
        heights: Option<&HeightField>,
        view_revision: u64,
        request: &RenderRequest,
    ) -> Result<Rendered, String> {
        let gpu = match self.gpu.get_or_insert_with(open_gpu) {
            Ok(gpu) => gpu,
            Err(why) => return Err(why.clone()),
        };
        for (path, bytes) in self.pending.drain(..) {
            gpu.renderer.provide_asset(&path, bytes);
        }
        let aspect = f64::from(request.width) / f64::from(request.height);
        let camera = camera_for(world, heights, &request.camera, aspect)?;
        // A camera that follows a character: its model is never culled by distance, which
        // is a change of the scene, not of the world.
        let followed = match &request.camera {
            CameraRequest::FirstPerson { entity } => Some(*entity),
            CameraRequest::ThirdPerson { entity, .. } => Some(*entity),
            CameraRequest::Framing | CameraRequest::LookAt { .. } => None,
        };
        let refollowed = gpu.followed != followed;
        gpu.followed = followed;
        gpu.renderer.set_followed_character(followed);
        if gpu.synced != Some(view_revision) || refollowed {
            gpu.renderer.sync(world, heights);
            gpu.synced = Some(view_revision);
        }
        let picture = gpu
            .renderer
            .render(&PictureRequest {
                width: request.width,
                height: request.height,
                camera,
                time_s: request.time_s,
            })
            .map_err(|e| e.to_string())?;
        let png =
            encode_png(&picture.rgba, picture.width, picture.height).map_err(|e| e.to_string())?;
        Ok(Rendered {
            png,
            info: json!({
                "renderer": "gpu",
                "camera": {
                    "position": camera.position,
                    "target": camera.target,
                    "fov_y_deg": camera.fov_y_deg,
                },
                "stats": {
                    "triangles": picture.stats.triangles,
                    "draws": picture.stats.draws,
                    "instances": picture.stats.instances,
                    "forward_triangles": picture.stats.forward_triangles,
                    "models": picture.stats.models.iter().map(|m| {
                        json!({
                            "asset": m.asset,
                            "instances": m.instances,
                            "drawn": m.drawn,
                            "culled": m.culled,
                            "until_m": m.until_m,
                            "levels": m.levels.iter().map(|l| {
                                json!({"level": l.level, "instances": l.instances, "triangles": l.triangles})
                            }).collect::<Vec<_>>(),
                        })
                    }).collect::<Vec<_>>(),
                },
                "problems": Value::from(picture.problems),
            }),
        })
    }
}

/// The camera a request means, in `world`.
fn camera_for(
    world: &World,
    heights: Option<&HeightField>,
    request: &CameraRequest,
    aspect: f64,
) -> Result<Camera, String> {
    let entity = |id: EntityId| {
        world.entity(id).ok_or_else(|| {
            format!(
                "entity {} does not exist in the world drawn; world.inspect lists the entities",
                id.0
            )
        })
    };
    match request {
        CameraRequest::Framing => Ok(Camera::framing(world, heights, aspect)),
        CameraRequest::LookAt {
            position,
            target,
            fov_y_deg,
        } => Ok(Camera {
            position: *position,
            target: *target,
            fov_y_deg: *fov_y_deg,
        }),
        CameraRequest::FirstPerson { entity: id } => {
            let e = entity(*id)?;
            Camera::first_person(e).ok_or_else(|| {
                format!(
                    "entity `{}` (id {}) has no character component, so it has no eyes to see \
                     through; use view third_person, or give it a character",
                    e.name, e.id.0
                )
            })
        }
        CameraRequest::ThirdPerson {
            entity: id,
            distance_m,
        } => Ok(Camera::third_person(entity(*id)?, *distance_m)),
    }
}
