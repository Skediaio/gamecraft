# Projectiles, ballistics, damage and simple behaviour

The mechanisms prototype P2 (a skirmish: a small map, one rifle, a few enemies) needs. Status:
built 2026-11-18 (see *As built*). The rifle, its cartridge and the enemies are pack data; everything here
names no weapon (AGENTS.md rule 16).

## Projectiles

A projectile is a point mass flown by the simulation, not an entity of the document: it lives
only while playing, it is part of the state hash, and `sim.state` lists the ones in flight.

- **Launcher** component on any entity: `launcher {muzzle_velocity_mps, mass_kg, ballistic_coefficient (G7, kg/m2 form: mass / (form factor x reference area)), dispersion_mrad, rate_s, magazine, reload_s, damage_per_joule, muzzle_height_m}`.
  A pack fills it in for a rifle, a bow, a thrown stone or a mortar. Firing is an input
  (`sim.input {fire: true}`, held fires at `rate_s`) or a command for anything that is not a
  character (`sim.fire {entity, yaw_deg, pitch_deg}`). Dispersion is drawn from the world's
  seeded random stream forked by the launcher's entity id and shot number, so it is
  deterministic.
- **Flight**, per substep of the tick (the tick is cut into `PROJECTILE_SUBSTEPS` so a shot at
  900 m/s moves at most about 2 m per substep):
  - the air: density from the height above sea level (the barometric formula for the
    international standard atmosphere, adjusted by `environment.temperature_c`), and the
    wind as a vector from `environment.wind_direction_deg` and `wind_speed_mps`, stronger with
    height above the ground (a logarithmic profile with a roughness length);
  - drag: `a = -(rho / rho0) * v_rel * |v_rel| * Cd(Mach) / (BC)` with `v_rel` the velocity
    relative to the air and `Cd(Mach)` the G7 reference drag function (a published table,
    interpolated linearly), integrated with a second-order scheme;
  - gravity.
  Rain and mist do not move a projectile; they shorten what characters see (below).
- **Hits**: the segment flown in a substep is tested against the terrain, every shape and every
  character (a capsule from feet to head). The first hit ends the flight and produces a `hit`
  event `{projectile, shooter, target?, point, normal, speed_mps, energy_j}`; a projectile that
  falls below the ground, leaves the terrain by more than its size or flies longer than
  `max_flight_s` ends without one.

Oracles: in a vacuum (density 0) the flight is the closed-form parabola; with drag and no
wind the drop and the time of flight at 100 m steps match a published G7 table for one
documented reference projectile within tolerances stated in the test; a crosswind drifts the
shot downwind, more at longer range; a shot fired straight up comes down; same seed, same
hash.

## Health and damage

`health {max, current, armor}`: a hit takes `energy_j * damage_per_joule - armor` (at least 0)
from `current`. Reaching 0 produces a `down` event and the character stops taking input. What
"down" means for a game (respawn, lose, a body that can be looted) is the pack's triggers.

## Perception and behaviour

Enough for enemies that patrol and react; navigation proper is M7.

- **Seeing**: a character sees another when it is within `view_range_m`, within its field of
  view, and `query.line_of_sight` from eye to eye (and to the chest) is clear. The range
  shrinks with the light (night, sun elevation), with mist and rain (the same extinction the
  renderer uses for its fog, so what a player cannot see in the picture an enemy cannot see
  either) and with the target's cover in vegetation (later).
- **Hearing**: a shot produces a `heard` event for every character within a range that falls
  with distance.
- **Behaviour** component: `behaviour {mode: "idle" | "patrol" | "guard", route: [[x, z], ...], team, engage_range_m, aim_error_mrad, reaction_s}`.
  A small state machine run by the simulation: patrol walks the route; on seeing a character
  of another team it turns, waits its reaction time and fires at it with its aim error; on
  hearing a shot it turns towards the sound and searches there; it returns to its route when
  it has seen nothing for a while. Its decisions are made from the same input records a player
  uses, so an agent can take over any enemy by sending input.

## Skill, detection and cover

Hard because they notice you, react and use cover; fair because their aim is fallible. The
split follows the tactical shooters players tune: *skill* (perception, reaction, tactics) and
*precision* (aim only), as numbers on the behaviour; named presets are a pack's data
(`crates/gamecraft-sim/src/ai.rs` has the formulas and constants).

- **Skill and precision** (0 to 1, default 1): skill multiplies reaction, spot time and the
  pauses in cover by `1 + (1 - skill)`; precision multiplies the aim error by
  `1 + 3 * (1 - precision)`, stress by `1 + 2 * stress`.
- **Gradual detection**: knowledge of every enemy, 0 to 1, rises while it is seen at
  `gain / (spot_time_s * time factor / alert)` per second (`gain`: half the effective range
  over the distance, between 0.25 and 4, times 1.5 when the enemy moves and 2 when it fires;
  alert 1, 1.5 or 2 for safe, aware, combat) and falls by 1 per 60 s unseen; the last known
  place stays. It reacts from 0.25 and fires from 0.6. Darkness, mist and rain shorten the
  effective range and so slow it. Vegetation does not count yet.
- **Alertness**: safe, aware (a shot heard, an enemy glimpsed), combat (contact, or the stress
  of near misses); `calm_s` without contact outside a fight calms it one step, never below its
  behaviour's `alertness`.
- **Aim settling**: the aim error falls from `aim_error_mrad` to `aim_error_floor_mrad` over
  `aim_settle_s` while the same target stays in view.
- **Cover** (`cover_search_m` above 0, in combat): a position picker queried every 30 ticks
  (skill scales it), at most 64 candidates (where it stands, eight points around every solid
  near it, two rings of 12), scored by protection from the known threats (line of sight to the
  chest blocked), the length of the way there (straight or round one or two corners), a peek
  point from which it can fire at the first threat, and spacing from friends. It runs there,
  stays down 2 s, peeks and fires 2.5 s, and goes back down. `query.cover` is the same query.
- **Suppression** (light): a projectile of someone else passing within 3 m of the chest adds
  `0.35 * (1 - courage) * (1 - distance / 3)` stress (recovering 0.1 per second); from 0.3 it
  is in combat and takes cover, from 0.6 it does not peek.
- With all these numbers at their defaults a behaviour decides exactly as before them, and
  the state hash leaves out what it knows and feels.

## Open

- Penetration of thin shapes, ricochets, materials (later, with physics, M4).
- Animation of aiming and firing (M6).
- Sound of shots (M8).

## As built (2026-11-18)

- Drag is `a = -rho * Cd_G7(M) / (2 * BC) * |v_rel| * v_rel`, with BC in kg/m2 (mass over form
  factor times reference area; a G7 BC in lb/in2 times 703.07 gives kg/m2, and the usual
  figure times 895.2 gives this formula's value with the reference-area constant folded in).
  Against a published reference trajectory (.308, 168 gr, G7 BC 0.223, 2750 ft/s) the time of
  flight is within 0.5 %, the velocity within 1 % and the path within 6 in at 1000 yd.
- The launcher also carries `max_flight_s` and `sound_range_m`; the behaviour also carries
  `view_range_m`, `fov_deg` and `search_s`.
- Event roles: for `hit` and `down`, `who` is the shooter and `target` the one hit; for
  `heard`, `who` is the listener and `target` the shooter.
- A character's team is its behaviour's `team`, else a `team:<name>` tag; a character without a
  team is never engaged.
- Sight range is `min(view_range * light, 3.912 / (mist + rain extinction))`, with light 0.15 at
  night (sun at or below -12 degrees) and 1 by day (+6 degrees and above), linear between.
- Wind is given at 10 m above the ground and follows a logarithmic profile with a roughness
  length of 0.03 m.
- Every shape stops a shot, solid or not; a shooter cannot hit itself in the first 0.25 s; the
  shooter's own motion is not added to the projectile.
- The ray maths lives in `gamecraft-sim/src/ray.rs`, shared by projectiles, sight and the
  engine's `query.raycast` and `query.line_of_sight`.

## Handling: fire modes, reloads, recoil, sway, sights (2026-11-21)

Mechanisms on the same `launcher`, numbers as data (`gamecraft-sim/src/handling.rs` has the
exact rules):

- **Selector**: `modes` (one to four of `safe`, `semi`, `auto`, `burst`, in switch order; the
  first at the start), `burst_count`, `cyclic_rpm` for `auto` and `burst` (else `rate_s`);
  `semi` fires once per pull and at most every `rate_s`. The input's `fire` is the trigger,
  `switch_mode` moves the selector, `sim.input {fire_mode}` sets it. Left out, `modes` is
  `["auto"]` at `rate_s`: a launcher fires as it always did, so behaviours that hold `fire`
  keep firing.
- **Reload**: the input's `reload` takes `reload_s`; from empty `reload_empty_s` (else
  `reload_s`).
- **Magazines keep their rounds**: `spare_magazines` (a count of full spares, 0 to 100000)
  fills a pouch where each magazine has its own round count; `null` or absent is unlimited full
  magazines, as a launcher always was (the partly used one is lost, the state hash unchanged).
  A reload, asked for or after the last shot, puts the launcher's magazine back in the pouch (an
  empty one is dropped) and fits the fullest one (ties are alike, so the choice is
  deterministic); the launcher holds no rounds while it is fitted (`magazine.left` is 0,
  `loading` the rounds coming) and has them when `reload_s` (from empty `reload_empty_s`) ends.
  A reload does nothing when no magazine in the pouch holds more rounds than the launcher does.
  `sim.state` and `entity.get` show `magazine.pouch` (rounds of each magazine, fullest first)
  and `spare` (their count); both null for unlimited. The pouch is hashed only once it holds a
  used magazine, so a pouch of full ones hashes as the count did. The window tells it as `NO
  MAGAZINES`, never as a round count.
- **No automatic reload**: `auto_reload` (default true, as before). False: the empty launcher
  clicks, firing does nothing until the input's `reload`; a behaviour whose launcher is dry
  sets `reload` by itself, as a player would.
- **A reload takes the weapon off the sights**: while reloading, `handling.ads` is false
  whatever the input's `ads` holds (sway and dispersion are the hip's), and `handling.ads_amount`
  (0 hip to 1 sights) goes down at `1 / ads_time_s` a second; when the reload ends a still held
  `ads` brings it up again. Nothing fires while reloading. `handling.reloading` and
  `handling.reload_progress` (0 to 1) tell it for the view model.
- **Recoil** `{kick_up_mrad, kick_side_mrad, recovery_s}`: a kick per shot on top of the aim,
  decaying exponentially (5 % left after `recovery_s`); fast shots stack. The window shows it
  as a climbing view the player pulls back with the mouse: the input's aim is the player's,
  the offset is added on top.
- **Sway** `{standing_mrad, crouched_mrad, prone_mrad, moving_factor, fatigue_factor,
  fatigue_build_s, fatigue_fade_s, breath_hold_s, breath_steady_factor, breath_recover_s,
  breath_recover_factor, period_s}`: a seeded sum of three sines; characters have no stances
  yet, so every one stands (`handling::stance_of` is the hook).
- **Sights**: the input's `ads` multiplies sway and dispersion by `ads_sway_factor` and
  `ads_dispersion_factor`; `held_model {ads_offset_m, ads_fov_factor, ads_time_s}` moves the
  view model to the sights and narrows the view in the window. Its `attachments` (an optic)
  ride on it, and its `reticle` is a collimated dot: at infinity along the barrel, so it shows
  the aim with its sway and recoil and wanders within the optic while the camera stays.
- **Muzzle**: `held_model.muzzle_m` is the barrel's mouth in the model's own axes `[right, up,
  forward]` (metres before `scale`; forward is glTF's -Z, the way the model points, so a barrel
  along -Z has a positive forward). With it, a character's shot starts at the eye plus the
  model's offset at the current sights blend (`offset_m` towards `ads_offset_m` by
  `handling.ads_amount`) plus the muzzle times `scale`, turned by the aim with sway and recoil,
  exactly as the view model is placed (`HeldModel::muzzle_point`, `AimAxes`). The bore is
  zeroed once per weapon: with the model at the sights, the barrel is tilted relative to the aim
  (up and sideways) so a shot crosses the line of sight at `zero_m` (default 100 m) in still
  standard air; the same tilt holds at the hip. The tilt is solved with the firing solution of
  `aim::solve` (drag and gravity, to a millimetre), not a flat angle. Without `muzzle_m`, and
  for characters without a held model (the AI), the shot leaves from the eye as before. With a muzzle the bullet also carries the shooter's own velocity (the muzzle velocity is relative to the rifle); the shot starts at the muzzle of the tick it is fired in (bodies move before launchers fire). A
  muzzle pushed through a wall (leaning on it) is not detected yet.
- Every handling field is left out of a document while at its default, so old worlds keep
  their text and their state hashes.

A 5.56 service rifle in these terms (research of 2026-11-21: published figures, and tuning
values to settle in play): `muzzle_velocity_mps` 870, `mass_kg` 0.004, `dispersion_mrad` 0.5,
`rate_s` 0.1, `magazine` 30, `reload_s` 2.5, `reload_empty_s` 3.5, `modes` `["safe", "semi",
"auto"]`, `cyclic_rpm` 650, `recoil` 1.5 / 0.6 mrad over 0.25 s, `sway` 2 / 1.2 / 0.5 mrad, a
fatigue factor of 2.5 fading over 10 s. The research gives the M855 ball's G1 coefficient
(about 0.30 lb/in2); this engine's drag is G7, whose figure for that bullet is about 0.15
lb/in2, so `ballistic_coefficient` is about 135 (times 895.2), not 211.
