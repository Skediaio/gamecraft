# Water

Where a world has water, how deep it is, what it does to a character, and how it is drawn.
Status: built 2026-11-26 (roadmap item 10). Decisions in `decisions.md` (*Water is a component:
an outline and a level*).

## The recipe

An entity with a `water` component is an area of water: a flat surface at one level over an
outline of the ground plane, or over everything (the sea out to the horizon).

```json
{"type": "water", "level_m": 0.25, "outline": [[-5, -5], [5, -5], [5, 5], [-5, 5]]}
```

- `level_m`: the height of the surface above the entity's position, metres. Default 0, written
  only when it is not 0.
- `outline`: the outline of the area, `[x, z]` offsets from the entity's position (turned by its
  `rotation_y_deg`, times its scale), as a road's points are: 3 to 64 points, consecutive
  points (the last and the first included) at least 0.01 m apart. Left out: water everywhere.

`water.add {name, level_m, outline, layer}` takes the level as a world height and the outline in
world metres, and places the entity at the outline's centre on the surface; `water.set {id,
level_m?, outline?}` changes either (the level moves the surface, the outline stays in the
entity's frame) or gives water to an entity without one. `entity.add` and
`entity.set_component` take the component as given. Water is removed with `entity.remove` or
`entity.remove_component {type: "water"}`.

Nothing else is stored. **A point is under water** when an area covers it (the even-odd rule over
the outline, the renderer's own rule, so the picture and the simulation agree) and the ground
there lies below the surface; the depth is the surface minus the ground. Where areas overlap the
highest surface counts (a lake in a flooded plain is the lake). All of it is `f64` with `+`, `-`,
`*` and `/` only (`gamecraft_world::water`).

`environment.sea_level_m` is not a surface: it says how high world y = 0 is above sea level, for
the density of the air projectiles fly through (`ballistics.md`). A map whose ground is in metres
above sea level has its real sea at y = -`sea_level_m`, and that sea is a `water` entity placed
there.

## Wading

The simulation reads the water areas of the running world (in id order, rebuilt when a command
changes them, as the solids are) and, for each character, the depth at its feet
(`gamecraft_sim::water`):

- **Up to 0.5 m** (`WADE_FREE_DEPTH_M`, knee-deep): as fast as on dry land.
- **From 0.5 to 1.5 m**: the share of its speed falls linearly, to **0.4**
  (`WADE_SLOWEST_SHARE`) at **1.5 m** (`WADE_MAX_DEPTH_M`, chest-deep on a 1.8 m character) and
  beyond: `1 - 0.6 * clamp((depth - 0.5) / 1.0, 0, 1)`.
- **Deeper than 1.5 m is refused**: a character on dry or wadeable ground does not step into it,
  as a wall blocks it; the step is not taken. A character that is in it (placed or dropped there,
  or the water rose under it) may move, at the slowest share, and so gets out. There is no
  swimming yet: a character keeps the ground under the water.

Rays, lines of sight and projectiles pass through water unchanged. A world without water moves as
before, to the bit: the wading code does nothing when the running world has no water.

## Asking

- `query.ground {points}` and `query.ground_height {x, z}` say of each point whether the surface
  there is `ground` or `water` (water when the ground is below the surface), with
  `water_level_m` and `water_depth_m` (null on dry land, and without ground): for the generator
  of a map, before anything is simulated.
- `sim.state` gives each character's `water_depth_m` while playing.

## The picture

VisionCraft's renderer draws one water surface per scene (`Sea`: a level and an outline of up to
64 points, Gerstner waves, a planar reflection). The adapter draws (`gamecraft-render/src/water.rs`):

- water everywhere when there is any (the first such in id order), else the **largest area** by
  its outline (the first of equal size in id order); the other areas are not drawn and every
  picture's `problems` names them. The simulation and the queries know them all the same;
- waves from the wind, as a fully developed sea's (Pierson-Moskowitz: a significant wave height of
  0.21 U²/g for a wind of U m/s), at most 5 m (WMO sea state 6), with a peak wavelength of 39
  times the height (at least 3 m); an outlined area is inland water, with little fetch: at most
  1 m and 14 times the height long (at least 1.5 m), as in VisionCraft's own model, and still
  water (waves under 0.1 m) freezes when the air is at 0 degrees or below;
- the waves travel with the wind; the colour is open coastal water's deep blue, foam as for the
  sea state.

## Open

- Swimming (M4, M6): a character in deep water floats and moves at a swimming speed; boats.
- Projectiles slow down and lose their way in water; sight under water.
- Several water surfaces in one picture (a sea and lakes, a river through a town): a renderer
  feature, to build in VisionCraft.
- Rivers with a flow along a line (VisionCraft's `Sea` has a flow), a coastline for the endless
  ground, ice as ground to walk on, the sound of water.
- Numbers of a pack: a tall or a loaded character wades less deep; per-character wading depths
  when a second pack needs them.
