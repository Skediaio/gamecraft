# Projectiles, ballistics, damage and simple behaviour

The mechanisms prototype P2 (a skirmish: a small map, one rifle, a few enemies) needs. Status:
designed 2026-11-18. The rifle, its cartridge and the enemies are pack data; everything here
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

## Open

- Penetration of thin shapes, ricochets, materials (later, with physics, M4).
- Animation of aiming and firing (M6).
- Sound of shots (M8).
