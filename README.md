# Moving TowerBloxx — Gurgaon

A browser 3D **crane-stacker city builder**. A hook swings a floor-slab over a
plot; you drop it to stack a tower, floor by floor, building **real DLF Gurgaon
buildings** to their actual floor counts. The build site sits inside a live
recreation of **Gurgaon (DLF Cyber City) pulled from OpenStreetMap**, with
expressways full of traffic, an elevated Rapid Metro, Times-Square-style LED
billboards, golden-hour haze, bloom and real-time shadows.

Runs entirely on **WebGPU** — Rust compiled to WebAssembly, rendering via `wgpu`.
No game engine, no external art assets; the city is generated from real map data
+ procedural geometry, shaded in WGSL.

## Tech stack
| Layer | Tech |
|-------|------|
| Language | **Rust** → `wasm32-unknown-unknown` |
| GPU | **WebGPU** via **`wgpu` 25** (native: DX12/Vulkan/Metal) |
| Window/input | **`winit` 0.29** |
| Wasm glue | **`wasm-bindgen`** (+ futures) |
| Math / GPU data | **`glam`** / **`bytemuck`** |
| Shaders | **WGSL** (`cube.wgsl`, `post.wgsl`) |
| Real city data | **OpenStreetMap Overpass API** |
| Audio | **Web Audio API** + **YouTube IFrame Player API** |
| Dev server | **Node.js** (`serve.js`, zero deps) |

**Rendering:** shadow pass (2048² sun shadow map) → HDR main pass (glass
curtain-wall, procedural grain, PCF shadows, haze) → composite (bloom + ACES
tone-map). Everything is one instanced unit-cube draw.

## Controls
- **Click / Space / tap** — drop the slab
- **Right-drag / touch-drag** — orbit the camera
- **P** pause · **F** fullscreen · **M** mute · **♪** toggle soundtrack

Land slabs aligned for full-width floors (combo on a perfect drop); sloppy drops
taper the tower, a near-total miss topples it. Complete a building's real floor
count to move to the next DLF building.

## Running it
### Prerequisites
- Rust + wasm target: `rustup target add wasm32-unknown-unknown`
- `wasm-bindgen-cli` **matching the crate version in `Cargo.lock`** (see below)
- Node.js (serves wasm with the right MIME type)
- A WebGPU browser: Chrome/Edge 113+ or Safari 18+

### Build & serve
```bash
# 1. Compile to WebAssembly
cargo build --release --target wasm32-unknown-unknown

# 2. Match the wasm-bindgen CLI to the crate, then generate JS bindings
VER=$(grep -A1 '^name = "wasm-bindgen"' Cargo.lock | grep version | head -1 | cut -d'"' -f2)
cargo install wasm-bindgen-cli --version "$VER"
wasm-bindgen --target web --out-dir pkg --no-typescript \
  target/wasm32-unknown-unknown/release/tetris3d.wasm

# 3. Serve over HTTP (wasm won't load from file://)
node serve.js            # -> http://localhost:8080
```
Open http://localhost:8080 and click once (starts audio + fullscreen). It fetches
the real Gurgaon buildings from OpenStreetMap on load.

> The crate is named `tetris3d` (historical), so the artifact is `tetris3d.wasm`
> and `index.html` imports `./pkg/tetris3d.js`.

### Native desktop build (fast shader/logic debugging)
```bash
cargo run --release --bin dlf_builder
```

## Project structure
```
src/
  lib.rs        wasm entry (run, set_city)
  main.rs       native entry
  app.rs        winit loop, mouse+touch input, game loop, JS bridges
  game/mod.rs   crane-stacker sim + real DLF building catalog
  render/       gpu, camera, cube, instance, scene, pipeline (shadow+HDR+bloom)
  shaders/      cube.wgsl (material) + post.wgsl (bloom + tone-map)
index.html      UI, WebGPU gate, OSM loader, audio, HUD bridges
serve.js        zero-dependency static server (correct wasm MIME)
```

## Data & attribution
- Building footprints/heights: **© OpenStreetMap contributors** (ODbL) via the
  Overpass API, sampled around DLF Cyber City, Gurugram.
- Real DLF facts baked into `src/game/mod.rs` from public sources.
- Soundtrack: your own file or a YouTube playlist (IFrame Player API).

## Notes
- Pinned to **wgpu 25 / winit 0.29**. wgpu ≤ 0.20 crashes current Chrome
  (`maxInterStageShaderComponents`) — do not downgrade.
- Renders at native device-pixel ratio (crisp on hi-DPI / 4K), capped at 3×.
