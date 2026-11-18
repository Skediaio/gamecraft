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
}

struct Gpu {
    renderer: WorldRenderer,
    /// The `view_revision` the renderer's scene was last synced to.
    synced: Option<u64>,
}

fn open_gpu() -> Result<Gpu, String> {
    match headless_gpu() {
        Ok((device, queue)) => Ok(Gpu {
            renderer: WorldRenderer::new(device, queue),
            synced: None,
        }),
        Err(RenderError::NoGpu(why)) => Err(format!(
            "no GPU was found ({why}); pictures need a GPU adapter or a software Vulkan \
             adapter (mesa-vulkan-drivers), everything else works without one"
        )),
        Err(other) => Err(other.to_string()),
    }
}

impl Pictures for GpuPictures {
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
        let aspect = f64::from(request.width) / f64::from(request.height);
        let camera = camera_for(world, heights, &request.camera, aspect)?;
        if gpu.synced != Some(view_revision) {
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
