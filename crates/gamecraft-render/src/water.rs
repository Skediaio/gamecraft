//! Water in the picture: the `water` components of a world as VisionCraft's water surface
//! (`visioncraft_render::Sea`), which there is one of per scene.
//!
//! The surface drawn is the water that is everywhere (the sea; the first such in id order),
//! else the largest area by its outline (the first of equal size in id order). The other
//! areas are not drawn and are named in the problems: a known gap, for VisionCraft. Where the
//! simulation says there is water and how deep ([`gamecraft_world::water`]) is independent of
//! what is drawn, and the outline's even-odd rule is the renderer's own.
//!
//! The waves follow the wind as a fully developed sea does (Pierson-Moskowitz: a significant
//! wave height of 0.21 U²/g for a wind of U m/s), at most sea state 6 (5 m); the peak
//! wavelength is 39 times the height (at least 3 m). An outlined area is inland water (a lake,
//! a wide river): little fetch, so its waves stop at 1 m and are 14 times their height long (at
//! least 1.5 m), as in VisionCraft's own model (`visioncraft-core/src/water.rs`, MIT), and
//! still water freezes when the air is below 0 degrees. The colour is open coastal water's
//! deep blue, the waves travel with the wind, foam as for the sea state.

use crate::light::wind_towards;
use gamecraft_world::{Environment, WaterArea, World, water_areas};
use visioncraft_render as vc;

/// Gravity, m/s2, in the wave formulas.
const GRAVITY_MPS2: f32 = 9.81;
/// The share of the wind's squared speed that is a fully developed sea's significant wave
/// height (Pierson-Moskowitz).
const WAVE_HEIGHT_PER_WIND_SQ: f32 = 0.21;
/// The most significant wave height drawn: WMO sea state 6.
const SEA_STATE_MAX_M: f32 = 5.0;
/// The most significant wave height of inland water (little fetch).
const INLAND_WAVE_MAX_M: f32 = 1.0;
/// Peak wavelength per metre of wave height: a fully developed sea's is about 39 Hs.
const SEA_WAVELENGTH_PER_HEIGHT: f32 = 39.0;
/// Shortest peak wavelength drawn at sea, metres.
const SEA_WAVELENGTH_MIN_M: f32 = 3.0;
/// Peak wavelength per metre of wave height of inland water: short, steep waves.
const INLAND_WAVELENGTH_PER_HEIGHT: f32 = 14.0;
/// Shortest peak wavelength drawn inland, metres.
const INLAND_WAVELENGTH_MIN_M: f32 = 1.5;
/// Linear RGB of deep water: open coastal water on a clear day (VisionCraft's default).
const DEEP_WATER_COLOR: [f32; 3] = [0.004, 0.026, 0.075];
/// How clear the water is, 0 (murky) to 1 (clear): the renderer's turbidity is 1 minus it.
const CLARITY: f32 = 0.75;
/// Foam on the crests and along the shore, 0 to 1.
const FOAM: f32 = 0.5;
/// Air temperature from which still inland water freezes, degrees Celsius.
const FREEZING_C: f64 = 0.0;
/// Significant wave height (WMO sea state 1) below which water counts as still.
const STILL_WAVE_M: f32 = 0.1;

/// What a world's water becomes in the scene.
pub(crate) struct WaterPlan {
    /// The surface to draw; `None` without water.
    pub sea: Option<vc::Sea>,
    /// The water areas not drawn, as sentences.
    pub problems: Vec<String>,
}

/// The area to draw: water everywhere (the first in id order), else the largest by its
/// outline (the first of equal size in id order).
fn drawn(areas: &[WaterArea]) -> Option<&WaterArea> {
    let everywhere = areas.iter().find(|a| a.outline.is_empty());
    if everywhere.is_some() {
        return everywhere;
    }
    let mut best: Option<&WaterArea> = None;
    for area in areas {
        let better = best.is_none_or(|b| area.signed_area_m2().abs() > b.signed_area_m2().abs());
        if better {
            best = Some(area);
        }
    }
    best
}

/// The water surface of `world` under `env`: what to draw, and what is left out.
pub(crate) fn plan(world: &World, env: &Environment) -> WaterPlan {
    let areas = water_areas(world);
    let Some(sea) = drawn(&areas) else {
        return WaterPlan {
            sea: None,
            problems: Vec::new(),
        };
    };
    let name = |id: gamecraft_world::EntityId| {
        world
            .entity(id)
            .filter(|e| !e.name.is_empty())
            .map_or_else(|| format!("entity {id}"), |e| format!("\"{}\"", e.name))
    };
    let problems = areas
        .iter()
        .filter(|a| a.entity != sea.entity)
        .map(|a| {
            format!(
                "Water {} ({}) is not drawn: the renderer draws one water surface, {}'s; the \
                 others are in the simulation and the queries only",
                name(a.entity),
                a.entity,
                name(sea.entity)
            )
        })
        .collect();
    WaterPlan {
        sea: Some(surface(sea, env)),
        problems,
    }
}

/// VisionCraft's water surface of `area` under the environment.
fn surface(area: &WaterArea, env: &Environment) -> vc::Sea {
    let inland = !area.outline.is_empty();
    let wind = if env.wind_speed_mps.is_finite() {
        (env.wind_speed_mps.max(0.0) as f32).min(100.0)
    } else {
        0.0
    };
    let full = WAVE_HEIGHT_PER_WIND_SQ * wind * wind / GRAVITY_MPS2;
    let (height, per_height, min_wavelength) = if inland {
        (
            full.min(INLAND_WAVE_MAX_M),
            INLAND_WAVELENGTH_PER_HEIGHT,
            INLAND_WAVELENGTH_MIN_M,
        )
    } else {
        (
            full.min(SEA_STATE_MAX_M),
            SEA_WAVELENGTH_PER_HEIGHT,
            SEA_WAVELENGTH_MIN_M,
        )
    };
    let [towards_x, _, towards_z] = wind_towards(env.wind_direction_deg);
    vc::Sea {
        level: area.level as f32,
        wave_height_m: height,
        peak_wavelength_m: (per_height * height).max(min_wavelength),
        direction: [towards_x as f32, towards_z as f32],
        color: DEEP_WATER_COLOR,
        turbidity: 1.0 - CLARITY,
        flow: [0.0; 2],
        foam: FOAM,
        freezes: inland && height <= STILL_WAVE_M && env.temperature_c <= FREEZING_C,
        outline: area
            .outline
            .iter()
            .map(|p| [p[0] as f32, p[1] as f32])
            .collect(),
        coast: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamecraft_world::{Component, EntityDraft, Water};

    fn env(wind: f64, temperature: f64) -> Environment {
        Environment {
            wind_speed_mps: wind,
            temperature_c: temperature,
            ..Environment::default()
        }
    }

    fn pond(name: &str, level: f64, outline: Vec<[f64; 2]>) -> EntityDraft {
        EntityDraft {
            name: name.into(),
            position: [0.0, level, 0.0],
            components: vec![Component::Water(Water::new(0.0, outline))],
            ..EntityDraft::default()
        }
    }

    #[test]
    fn without_water_nothing_is_drawn() {
        let world = World::new("dry", 1);
        let planned = plan(&world, &env(3.0, 15.0));
        assert!(planned.sea.is_none() && planned.problems.is_empty());
    }

    #[test]
    fn one_area_is_the_surface_with_waves_from_the_wind() {
        let mut world = World::new("sea", 1);
        world
            .add_entity(EntityDraft {
                name: "Sea".into(),
                position: [0.0, -2.5, 0.0],
                components: vec![Component::Water(Water::default())],
                ..EntityDraft::default()
            })
            .unwrap();
        let planned = plan(&world, &env(3.0, 15.0));
        let sea = planned.sea.as_ref().expect("a surface");
        assert!(planned.problems.is_empty());
        assert!((sea.level + 2.5).abs() < 1e-6);
        assert!(
            (sea.wave_height_m - 0.21 * 9.0 / 9.81).abs() < 1e-6,
            "{}",
            sea.wave_height_m
        );
        assert!((sea.peak_wavelength_m - 39.0 * sea.wave_height_m).abs() < 1e-4);
        assert!(sea.outline.is_empty() && !sea.freezes);
        // A gale is cut at sea state 6; glassy calm keeps 3 m waves of no height.
        let gale = plan(&world, &env(30.0, 15.0)).sea.unwrap();
        assert_eq!(gale.wave_height_m, SEA_STATE_MAX_M);
        let calm = plan(&world, &env(0.0, 15.0)).sea.unwrap();
        assert_eq!(
            (calm.wave_height_m, calm.peak_wavelength_m),
            (0.0, SEA_WAVELENGTH_MIN_M)
        );
        assert!(!calm.freezes, "the sea never freezes");
    }

    #[test]
    fn inland_water_has_short_waves_freezes_when_still_and_cold_and_the_largest_is_drawn() {
        let mut world = World::new("lakes", 1);
        let small = vec![[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]];
        let large = vec![[-10.0, -10.0], [10.0, -10.0], [10.0, 10.0], [-10.0, 10.0]];
        world
            .add_entity(pond("Small pond", 12.0, small.clone()))
            .unwrap();
        let big = world.add_entity(pond("Lake", 3.0, large.clone())).unwrap();
        // An equal twin of the lake, later in id order: the first of equal size is drawn.
        world
            .add_entity(pond("Lake twin", 4.0, large.clone()))
            .unwrap();
        let planned = plan(&world, &env(10.0, -5.0));
        let sea = planned.sea.as_ref().expect("a surface");
        assert_eq!(sea.level, 3.0);
        assert_eq!(sea.outline.len(), 4);
        assert_eq!(sea.wave_height_m, INLAND_WAVE_MAX_M, "little fetch");
        assert!((sea.peak_wavelength_m - 14.0).abs() < 1e-5);
        assert!(!sea.freezes, "rough water does not freeze");
        let still = plan(&world, &env(1.0, -5.0)).sea.unwrap();
        assert!(still.freezes, "still and freezing");
        assert_eq!(planned.problems.len(), 2, "{:?}", planned.problems);
        assert!(
            planned.problems[0].contains("\"Small pond\"")
                && planned.problems[1].contains("\"Lake twin\""),
            "{:?}",
            planned.problems
        );
        // Water everywhere wins over every lake, whichever came first.
        world.add_entity(pond("Sea", 0.0, Vec::new())).unwrap();
        let planned = plan(&world, &env(1.0, 15.0));
        assert!(planned.sea.as_ref().unwrap().outline.is_empty());
        assert_eq!(planned.problems.len(), 3);
        // A warm breeze on the lake: no ice, waves below the cap.
        let _ = big;
        let mut dry = World::new("one lake", 1);
        dry.add_entity(pond("Lake", 3.0, large)).unwrap();
        let sea = plan(&dry, &env(5.0, 20.0)).sea.unwrap();
        assert!(!sea.freezes);
        assert!((sea.wave_height_m - 0.21 * 25.0 / 9.81).abs() < 1e-6);
    }

    #[test]
    fn hostile_numbers_give_a_surface_that_can_be_drawn() {
        let mut world = World::new("odd", 1);
        world
            .add_entity(pond("Odd", 0.0, vec![[0.0; 2], [1.0, 0.0], [0.0, 1.0]]))
            .unwrap();
        for env in [
            env(f64::NAN, f64::NAN),
            env(-5.0, f64::MAX),
            env(1e300, 15.0),
        ] {
            let sea = plan(&world, &env).sea.unwrap();
            assert!(
                sea.level.is_finite()
                    && sea.wave_height_m.is_finite()
                    && sea.peak_wavelength_m.is_finite()
                    && sea.direction.iter().all(|c| c.is_finite()),
                "{sea:?}"
            );
        }
    }
}
