# gamecraft: the window

A person walks a character through a GameCraft world, natively and in the browser on WebGPU,
with plain winit on wgpu (no UI toolkit; menus and a heads-up display are M10). The session is
the source of truth: the window starts the simulation with `sim.start`, stores the player's
input with `sim.input` when it changes and advances it with `sim.step` at a fixed 60 ticks per
second, at most four ticks per frame; the picture is interpolated between the last two ticks.

## Natively

```sh
cargo run -p gamecraft --release                      # the engine's example valley
cargo run -p gamecraft --release -- my.gamecraft      # a world file
cargo run -p gamecraft --release -- --quality low     # low, medium (default) or high
cargo run -p gamecraft --release -- --invert-look     # a gamepad's right stick up looks down
cargo run -p gamecraft --release -- --fov 90          # your own vertical field of view, 30 to 120
```

The first character in id order is the player. The world's view rules (`view.set`, the `view`
section of the document: `modes` and `fov_y_deg`) say which views the player may use and how
wide the camera sees. The window starts in the first view listed (third person for a world
without rules, which allows both), and F1 or select switches only among those listed: a world
with `"modes": ["first"]` stays in first person. The camera's vertical field of view is the
world's `fov_y_deg` (70 by default) unless `--fov` (natively) or `?fov=` (browser) sets the
player's own. In first person the player's `held_model` (a weapon's view model) is drawn at the
camera along the aim. While playing with a launcher, the bottom right corner shows the fire mode and the rounds
left (`AUTO 27/30`, `RELOAD` while reloading). Keys:

| Key | Does |
|---|---|
| W / S | walk forward / back |
| A / D | strafe left / right |
| Shift | run; while aiming down the sights, hold the breath instead (`hold_breath`: a steadier sway for a few seconds, worse for a while after) |
| Space | jump |
| E | interact (`sim.input`'s `interact`) |
| mouse | turn (the input's `yaw_deg`) and look up and down (the camera, and the input's `pitch_deg` for a launcher); click the window to lock the pointer |
| Left Alt, held | free look: the mouse (and the right stick) turns only the head, at most 100 degrees to either side and within the usual tilt; the character's facing, its aim and its held model stay; let go and the camera is back on the aim |
| left button | the trigger of the character's launcher (`fire`) once the pointer is locked: once per click in semi, a burst in burst, while held in auto, nothing on safe; tracers, impact puffs and, in first person, a crosshair are drawn; the recoil climbs the view and the mouse pulls it back |
| right button, held | aim down the sights (`ads`): less sway and spread; in first person the held model comes up to its `ads_offset_m`, the view narrows by its `ads_fov_factor` (over its `ads_time_s`) and the crosshair is hidden |
| B | switch the fire mode (`switch_mode`: the next of the launcher's `modes`) |
| R | reload (`reload`) |
| Esc | release the pointer |
| F1 | the next view the world allows (first or third person) |
| F5 | restart the simulation from the document |
| P | pause |

### Gamepad

A connected gamepad plays the same character through the same input records as the keyboard
(natively, and in the browser through its Gamepad API: press a button once so the page sees the
pad). The keyboard and the pad may be used together: the last one used owns the walking axes,
and the right stick adds to the mouse's look.

| Pad | Does |
|---|---|
| left stick | walk (radial dead zone of 0.2) |
| right stick | look at up to 150 degrees per second, whatever the frame rate; `--invert-look` (natively) or `?invert_look=1` (browser) flips up and down |
| left stick click | run; while aiming down the sights, hold the breath |
| left trigger | aim down the sights |
| north button (Y, triangle) | switch the fire mode |
| east button (B, circle) | reload |
| south button (A, cross) | jump |
| west button (X, square) | interact |
| right trigger | fire |
| start | pause |
| select | the next view the world allows |

New scenario messages and objective changes are printed to stderr, and the newest message is
in the window's title.

### An agent drives the window: `--stdin`

```sh
gamecraft --stdin < requests.jsonl
```

With `--stdin` the window reads the JSON-lines protocol of `gamecraft-cli serve`
(`docs/control-protocol.md`) on stdin, runs each request through the same session between two
frames, and answers each on stdout (stdout carries nothing else). For example:

```json
{"id": 1, "method": "engine.execute", "params": {"command": "environment.set", "params": {"time_of_day_h": 20}}}
{"id": 2, "method": "engine.execute", "params": {"command": "sim.state"}}
```

The window's automation may not read or write files, and `view.render` answers that no
renderer is attached: a picture of the window, and the control channel over a socket
(`gamecraft-cli mcp --bridge`), come later in M1.

## In the browser

WebGPU only, no WebGL. The page is built with [Trunk](https://trunkrs.dev) from the same crate
(`index.html` here):

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk --locked          # Trunk fetches wasm-bindgen 0.2.129 and wasm-opt itself
cd apps/gamecraft
trunk serve --release                 # http://127.0.0.1:8080
trunk build --release                 # or a static build in apps/gamecraft/dist
```

The page takes these query parameters: `?world=<url>` fetches a `.gamecraft` file (same origin,
or one that allows it), otherwise the example world opens; `?quality=low|medium|high`; `?invert_look=1` inverts a gamepad's look; `?fov=90` sets your own
vertical field of view, 30 to 120 degrees (a bad value is told on the console and the world's is
used). Click
the canvas to look around with the mouse; the keys are the same as natively, and messages go
to the browser's console. When the browser offers no WebGPU, the page says so and how to turn
it on.

Chrome, Edge and Brave on Windows, macOS, ChromeOS and Android work when up to date. On Linux,
Chromium-based browsers may need `chrome://flags/#enable-unsafe-webgpu` and
`chrome://flags/#enable-vulkan` (and `#ignore-gpu-blocklist`); `chrome://gpu` should say
"WebGPU: Hardware accelerated".

### Browser smoke test

After `trunk build --release`, with Node 22 or later and a Chromium:

```sh
node apps/gamecraft/browser-smoke/smoke.mjs --dist apps/gamecraft/dist \
  [--chrome /usr/bin/chromium-browser] [--out out] [--size 1280x720] [--query "quality=low"]
```

It serves the build, opens it in headless Chromium on WebGPU (`--enable-unsafe-webgpu
--enable-features=Vulkan --use-angle=vulkan --ignore-gpu-blocklist`, and flags that keep a
background page's frames running), waits for 60 frames, holds W for two seconds, and fails on
console errors, WebGPU validation messages, a panic, the page's error card, or a walk that
changes nothing. It writes `out/window-browser.png` and `out/window-browser-walked.png`.

## Tests

```sh
cargo test -p gamecraft                                   # input, step, interpolation, cameras, the game loop
GAMECRAFT_REQUIRE_GPU=1 cargo test -p gamecraft --test offscreen
```

The offscreen test plays the example valley for 120 frames with W held through the window's
frame path into a texture, and writes `out/window-third.png`, `out/window-first.png` and a
night walk with a lantern, `out/window-night.png`. Without a GPU it skips, unless
`GAMECRAFT_REQUIRE_GPU=1`.
