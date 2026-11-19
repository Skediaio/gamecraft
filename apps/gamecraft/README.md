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
```

The first character in id order is the player. Keys:

| Key | Does |
|---|---|
| W / S | walk forward / back |
| A / D | strafe left / right |
| Shift | run |
| Space | jump |
| E | interact (`sim.input`'s `interact`) |
| mouse | turn (the input's `yaw_deg`) and look up and down (the camera, and the input's `pitch_deg` for a launcher); click the window to lock the pointer |
| left button | fire the character's launcher (`fire`) once the pointer is locked; tracers, impact puffs and, in first person, a crosshair are drawn |
| Esc | release the pointer |
| F1 | first or third person |
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
| left stick click, or left trigger held | run |
| south button (A, cross) | jump |
| west button (X, square) | interact |
| right trigger | fire |
| start | pause |
| select | first or third person |

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

The page takes three query parameters: `?world=<url>` fetches a `.gamecraft` file (same origin,
or one that allows it), otherwise the example world opens; `?quality=low|medium|high`; `?invert_look=1` inverts a gamepad's look. Click
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
