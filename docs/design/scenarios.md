# Scenarios: zones, interactions, variables, objectives, triggers

How a world becomes a game that can be steered. Status: built 2026-11-18 for prototype P1
(`roadmap.md`); the journal and roles are open.

## Method

A scenario is data in the world document, not code. It has five parts, all mechanisms that
name no theme (AGENTS.md rule 16):

- **Components** that make entities take part:
  - `zone {shape: "circle" | "box", radius_m | size: [x, z], height_m}`: a volume standing on the
    entity's position (a cylinder or a box turned by the entity's yaw). It produces `enter` and
    `leave` events for characters.
  - `interactable {reach_m = 2, prompt = ""}`: a character whose input asks to interact, and whose
    eye is within reach and facing it (within 45 degrees), produces an `interact` event for the
    nearest such entity.
  - `light {color, intensity_cd, range_m, on}`: a point light that the renderer draws and that a
    trigger turns on and off. On a character it moves with it.
  - `tags ["..."]`: free labels that triggers and queries filter by (`team:red`, `beacon`).
- **Variables**: `vars`, a map of names to numbers, booleans or strings in the world. Commands set
  them; conditions read them.
- **Objectives**: `objectives`, a list of `{id, title, status: open | done | failed, hidden}`.
  Players and the game master read them; triggers change them.
- **Triggers**: `triggers`, a list of `{id, when, if?, do, once = true, enabled = true}`:
  - `when` is one event pattern: `{event: "enter" | "leave", zone, who?}`,
    `{event: "interact", target?, who?}`, `{event: "time", at_s}` (simulated time since start),
    `{event: "var", name}` (a variable changed), `{event: "start"}`. `zone` and `target` are
    entity references; `who` filters the acting entity by reference or by tag (`"tag:player"`).
  - `if` is an optional condition over variables: `{var, op: "==" | "!=" | "<" | "<=" | ">" | ">=", value}`,
    or `{all: [...]}` and `{any: [...]}` of those.
  - `do` is a list of **commands**, exactly as an agent sends them (`environment.set`,
    `entity.set_component`, `objective.set`, `var.set`, `entity.add`, ...). Inside them the
    strings `$who` and `$target` stand for the entity that caused the event and the entity it
    was about.
- **Messages**: `message.show {text, to?, seconds?}` puts a line on the players' screens and in
  `sim.state`; it is how a scenario and a game master talk to players.

Because a trigger's actions are commands, there is one way to change a world: what a trigger
can do, an agent can do, and the other way round (rules 1 and 6).

## Running

The simulation detects events during a tick and hands them back in order (by tick, then by
the acting entity's id, then by the zone's or target's id). The session runs them after the
tick, before the next one: each matching, enabled trigger whose condition holds runs its
commands in order on the running world, as one all-or-nothing transaction; a trigger with
`once` is disabled when it has fired. A failing command stops that trigger, leaves the world as
it was before it, and is reported in `sim.state`'s `trigger_errors` and in the step's answer.
Triggers fired by commands of triggers are run in the same pass, at most 32 deep; deeper is an
error that names the chain. Everything is deterministic: same world, same inputs, same events,
same hash.

`sim.step` answers with the events of the steps it ran and the triggers that fired, so an agent
that plays or directs sees what happened without polling.

## The game master

The game master is whoever sends world commands to a running world. What makes it practical
is a view and a few orders: `gm.overview` (every character with team, health, behaviour state
and whom it sees; objectives, recent events, weather), `gm.sight` and `gm.order` (patrol, guard,
hold, move, for one character or a squad by tag), messages to a character or a tag, and the
events in each step's answer. The guide is `docs/guide/game-master.md`. An agent directing a
scenario loops: read the state and the events, decide, send commands, step.

## Open

- A journal of every command and input with its tick, so a directed session replays to the
  same hash (M2).
- Who may send which command once there are players (M9): a game master's role.
