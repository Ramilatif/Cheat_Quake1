# Cheat_Quake1 — `qcheat`

External Rust toolkit for reading and steering a running **ioquake3** client on Windows x86_64. It attaches to the game process from the outside, locates the engine's data structures in memory, and builds on them: entity dumps, aim assistance, a screen overlay, and a comparison between what the server knows and what the client receives.

The project is a hands-on study of memory reverse-engineering: mirroring C structures byte for byte in Rust, finding them again by signature in a heap whose addresses change on every launch, and reading the client/server split of a real-time game.

> **Language note.** This README is in English. The detailed technical documents (`docs/`) are written in French.

---

## Contents

1. [Features](#features)
2. [Requirements](#requirements)
3. [Quick start](#quick-start)
4. [Command reference](#command-reference)
5. [How it works](#how-it-works)
6. [Repository layout](#repository-layout)
7. [Architecture](#architecture)
8. [Testing](#testing)
9. [Known limitations](#known-limitations)
10. [Documentation](#documentation)
11. [References](#reference)
12. [License](#license)

---

## Features

| Area | What it does | Commands |
|---|---|---|
| Recon | Find the process, list its modules, poll a single value | `find`, `modules`, `hp` |
| Memory | Interpret bytes as game structures, scan for them, dump the live snapshot | `inspect`, `scan`, `players`, `snapshot` |
| Server | Read the server's authoritative entity array and compare it with the client's view | `server-entities` |
| Aim | Locate the view-angle variable; steer the camera by memory write or by mouse input | `find-viewangles`, `aimbot`, `aim-mouse` |
| Overlay | Draw boxes over the players of the current snapshot | `esp` |

All features ship in a single executable, `qcheat`, with one self-documented subcommand per operation.

---

## Requirements

- Windows 10/11, x86_64
- Rust 1.77 or newer (`rustup` stable is fine)
- ioquake3 (`ioquake3.x86_64.exe`) running a locally hosted map:
  ```
  /devmap q3dm17
  /addbot sarge
  ```
  A hosted game keeps client and server code in one process, which `server-entities` relies on. All other commands only need a running client.

---

## Quick start

```powershell
cargo build --release
.\target\release\qcheat.exe --help
```

`qcheat` is a console application: it must be started from PowerShell or cmd. Double-clicking from Explorer opens and closes a window immediately.

Typical session, with the game running and a map loaded:

```powershell
qcheat find                       # PID and module base
qcheat snapshot                   # locate the live snapshot, dump local player + visible entities
qcheat server-entities            # compare the server's player list with the snapshot
qcheat find-viewangles            # move the mouse while it runs
qcheat aim-mouse                  # Insert toggles it
qcheat esp --fov 90               # overlay
```

Absolute addresses inside the game heap change on every launch. Addresses relative to the executable base (RVA) stay valid for a given ioquake3 build.

---

## Command reference

Every command accepts `--process <name>` (default `ioquake3.x86_64.exe`) and `--help`.

### Recon

| Command | Purpose |
|---|---|
| `find` | Report PID and main-module base address. |
| `modules [--filter <text>]` | List loaded modules (exe and DLLs). |
| `hp <addr>` | Poll a 32-bit integer at a fixed address (engine-side HP). |

### Memory

| Command | Purpose |
|---|---|
| `inspect <addr> --mode origin\|raw\|vec3` | Print an address as an `entityState_t`. |
| `scan <center> [range]` | Scan a window (positional hex arguments, default half-range `0x80000`) for `entityState_t`-shaped blocks and detect arrays by stride. |
| `players` | Heap-wide scan filtered on `ET_PLAYER` entities. |
| `snapshot` | Locate `cg.activeSnapshots[2]` and print the local player and every visible entity. |

`players` and `snapshot` scan the window `0x07000000 ± 0x02000000` by default; `--center` and `--range` (hex) override it.

### Server

| Command | Purpose |
|---|---|
| `server-entities` | Locate `g_entities[]` in a hosted game, infer `sizeof(gentity_t)`, cross-check against the engine's `sv.gentities`, then list every server-side player and flag those absent from the client snapshot (removed by PVS). |

Useful flags: `--tolerance` (max client/server position gap, default 150), `--stride` (force `sizeof(gentity_t)`), `--sv-rva` (RVA of `sv.gentities`, skips the scan on later runs), `--center` / `--range`.

### Aim

| Command | Purpose |
|---|---|
| `find-viewangles` | Isolate the live view-angle variable by elimination over several rounds while the mouse moves. |
| `aimbot` | Aim at the closest player by writing view angles into game memory. Experimental: some written addresses are copies that do not feed the renderer. |
| `aim-mouse` | Compute the angular error to the closest player and send relative mouse movement through `SendInput`. Smoothing, per-tick cap, toggle key, and target prediction between snapshots. |

`aim-mouse` defaults: `--sensitivity 5.0 --m-yaw 0.022 --m-pitch 0.022 --smooth 0.15 --max-delta 60 --interval-ms 8 --toggle-key 0x2D`. The sensitivity values must match the game's `sensitivity`, `m_yaw`, and `m_pitch` cvars. Input goes to the foreground window.

### Overlay

| Command | Purpose |
|---|---|
| `esp [--fov] [--interval-ms]` | Transparent click-through window projecting snapshot players to screen coordinates, with linear extrapolation between snapshots. |

---

## How it works

**1. Process access.** `OpenProcess` returns a handle; `ReadProcessMemory` and `WriteProcessMemory` operate on the target from a separate process. No code runs inside the game.

**2. Structure mirrors.** `entityState_t`, `playerState_t`, and `snapshot_t` are redefined in Rust with `#[repr(C)]`. Compile-time assertions fix their sizes (`EntityState` = 208, `PlayerState` = 468, `Snapshot` = 53 772) and key field offsets, so a mistake in a mirror breaks the build instead of producing garbage at runtime.

**3. Finding the snapshot.** The cgame VM keeps `cg` in a dynamically allocated heap, so no fixed address exists. Each 4-byte-aligned offset is treated as a candidate `SnapshotHeader` and passed through range checks (`pm_type`, `clientNum`, `weapon`, origin, HP). Two candidates exactly 53 772 bytes apart identify `cg.activeSnapshots[0..2]`; the one with the larger `serverTime` is the active snapshot.

**4. Entity positions.** For player entities the reliable position is `pos.trBase`; the plain `origin` field is zero for interpolated entities.

**5. Server side.** The server only sends entities inside the client's PVS (potentially visible set), so the snapshot is a filtered view. `g_entities[]`, owned by the server module, holds all entities. In a hosted game it lives in the same process. It is found by matching snapshot players against memory (same entity number, close position, consistent shared block), inferring the array stride from two matches (`stride = Δaddress / Δnumber`), and confirming through the engine pointer `sv.gentities`.

**6. Steering the camera.** `aimbot` writes angles directly. `aim-mouse` follows the game's own input path (mouse delta → view angles → renderer) and therefore lands on the variable the renderer actually uses.

---

## Repository layout

```
Cheat_Quake1/
├── Cargo.toml                 workspace definition and shared dependencies
├── offsets.json               struct sizes, field offsets, engine RVAs
├── README.md
├── LICENSE
├── .github/workflows/ci.yml   Windows build + tests
├── docs/                      detailed documents (French)
│   ├── architecture.md
│   ├── reverse-engineering.md
│   ├── server-reverse.md
│   ├── cahier-des-charges.md
│   └── comprendre-le-projet.md
└── crates/
    ├── sdk/                   struct mirrors, constants, pure logic
    │   └── src/  vector, trajectory, entitystate, playerstate, snapshot,
    │             constants, server*, plausibility*
    ├── process/               Windows process access
    │   └── src/  discovery (Toolhelp32), handle (Read/WriteProcessMemory)
    ├── scanner/               generic memory-scan primitives
    │   └── src/  scan (aligned scan), stride (array detection)
    └── cli/                   the `qcheat` binary
        └── src/
            ├── main.rs        argument parsing and dispatch
            ├── util.rs        shared helpers (hex parsing, defaults)
            ├── locate.rs*     shared snapshot locator
            └── cmd/           one file per subcommand
                find, modules, hp, inspect, scan, players, snapshot,
                find_viewangles, aimbot, aim_mouse, esp,
                server_entities*
```

Files marked `*` are the server-side additions and the shared locator.

---

## Architecture

Four crates with one-way dependencies.

```
            cli (qcheat)
          /    |     \
      sdk   scanner   process
                \       ^
                 +------+
```

| Crate | Responsibility | Depends on |
|---|---|---|
| `sdk` | `#[repr(C)]` mirrors with layout assertions; sanity filters; server-side pure logic (server twin matching, stride inference). No Windows API, no I/O. | `bytemuck` |
| `process` | `ProcessHandle` (read and write) and process/module discovery. Knows nothing about Quake. | `windows`, `thiserror` |
| `scanner` | Chunked aligned scan with a caller-supplied predicate; stride detection. Knows nothing about Quake. | `process` |
| `cli` | The only crate that combines game knowledge and the Windows API (SendInput, GDI). | all |

Rules:

- Pure logic goes in `sdk` so it can be unit-tested on any machine.
- A subcommand is one file in `cmd/` with `Args` and `run`; shared scan code goes in `locate.rs`.
- Libraries return typed errors (`thiserror`); the CLI uses `anyhow`.
- Public items in libraries are documented (`#![warn(missing_docs)]`).

Details and a step-by-step reading order: [`docs/architecture.md`](docs/architecture.md).

---

## Testing

```powershell
cargo test --workspace
```

Covered today: `sdk` (snapshot filters, server twin matching, stride inference; layout assertions run at compile time) and `scanner::stride`. Commands that need a live game are checked manually against a running ioquake3.

---

## Known limitations

- Windows x86_64 only; ioquake3 with the cgame VM (QVM).
- Several engine RVAs are hard-coded in `aimbot.rs` and tied to one ioquake3 build; they are not yet listed in `offsets.json`.
- `aimbot` (memory write) is partly ineffective because some target addresses are copies; `aim-mouse` is the reliable path.
- `server-entities` assumptions still to be confirmed in-game: the `svFlags` offset, the default scan window, and the tail layout of `server_t` (see `docs/server-reverse.md`).
- The client can only see entities inside its PVS; entities outside it are visible only through the server array.
- No configuration file, interactive UI, or structured logging yet.

---

## Documentation

All files below are in French.

| File | Content |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | Structure, data flow, how to add a command, study path |
| [`docs/reverse-engineering.md`](docs/reverse-engineering.md) | Client-side walkthrough: from the first HP value to the full snapshot |
| [`docs/server-reverse.md`](docs/server-reverse.md) | Server-side study: `g_entities[]`, PVS, method and open points |
| [`docs/cahier-des-charges.md`](docs/cahier-des-charges.md) | Project specification, phases, status |
| [`docs/comprendre-le-projet.md`](docs/comprendre-le-projet.md) | Concepts, glossary, typical questions |

---

## References

    ioquake3 source — code/qcommon/q_shared.h, code/cgame/cg_public.h, code/client/client.h
    windows-rs — ReadProcessMemory, Toolhelp32 bindings
    bytemuck — safe Pod casting

Some documentation files may describe work in progress or planned features and may not yet exist in the repository.

---

## License

MIT. ioquake3 is GPL-2.0 and remains the property of its authors; none of its code is included or redistributed here.
