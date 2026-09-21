# Cheat_Quake1

External-process Rust toolkit for an [ioquake3](https://github.com/ioquake/ioq3)
client: reads the engine's own per-frame state (position, HP, weapons,
view angles, the full visible entity list) and, on top of that, an
aimbot and a wallhack overlay, tied together by an in-game settings
menu (`qcheat menu`). Built as a learning project around
reverse-engineering, memory scanning, and matching Rust struct layouts
byte-for-byte to a real C codebase — every write path stays external
(`ReadProcessMemory` / synthetic `SendInput`), there is no code
injection or engine patch anywhere in this repo.

The code targets **ioquake3 on Windows x86_64**. ioquake3 is GPL; this
project does not redistribute any of its code.

> Scope: educational reverse-engineering against a process running on
> the same machine. Not designed for use against online servers.

## Workspace layout

Four crates with strictly layered responsibilities. A dependency never
travels up the stack: `sdk` knows nothing about Windows, `process`
knows nothing about Quake, `scanner` knows nothing about either, and
the binaries in `cli` glue everything together.

```
crates/
  sdk/         repr(C) mirrors of ioquake3 engine structs
               (Vec3, Trajectory, EntityState, PlayerState, Snapshot)
               with compile-time size and offset assertions.
               No Windows deps, no I/O.

  process/     External-process discovery and memory access on Windows.
               ProcessHandle, ReadProcessMemory/WriteProcessMemory
               wrapper, Toolhelp32-based find_by_name / list_modules.
               Game-agnostic.

  scanner/     Generic memory-scan primitives. scan_aligned() streams a
               window through a reusable buffer and hands every aligned
               candidate to a caller-supplied predicate; stride
               detection recognises array layouts in scattered hits.
               Unit-tested, no game knowledge.

  cli/         `qcheat` — single binary with one subcommand per
               operation, built on top of the lower-level crates. Also
               owns everything Win32-UI-shaped that the read-only
               crates don't need: SendInput mouse injection, and the
               transparent/click-through GDI overlay window used by
               the wallhack and the in-game menu.

offsets.json   Reference table of struct sizes, field offsets, and
               engine RVAs derived from ioquake3 master.

docs/
  reverse-engineering.md   Full walkthrough: locating cg.activeSnapshots,
                           why client-side ESP is PVS-bounded, why the
                           aimbot has to drive mouse input instead of
                           writing view angles in memory, and how the
                           aimbot/wallhack/menu got merged into one
                           robust polling loop.
```

### Dependency graph

```
            sdk  ──────────────┐
             ▲                 │
             │                 │
         scanner               │
             ▲                 │
             │                 │
          process ◄────────────┤
             ▲                 │
             │                 │
            cli ◄──────────────┘
```

## The `qcheat` binary

Everything ships as a single executable, `qcheat`, with one subcommand
per operation. Each subcommand lives under `crates/cli/src/cmd/` and
exposes its own `--help`.

| Subcommand              | What it does                                                       |
| ------------------------ | ------------------------------------------------------------------ |
| `qcheat menu`            | **Main entry point.** Unified aimbot + wallhack, one shared snapshot read, plus an in-game settings menu (`F1` by default, keyboard-navigated — the game captures the mouse cursor, so the menu never relies on it). Target-selection mode (closest / smallest angle), sensitivity, smoothing, FOV and hotkeys can all be sourced from [`qcheat.toml`](qcheat.example.toml) instead of retyped flags — CLI flags still override the file. See [below](#the-cheat-aimbot--wallhack--in-game-menu). |
| `qcheat doctor`          | Environment diagnostic: process found, memory read/write access, game window locatable, a real snapshot currently present, synthetic mouse input accepted by the OS, `qcheat.toml` valid. Run this first after an ioquake3 update or when something that used to work stops working. |
| `qcheat aim-mouse`       | Aimbot only, standalone: computes the angle to the closest living player and injects the correction as relative mouse movement (`SendInput`), never writes to game memory. |
| `qcheat esp`             | Wallhack only, standalone: projects every living enemy's `pos.trBase` into a transparent, click-through overlay window — boxes drawn through walls, since occlusion is a render-time decision, not a property of the snapshot data. |
| `qcheat aimbot`          | Original memory-write aimbot attempt, kept for reference. Writes to `cl.snapshots[]`; the engine recomputes the view angle from mouse input every predicted frame regardless, so it has **no effect on the actual camera** — see [docs/reverse-engineering.md §9](docs/reverse-engineering.md) for why. |
| `qcheat find-viewangles` | Diagnostic scanner used while chasing the (nonexistent) writable view-angle address: narrows candidate floats against a known-good reference read from the snapshot. |
| `qcheat find`            | Locate `ioquake3.x86_64.exe`, report PID and main-module base.     |
| `qcheat modules`         | Enumerate every DLL loaded in the target process.                  |
| `qcheat hp`              | Poll a 32-bit value at a fixed address (typically engine-side HP). |
| `qcheat inspect`         | Treat an arbitrary address as an `entityState_t` and pretty-print. Modes: `--mode origin\|raw\|vec3`. |
| `qcheat scan`            | Brute-scan a memory window for `entityState_t`-shaped bytes and detect array strides. |
| `qcheat players`         | Whole-heap scan filtered to `ET_PLAYER` entities.                  |
| `qcheat snapshot`        | Locate `cg.activeSnapshots[2]` by its 53 772-byte signature pair, then dump the live local-player block and every visible entity for the current frame. |

Build (debug + release):

```powershell
cargo build --workspace            # target/debug/qcheat.exe
cargo build --release              # target/release/qcheat.exe — distributable
```

Run the unit tests (currently `scanner::stride`):

```powershell
cargo test --workspace
```

Typical workflow with ioquake3 running and a map loaded:

```powershell
cargo run -p cli -- find
cargo run -p cli -- snapshot
cargo run -p cli -- inspect 0x0613F728 --mode raw
cargo run -p cli -- scan 0x06800000 0x80000

# Or directly with the built binary:
.\target\release\qcheat.exe snapshot
.\target\release\qcheat.exe --help

# The cheat itself — aimbot + wallhack + in-game menu, F1 to open:
.\target\release\qcheat.exe menu
```

Note: `qcheat` is a console application. Launching it by double-click
from Explorer just flashes a window with the auto-generated help and
exits — always run it from PowerShell / cmd.

## How the snapshot reader works

ioquake3's client renders each frame from a `snapshot_t` produced by the
server. Inside the cgame VM, the active and next snapshots are stored
back-to-back as `cg.activeSnapshots[2]` (~53.8 KiB each).

`qcheat snapshot` walks the QVM heap window at 4-byte alignment,
treating every offset as a candidate `SnapshotHeader` and applying a
strict sanity filter (`pm_type` ∈ 0..=8, `clientNum` ∈ 0..MAX_CLIENTS,
weapon ∈ 0..=15, finite in-map origin, plausible HP, non-empty player
state). The decisive signal is when two candidates sit **exactly**
`sizeof(snapshot_t) = 53 772` bytes apart — that's the
`cg.activeSnapshots[0..2]` pair, and we pick whichever has the larger
`serverTime` (= the active `cg.snap`).

Once located, the subcommand reads the 53 KiB struct in one
`ReadProcessMemory` round-trip and iterates
`entities[0..numEntities]`. For `ET_PLAYER` entities, the canonical
position is `pos.trBase` (not `origin`, which the engine leaves zero for
interpolated entities).

## The cheat: aimbot + wallhack + in-game menu

### Why the aimbot drives the mouse instead of writing memory

The obvious approach — compute the angle to the closest enemy and
`WriteProcessMemory` it into `cg.snap.ps.viewangles` — compiles, runs,
and does nothing. `code/game/bg_pmove.c`'s `PM_UpdateViewAngles`
recomputes that angle from `cmd->angles` (itself derived from raw
mouse delta) on **every predicted frame**, client-side, to hide
network latency. Any value written there is discarded before the next
frame renders, no matter which of the several in-memory copies gets
targeted — `qcheat aimbot` is kept in the tree as a record of that
dead end.

`qcheat aim-mouse` instead computes the same angle and injects it as a
relative mouse delta via `SendInput` (`Win32::UI::Input::KeyboardAndMouse`).
The engine can't tell it apart from a real mouse move, so it goes
through the exact same path a human's input would — no address to
find, no write to race against the engine's own recompute.

Two details that mattered in practice:
- `cl_input.c`: `cl.viewangles[YAW] -= m_yaw * mx` — moving the mouse
  right *decreases* yaw. Get the sign backwards and the aimbot
  actively steers away from the target instead of doing nothing.
- The server ticks (`sv_fps`) far slower than a responsive poll loop
  (~20-40 Hz vs. ~120 Hz), so recomputing a correction from the same
  stale snapshot every poll overshoots and oscillates. The fix mirrors
  what the engine's own renderer does: extrapolate the target's
  position from the velocity observed between the last two real
  snapshots, and integrate the aimbot's own view-angle estimate
  locally between real updates instead of re-reading a value that
  won't reflect its own just-sent correction until the next tick.

### The wallhack: projecting the snapshot, not reading new memory

`cg.snap.entities[]` already contains every player in the local PVS
(§ below), occluded or not — occlusion is a rendering decision, not a
property of the data. `qcheat esp` reprojects each living enemy's
`pos.trBase` into screen space with the same math the engine's
`AngleVectors` uses, and draws a box in a layered, click-through,
always-on-top GDI window sitting over the game. Same velocity
extrapolation as the aimbot keeps the boxes smooth between real
snapshots instead of stair-stepping at server tick rate.

### `qcheat menu`: both together, one shared loop

Running the aimbot and the wallhack as separate processes means two
independent memory scans and no shared configuration. `qcheat menu`
merges both onto a single snapshot read per tick and adds a
keyboard-driven settings menu drawn into the same overlay window
(`F1` by default — the game captures/hides the system cursor for
mouselook, so the menu is navigated with the arrow keys, never the
mouse). Toggle the aimbot and wallhack independently, and tune
sensitivity/smoothing/FOV live without restarting.

Full narrative — every dead end, every bug found and why, in the
order they happened — is in
[docs/reverse-engineering.md](docs/reverse-engineering.md) (§9 onward,
in French).

## Why client-side ESP is naturally limited

Quake III applies PVS (Potentially Visible Set) culling on the server
before sending each snapshot. Entities outside the local PVS are simply
not transmitted, so a client-side reader cannot see them no matter how
deep it digs. Reading `cg.snap` gives the *exact* set of entities the
engine renders — nothing less, but nothing more either.

To see *every* player regardless of PVS, the reader needs to attach to
a process that has the authoritative world state — i.e. the server. In
ioquake3 that means reading `g_entities[MAX_GENTITIES]` from
`qagame.qvm` / `qagamex86_64.dll` when running a listen server (e.g.
`devmap` + `addbot`). Not currently implemented here.

## Layout validation

Every mirror in `crates/sdk` has compile-time `assert!`s on size and
key field offsets:

```rust
const _: () = assert!(core::mem::size_of::<EntityState>() == 208);
const _: () = assert!(core::mem::size_of::<PlayerState>()  == 468);
const _: () = assert!(core::mem::size_of::<Snapshot>()     == 53_772);
```

If ioquake3 ever changes a struct, the build fails immediately —
nothing reads garbage at runtime. The same values are recorded in
[offsets.json](offsets.json) as a single source of truth that can be
consulted without running the build.

## Writeup

A full walkthrough of the reverse-engineering work lives in
[docs/reverse-engineering.md](docs/reverse-engineering.md): how
`cg.activeSnapshots` was located, why client-side ESP in Quake III is
PVS-bounded by construction, every dead end hit trying to write the
view angle into memory before landing on mouse-input injection, the
server-tick-vs-poll-rate jitter and how prediction fixed it, the
wallhack's 3D→2D projection, and how it all got merged into one
robust `qcheat menu` loop.

## References

- [ioquake3 source](https://github.com/ioquake/ioq3) — `code/qcommon/q_shared.h`, `code/cgame/cg_public.h`, `code/client/client.h`, `code/client/cl_input.c`, `code/game/bg_pmove.c`
- [windows-rs](https://github.com/microsoft/windows-rs) — `ReadProcessMemory`/`WriteProcessMemory`, `Toolhelp32`, `SendInput`, GDI and window-management bindings
- [bytemuck](https://github.com/Lokathor/bytemuck) — safe `Pod` casting

## License

MIT. ioquake3 is GPL-2.0 and remains the property of its authors;
none of its code is included or redistributed here.
