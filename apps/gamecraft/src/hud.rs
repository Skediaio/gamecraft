//! What the window shows over the world besides the crosshair, as pure functions from the
//! game's state to strings and colours: the ammunition (no round count, as in a military
//! sandbox: a fire mode, a rough magazine state and the spare magazines, shown for a few
//! seconds after something happens to the weapon), the red flash of a hit and the death
//! screen. [`build`] puts them into the renderer's [`Hud`].

use gamecraft_render::hud::{Hud, HudLine};

/// How long the ammunition stays up after the last shot, a reload, a mode switch or a check,
/// seconds.
pub const AMMO_SHOW_S: f64 = 3.0;
/// How long the red flash of a hit lasts, seconds.
pub const HIT_FLASH_S: f64 = 0.4;
/// How long the screen takes to fade to the death screen, seconds.
pub const DEATH_FADE_S: f64 = 1.2;
/// The height of the ammunition's glyphs, physical pixels.
pub const AMMO_GLYPH_PX: f64 = 18.0;
/// The height of the death screen's headline, physical pixels.
pub const DEAD_GLYPH_PX: f64 = 72.0;
/// The height of the death screen's other lines, physical pixels.
pub const DEAD_SMALL_GLYPH_PX: f64 = 24.0;

/// The ammunition's text colour.
const AMMO_RGBA: [u8; 4] = [255, 255, 255, 240];
/// The colour of a warning in the ammunition (empty, no magazines).
const WARN_RGBA: [u8; 4] = [255, 150, 90, 255];
/// The death screen's headline.
const DEAD_RGBA: [u8; 4] = [235, 50, 40, 255];
/// The death screen's other lines.
const DEAD_SMALL_RGBA: [u8; 4] = [240, 225, 220, 255];
/// The tint of the death screen: dark and red, its alpha at the end of the fade.
const DEAD_VEIL: [u8; 4] = [30, 3, 3, 210];
/// The red at the edges of the death screen, its alpha at the end of the fade.
const DEAD_VIGNETTE: [u8; 4] = [140, 0, 0, 170];
/// The red at the edges at the moment of a hit.
const HIT_VIGNETTE: [u8; 4] = [190, 0, 0, 150];

/// The state of a launcher's magazine for the HUD.
#[derive(Clone, Debug, PartialEq)]
pub struct Ammo {
    /// The selector: `safe`, `semi`, `auto` or `burst`.
    pub mode: String,
    /// Rounds left in the magazine.
    pub left: u32,
    /// Rounds of a full magazine.
    pub capacity: u32,
    /// Spare magazines; `None` is unlimited.
    pub spare: Option<u32>,
    /// Whether a reload is running.
    pub reloading: bool,
}

/// Who killed the player, and how far away.
#[derive(Clone, Debug, PartialEq)]
pub struct Killer {
    /// The shooter's name.
    pub name: String,
    /// The distance when the shot hit, metres.
    pub distance_m: f64,
}

/// The death screen's state.
#[derive(Clone, Debug, PartialEq)]
pub struct Death {
    /// The shooter, if the simulation knows it.
    pub killer: Option<Killer>,
    /// Seconds since the player went down.
    pub dead_s: f64,
}

/// Everything the HUD depends on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudState {
    /// The weapon in the player's hands.
    pub ammo: Option<Ammo>,
    /// Whether the ammunition is up now.
    pub ammo_visible: bool,
    /// Seconds of the red flash left.
    pub hit_flash_s: f64,
    /// Set while the player is dead.
    pub death: Option<Death>,
}

/// A rough word for a magazine of `left` rounds out of `capacity`: `full` from three quarters
/// up, `about half` from a third, `low` from a seventh, `almost empty` for the rest, `empty`
/// for none.
pub fn magazine_word(left: u32, capacity: u32) -> &'static str {
    if left == 0 {
        return "empty";
    }
    let share = f64::from(left) / f64::from(capacity.max(1));
    if share >= 0.75 {
        "full"
    } else if share >= 0.35 {
        "about half"
    } else if share >= 0.15 {
        "low"
    } else {
        "almost empty"
    }
}

/// The ammunition's lines, top to bottom: the fire mode, the magazine (or the reload), and the
/// spare magazines (none for unlimited).
pub fn ammo_lines(ammo: &Ammo) -> Vec<String> {
    let mut lines = vec![ammo.mode.to_ascii_uppercase()];
    lines.push(if ammo.reloading {
        "RELOADING".to_string()
    } else {
        format!(
            "MAGAZINE {}",
            magazine_word(ammo.left, ammo.capacity).to_ascii_uppercase()
        )
    });
    match ammo.spare {
        None => {}
        Some(0) => lines.push("NO MAGAZINES".to_string()),
        Some(1) => lines.push("1 SPARE MAGAZINE".to_string()),
        Some(n) => lines.push(format!("{n} SPARE MAGAZINES")),
    }
    lines
}

/// The death screen's lines: the headline, what killed the player (when known) and how to go
/// on.
pub fn death_lines(killer: Option<&Killer>) -> Vec<String> {
    let mut lines = vec!["YOU ARE DEAD".to_string()];
    if let Some(k) = killer {
        let name = k.name.to_ascii_uppercase();
        lines.push(if k.distance_m.is_finite() {
            format!("KILLED BY {name} AT {:.0} M", k.distance_m)
        } else {
            format!("KILLED BY {name}")
        });
    }
    lines.push("PRESS ENTER TO RESTART".to_string());
    lines
}

/// How red the edges are `left_s` seconds before a hit's flash ends: 1 at the hit, falling
/// linearly to 0.
pub fn hit_flash(left_s: f64) -> f64 {
    if left_s.is_finite() {
        (left_s / HIT_FLASH_S).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// How far the screen has faded to the death screen after `dead_s` seconds: 0 to 1.
pub fn death_fade(dead_s: f64) -> f64 {
    if dead_s.is_finite() {
        (dead_s / DEATH_FADE_S).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// `rgba` with its alpha times `share` (0 to 1).
fn scaled(rgba: [u8; 4], share: f64) -> [u8; 4] {
    let [r, g, b, a] = rgba;
    [
        r,
        g,
        b,
        (f64::from(a) * share).round().clamp(0.0, 255.0) as u8,
    ]
}

/// The renderer's HUD for `state`.
pub fn build(state: &HudState) -> Hud {
    if let Some(death) = &state.death {
        let fade = death_fade(death.dead_s);
        let centre = death_lines(death.killer.as_ref())
            .into_iter()
            .enumerate()
            .map(|(i, text)| {
                if i == 0 {
                    HudLine::new(text, DEAD_GLYPH_PX, scaled(DEAD_RGBA, fade))
                } else {
                    HudLine::new(text, DEAD_SMALL_GLYPH_PX, scaled(DEAD_SMALL_RGBA, fade))
                }
            })
            .collect();
        return Hud {
            corner: Vec::new(),
            centre,
            veil: Some(scaled(DEAD_VEIL, fade)),
            vignette: Some(scaled(DEAD_VIGNETTE, fade)),
        };
    }
    let flash = hit_flash(state.hit_flash_s);
    let mut hud = Hud {
        vignette: (flash > 0.0).then(|| scaled(HIT_VIGNETTE, flash)),
        ..Hud::default()
    };
    if state.ammo_visible
        && let Some(ammo) = &state.ammo
    {
        hud.corner = ammo_lines(ammo)
            .into_iter()
            .map(|text| {
                let warn = text == "NO MAGAZINES" || text.ends_with("EMPTY");
                let rgba = if warn { WARN_RGBA } else { AMMO_RGBA };
                HudLine::new(text, AMMO_GLYPH_PX, rgba)
            })
            .collect();
    }
    hud
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn ammo(left: u32, spare: Option<u32>, reloading: bool) -> Ammo {
        Ammo {
            mode: "auto".into(),
            left,
            capacity: 30,
            spare,
            reloading,
        }
    }

    #[test]
    fn a_magazine_is_told_roughly_never_in_rounds() {
        let words: Vec<&str> = [30, 27, 23, 22, 15, 11, 10, 5, 4, 1, 0]
            .iter()
            .map(|&n| magazine_word(n, 30))
            .collect();
        assert_eq!(
            words,
            [
                "full",
                "full",
                "full",
                "about half",
                "about half",
                "about half",
                "low",
                "low",
                "almost empty",
                "almost empty",
                "empty"
            ]
        );
        assert_eq!(
            magazine_word(1, 0),
            "full",
            "a zero capacity does not divide"
        );
        // 29 rounds left never shows as a number anywhere.
        let text = ammo_lines(&ammo(29, None, false)).join(" ");
        assert!(!text.contains("29"), "{text}");
    }

    #[test]
    fn the_ammunition_lines_show_the_mode_the_magazine_and_the_spares() {
        assert_eq!(
            ammo_lines(&ammo(14, Some(3), false)),
            ["AUTO", "MAGAZINE ABOUT HALF", "3 SPARE MAGAZINES"]
        );
        assert_eq!(
            ammo_lines(&ammo(30, Some(1), false)),
            ["AUTO", "MAGAZINE FULL", "1 SPARE MAGAZINE"]
        );
        assert_eq!(
            ammo_lines(&ammo(0, Some(0), false)),
            ["AUTO", "MAGAZINE EMPTY", "NO MAGAZINES"]
        );
        assert_eq!(
            ammo_lines(&ammo(5, Some(2), true)),
            ["AUTO", "RELOADING", "2 SPARE MAGAZINES"]
        );
        // Unlimited magazines say nothing about spares.
        assert_eq!(ammo_lines(&ammo(30, None, false)).len(), 2);
    }

    #[test]
    fn the_death_screen_says_what_killed_the_player() {
        let killer = Killer {
            name: "Grunt".into(),
            distance_m: 123.4,
        };
        assert_eq!(
            death_lines(Some(&killer)),
            [
                "YOU ARE DEAD",
                "KILLED BY GRUNT AT 123 M",
                "PRESS ENTER TO RESTART"
            ]
        );
        assert_eq!(
            death_lines(None),
            ["YOU ARE DEAD", "PRESS ENTER TO RESTART"]
        );
        let far = Killer {
            name: "x".into(),
            distance_m: f64::NAN,
        };
        assert_eq!(death_lines(Some(&far))[1], "KILLED BY X");
    }

    #[test]
    fn the_hud_shows_the_ammunition_only_while_it_is_up() {
        let mut state = HudState {
            ammo: Some(ammo(30, Some(2), false)),
            ..HudState::default()
        };
        assert!(build(&state).is_empty(), "hidden by default");
        state.ammo_visible = true;
        let hud = build(&state);
        assert_eq!(hud.corner.len(), 3);
        assert!(hud.corner.iter().all(|l| l.glyph_px == AMMO_GLYPH_PX));
        state.ammo = None;
        assert!(build(&state).is_empty(), "no weapon, no ammunition");
    }

    #[test]
    fn a_hit_flashes_the_edges_red_and_fades() {
        let hit = |left_s| {
            build(&HudState {
                hit_flash_s: left_s,
                ..HudState::default()
            })
        };
        assert_eq!(
            hit(HIT_FLASH_S).vignette.map(|v| v[3]),
            Some(HIT_VIGNETTE[3])
        );
        let half = hit(HIT_FLASH_S / 2.0).vignette.unwrap();
        assert!(half[3] > 0 && half[3] < HIT_VIGNETTE[3]);
        assert_eq!(hit(0.0).vignette, None);
        assert_eq!(hit(f64::NAN).vignette, None);
    }

    #[test]
    fn the_death_screen_fades_in_and_replaces_the_ammunition() {
        let death = |dead_s| HudState {
            ammo: Some(ammo(30, Some(2), false)),
            ammo_visible: true,
            hit_flash_s: 0.3,
            death: Some(Death {
                killer: None,
                dead_s,
            }),
        };
        let start = build(&death(0.0));
        assert_eq!(start.veil.map(|v| v[3]), Some(0));
        let end = build(&death(DEATH_FADE_S * 5.0));
        assert_eq!(end.veil, Some(DEAD_VEIL));
        assert_eq!(end.vignette, Some(DEAD_VIGNETTE));
        assert!(end.corner.is_empty(), "no ammunition while dead");
        assert_eq!(end.centre.len(), 2);
        assert_eq!(end.centre[0].text, "YOU ARE DEAD");
        assert!(end.centre[0].glyph_px > end.centre[1].glyph_px);
        let mid = build(&death(DEATH_FADE_S / 2.0)).veil.unwrap();
        assert!(mid[3] > 0 && mid[3] < DEAD_VEIL[3]);
    }
}
