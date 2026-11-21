//! How a launcher handles: fire modes, the cyclic rate, reloads, recoil and sway.

use crate::{Component, EntityDraft, FireMode, Launcher, Recoil, Sway, World};

/// The research's 5.56 service rifle (`docs/design/ballistics.md`): selector, cyclic rate,
/// reloads, recoil and sway.
fn service_rifle() -> Launcher {
    Launcher {
        muzzle_velocity_mps: 870.0,
        mass_kg: 0.004,
        ballistic_coefficient: 211.0,
        dispersion_mrad: 0.5,
        rate_s: 0.1,
        magazine: 30,
        reload_s: 2.5,
        modes: vec![FireMode::Safe, FireMode::Semi, FireMode::Auto],
        cyclic_rpm: Some(650.0),
        reload_empty_s: Some(3.5),
        recoil: Recoil {
            kick_up_mrad: 1.5,
            kick_side_mrad: 0.6,
            recovery_s: 0.25,
        },
        sway: Sway {
            standing_mrad: 2.0,
            crouched_mrad: 1.2,
            prone_mrad: 0.5,
            ..Sway::default()
        },
        ..Launcher::default()
    }
}

#[test]
fn a_launcher_without_the_handling_fields_fires_as_before() {
    let c: Component = serde_json::from_str(r#"{"type": "launcher", "rate_s": 0.2}"#).unwrap();
    let Component::Launcher(l) = c else { panic!() };
    assert_eq!(l.modes, vec![FireMode::Auto]);
    assert_eq!(l.first_mode(), FireMode::Auto);
    assert_eq!(l.interval_s(FireMode::Auto), 0.2);
    assert_eq!(l.empty_reload_s(), l.reload_s);
    assert_eq!(l.recoil.kick_up_mrad, 0.0);
    assert_eq!(l.sway.standing_mrad, 0.0);
    assert_eq!(l.burst_count, 3);
}

/// A launcher at the handling defaults is written as before they existed: old worlds keep
/// their text, and so their state hashes. The defaults are listed for the catalog.
#[test]
fn default_handling_is_not_written() {
    let text = serde_json::to_string(&Component::Launcher(Launcher::default())).unwrap();
    for field in [
        "modes",
        "burst_count",
        "cyclic_rpm",
        "reload_empty_s",
        "spare_magazines",
        "auto_reload",
        "recoil",
        "sway",
        "ads_sway_factor",
        "ads_dispersion_factor",
    ] {
        assert!(!text.contains(field), "{field} in {text}");
        assert!(
            crate::launcher_handling_defaults().get(field).is_some(),
            "{field}"
        );
    }
    let held = serde_json::to_string(&crate::HeldModel::of("models/rifle.glb")).unwrap();
    assert!(!held.contains("ads_"), "{held}");
    assert!(
        !held.contains("muzzle_m") && !held.contains("zero_m"),
        "{held}"
    );
    for field in ["muzzle_m", "zero_m"] {
        assert!(
            crate::held_model_sights_defaults().get(field).is_some(),
            "{field}"
        );
    }
    let sights = crate::held_model_sights_defaults();
    assert_eq!(sights["ads_fov_factor"], 0.85);
}

#[test]
fn a_service_rifle_round_trips_and_switches_its_modes_in_order() {
    let rifle = service_rifle();
    let text = serde_json::to_string(&Component::Launcher(rifle.clone())).unwrap();
    assert!(text.contains(r#""modes":["safe","semi","auto"]"#), "{text}");
    let back: Component = serde_json::from_str(&text).unwrap();
    assert_eq!(back, Component::Launcher(rifle.clone()));
    // 650 rounds a minute: one every 92 ms; semi waits rate_s.
    assert!((rifle.interval_s(FireMode::Auto) - 60.0 / 650.0).abs() < 1e-15);
    assert_eq!(rifle.interval_s(FireMode::Semi), 0.1);
    assert_eq!(rifle.empty_reload_s(), 3.5);
    assert_eq!(rifle.next_mode(FireMode::Safe), FireMode::Semi);
    assert_eq!(rifle.next_mode(FireMode::Auto), FireMode::Safe);
    assert_eq!(rifle.next_mode(FireMode::Burst), FireMode::Safe);
    for name in FireMode::NAMES {
        assert_eq!(FireMode::from_name(name).unwrap().name(), name);
    }
    assert_eq!(FireMode::from_name("full"), None);
}

#[test]
fn bad_handling_values_name_the_field() {
    let bad = |f: &dyn Fn(&mut Launcher)| {
        let mut w = World::new("h", 1);
        let mut l = service_rifle();
        f(&mut l);
        w.add_entity(EntityDraft {
            components: vec![Component::Launcher(l)],
            ..EntityDraft::default()
        })
        .unwrap_err()
        .to_string()
    };
    let msg = bad(&|l| l.modes.clear());
    assert!(
        msg.contains("launcher.modes") && msg.contains("one to four"),
        "{msg}"
    );
    let msg = bad(&|l| l.modes = vec![FireMode::Semi, FireMode::Semi]);
    assert!(
        msg.contains("launcher.modes") && msg.contains("twice"),
        "{msg}"
    );
    let msg = bad(&|l| l.burst_count = 0);
    assert!(msg.contains("launcher.burst_count"), "{msg}");
    let msg = bad(&|l| l.cyclic_rpm = Some(0.0));
    assert!(msg.contains("launcher.cyclic_rpm"), "{msg}");
    let msg = bad(&|l| l.cyclic_rpm = Some(f64::NAN));
    assert!(msg.contains("launcher.cyclic_rpm"), "{msg}");
    let msg = bad(&|l| l.reload_empty_s = Some(-1.0));
    assert!(msg.contains("launcher.reload_empty_s"), "{msg}");
    let msg = bad(&|l| l.spare_magazines = Some(u32::MAX));
    assert!(msg.contains("launcher.spare_magazines"), "{msg}");
    let msg = bad(&|l| l.recoil.kick_up_mrad = f64::INFINITY);
    assert!(msg.contains("launcher.recoil.kick_up_mrad"), "{msg}");
    let msg = bad(&|l| l.recoil.recovery_s = -0.1);
    assert!(msg.contains("launcher.recoil.recovery_s"), "{msg}");
    let msg = bad(&|l| l.sway.period_s = 0.0);
    assert!(msg.contains("launcher.sway.period_s"), "{msg}");
    let msg = bad(&|l| l.sway.standing_mrad = 1000.0);
    assert!(msg.contains("launcher.sway.standing_mrad"), "{msg}");
    let msg = bad(&|l| l.sway.fatigue_fade_s = f64::NAN);
    assert!(msg.contains("launcher.sway.fatigue_fade_s"), "{msg}");
    let msg = bad(&|l| l.ads_sway_factor = 1.5);
    assert!(msg.contains("launcher.ads_sway_factor"), "{msg}");
    let msg = bad(&|l| l.ads_dispersion_factor = -0.5);
    assert!(msg.contains("launcher.ads_dispersion_factor"), "{msg}");
    // An unknown mode is refused when read.
    let err = serde_json::from_str::<Component>(r#"{"type": "launcher", "modes": ["full"]}"#)
        .unwrap_err()
        .to_string();
    assert!(err.contains("full"), "{err}");
}
