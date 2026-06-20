# Cheat_Quake1

External-process Rust toolkit for reading the running state of an
[ioquake3](https://github.com/ioquake/ioq3) client — position, HP,
weapons, view angles, and the full per-frame entity list as the engine
sees it. Built as a learning project around reverse-engineering, memory
scanning, and matching Rust struct layouts byte-for-byte to a real C
codebase.

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
               ProcessHandle, ReadProcessMemory wrapper, Toolhelp32-based
               find_by_name / list_modules. Game-agnostic.

  scanner/     Generic memory-scan primitives. scan_aligned() streams a
               window through a reusable buffer and hands every aligned
               candidate to a caller-supplied predicate; stride
               detection recognises array layouts in scattered hits.
               Unit-tested, no game knowledge.

  cli/         Binaries that drive the lower-level crates against a live
               ioquake3 process. The only crate that imports all the
               others.

offsets.json   Reference table of struct sizes, field offsets, and
               engine RVAs derived from ioquake3 master.

docs/
  reverse-engineering.md   Full walkthrough of how dump-snapshot was
                           built and why client-side ESP is PVS-bounded.
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

## Binaries

All live under `crates/cli/src/bin/`:

| Binary           | What it does                                                       |
| ---------------- | ------------------------------------------------------------------ |
| `find-process`   | Locate `ioquake3.x86_64.exe`, report PID and main-module base.     |
| `list-modules`   | Enumerate every DLL loaded in the target process.                  |
| `read-hp`        | Read the engine-side `cl.snap.ps.stats[STAT_HEALTH]` at a known RVA. |
| `inspect-entity` | Treat an arbitrary address as an `entityState_t` and pretty-print. |
| `scan-entities`  | Brute-scan a memory window for `entityState_t`-shaped bytes and detect array strides. |
| `dump-players`   | Whole-heap scan filtered to `ET_PLAYER` entities.                  |
| `dump-snapshot`  | Locate `cg.activeSnapshots[2]` by its 53 772-byte signature pair, then dump the live local-player block and every visible entity for the current frame. |

Build everything:

```powershell
cargo build --workspace
```

Run the unit tests (currently `scanner::stride`):

```powershell
cargo test --workspace
```

Typical workflow with ioquake3 running and a map loaded:

```powershell
cargo run -p cli --bin find-process
cargo run -p cli --bin dump-snapshot
```

## How the snapshot reader works

ioquake3's client renders each frame from a `snapshot_t` produced by the
server. Inside the cgame VM, the active and next snapshots are stored
back-to-back as `cg.activeSnapshots[2]` (~53.8 KiB each).

`dump-snapshot` walks the QVM heap window at 4-byte alignment, treating
every offset as a candidate `SnapshotHeader` and applying a strict
sanity filter (`pm_type` ∈ 0..=8, `clientNum` ∈ 0..MAX_CLIENTS, weapon ∈
0..=15, finite in-map origin, plausible HP, non-empty player state).
The decisive signal is when two candidates sit **exactly**
`sizeof(snapshot_t) = 53 772` bytes apart — that's the
`cg.activeSnapshots[0..2]` pair, and we pick whichever has the larger
`serverTime` (= the active `cg.snap`).

Once located, the binary reads the 53 KiB struct in one
`ReadProcessMemory` round-trip and iterates
`entities[0..numEntities]`. For `ET_PLAYER` entities, the canonical
position is `pos.trBase` (not `origin`, which the engine leaves zero for
interpolated entities).

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

A walkthrough of the reverse-engineering work — how `dump-snapshot`
was built, the dead ends along the way, and why client-side ESP in
Quake III is PVS-bounded by construction — lives in
[docs/reverse-engineering.md](docs/reverse-engineering.md).

## References

- [ioquake3 source](https://github.com/ioquake/ioq3) — `code/qcommon/q_shared.h`, `code/cgame/cg_public.h`, `code/client/client.h`
- [windows-rs](https://github.com/microsoft/windows-rs) — `ReadProcessMemory`, `Toolhelp32` bindings
- [bytemuck](https://github.com/Lokathor/bytemuck) — safe `Pod` casting

## License

MIT. ioquake3 is GPL-2.0 and remains the property of its authors;
none of its code is included or redistributed here.
