# Development

How to build, test and run GameCraft, and how to work with its VisionCraft dependency.

## Needs

- A recent stable Rust toolchain with `rustfmt` and `clippy` (1.100 at the time of writing).
- The wasm target: `rustup target add wasm32-unknown-unknown`.
- For pictures: a GPU that wgpu can use, or a software Vulkan adapter (on Debian and Ubuntu the
  package `mesa-vulkan-drivers`). Everything but pictures works without one.

The VisionCraft crates are fetched by Cargo from <https://github.com/Skediaio/visioncraft>.

## Checks

```sh
cargo xtask ci        # fmt, clippy, tests, layers, wasm: the gate
cargo xtask layers    # the layering and dependency rules of AGENTS.md
cargo xtask wasm      # cargo check for wasm32-unknown-unknown, layers L0 to L4
```

While iterating, run less: `cargo test -p <crate>`, `cargo test -p <crate> --test <file>`,
`cargo clippy -p <crate> --all-targets -- -D warnings`.

Tests that need a GPU skip themselves, with a line that says so, when no adapter is found. Set
`GAMECRAFT_REQUIRE_GPU=1` to turn a missing adapter into a failure, as CI does.

### Several builds at once

Cargo locks its build directory, so two builds of the same checkout wait for each other. An
agent that works beside others gives itself a directory of its own and removes it afterwards:

```sh
export CARGO_BUILD_BUILD_DIR=<scratch>/gamecraft-agent-<name>
export CARGO_TARGET_DIR=<scratch>/gamecraft-agent-<name>/target
```

## The VisionCraft dependency

`visioncraft-render` and `visioncraft-flora` are git dependencies pinned to one rev in the root
`Cargo.toml`. Only the crates named in rule 13 of `AGENTS.md` depend on them, and
`cargo xtask layers` checks that.

### Updating VisionCraft

1. Change the `rev` of both entries in the root `Cargo.toml` to the new commit, and run
   `cargo update -p visioncraft-render -p visioncraft-flora`.
2. Run `cargo xtask ci`. What breaks is in `gamecraft-render` or in the engine's `plants`
   module, by construction.
3. Render the worked example of `docs/agent.md` and look at the picture: a renderer change can
   be valid code and a different image.
4. Say in the commit which VisionCraft commit it moves to and what had to follow.

`wgpu` and `glam` must stay on the versions VisionCraft's renderer uses, since its types cross
the boundary; move them in the same change when VisionCraft moves.

### Working against a local VisionCraft

When a change is needed on both sides, point Cargo at a checkout beside this one without
touching the manifest. Put this in `.cargo/config.local.toml` (ignored by git):

```toml
[patch."https://github.com/Skediaio/visioncraft"]
visioncraft-render = { path = "../visioncraft/crates/visioncraft-render" }
visioncraft-flora = { path = "../visioncraft/crates/visioncraft-flora" }
```

and pass it to the commands that should use it:

```sh
cargo --config .cargo/config.local.toml test -p gamecraft-render
```

Paths in that file are relative to the directory that holds `.cargo`. The change is finished
when VisionCraft's side is on its `main` and the rev here points at it; a commit never depends on
the local file.
