//! The frame path: a [`Game`] drawn into a colour target with `gamecraft-render`. A window
//! draws into its surface, the offscreen test into a texture; both take this path.

use crate::game::Game;
use crate::rig::ViewMode;
use gamecraft_render::{Quality, RenderError, WorldRenderer};

/// The renderer of a game's frames on one device.
pub struct GameView {
    renderer: WorldRenderer,
    /// The problems of the last sync, so new ones are told once.
    problems: Vec<String>,
}

impl GameView {
    /// A view on `device` with live frames of `quality`, `width` x `height` pixels. Compiles
    /// the renderer's pipelines, so this takes a moment.
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        quality: Quality,
        (width, height): (u32, u32),
    ) -> GameView {
        let mut renderer = WorldRenderer::new(device, queue);
        renderer.set_quality(quality);
        renderer.resize(width, height);
        GameView {
            renderer,
            problems: Vec::new(),
        }
    }

    /// Hands asset files to the renderer ([`Game::take_assets`]).
    pub fn provide_assets(&mut self, files: Vec<(String, Vec<u8>)>) {
        for (path, bytes) in files {
            self.renderer.provide_asset(&path, bytes);
        }
    }

    /// The frames' size changed (the window was resized).
    pub fn resize(&mut self, width: u32, height: u32) {
        self.renderer.resize(width, height);
    }

    /// Records a frame of `game` into `target` (a view in its texture's own format); submit
    /// `encoder` afterwards. Returns the renderer's problems that are new since the last frame
    /// (an unknown plant species, ...), as sentences to tell.
    pub fn draw(
        &mut self,
        game: &Game,
        target: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<Vec<String>, RenderError> {
        let world = game.display_world();
        self.renderer.set_hidden_character(game.hidden_character());
        let problems = self.renderer.sync(&world, game.heights());
        let fresh: Vec<String> = problems
            .iter()
            .filter(|p| !self.problems.contains(p))
            .cloned()
            .collect();
        self.problems = problems;
        self.renderer.set_effects(&game.effects());
        self.renderer
            .set_crosshair(game.view() == ViewMode::FirstPerson && game.player().is_some());
        let (width, height) = self.renderer.size();
        let camera = game.camera(f64::from(width) / f64::from(height));
        self.renderer
            .draw(&camera, game.time_s(), target, encoder)?;
        Ok(fresh)
    }

    /// How many times the scene was built: a walk moves characters without a rebuild.
    pub fn rebuild_count(&self) -> u64 {
        self.renderer.rebuild_count()
    }
}
