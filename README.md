# Cheat_Quake1

External-process Rust toolkit for reading the running state of an [ioquake3](https://github.com/ioquake/ioq3) client — position, HP, weapons, view angles, and the full per-frame entity list as the engine sees it. Built as a learning project around reverse-engineering, memory scanning, and matching Rust struct layouts byte-for-byte to a real C codebase.

ioquake3 is licensed under GPL-2.0. This repository does not redistribute any of its source code.

Scope: Intended for educational reverse engineering in a locally hosted game (/devmap + /addbot), against bots on the same machine. It is not designed for use against online servers or real players. See Responsible Use.
Development status: Work in progress. Some features are experimental, implementation details may change, and the roadmap describes planned work rather than existing functionality.
What the project currently does

The qcheat binary exposes commands for inspecting the running game, scanning memory, and experimenting with client-side interaction.

Read-only inspection
Command	Description
find	Locates the game process and reports its PID and main-module base address.
modules [--filter]	Lists modules loaded by the target process, including the executable and DLLs.
hp <addr>	Reads a 32-bit integer from a specified address, such as a candidate engine-side health value.
inspect <addr> --mode origin|raw|vec3	Interprets memory at an address as an entityState_t and prints the selected representation.
scan <center> [--range]	Scans a memory window for entityState_t-like data and looks for repeated array strides.
players	Scans the process heap for candidate entities with the ET_PLAYER type.
snapshot	Locates cg.activeSnapshots[2] using a pair of candidates separated by 53,772 bytes, then displays the local-player state and the entities in the active snapshot.
find-viewangles	Attempts to identify the rendered yaw address by comparing candidate values across confirmation rounds while the mouse is moved.
Experimental interaction features

These commands go beyond passive inspection. They are experimental and intended only for a locally hosted test session.

Command	Description	Notes
aimbot	Attempts to aim at the nearest player by writing view-angle values to candidate memory addresses, including snapshot copies.	Reliability is limited. Some target addresses may refer to historical snapshot copies rather than the values used for rendering. Runs continuously and has no activation key.
aim-mouse	Calculates angular error toward the nearest player and sends relative mouse movement through SendInput.	Supports smoothing, per-tick movement limits, an activation toggle (Insert by default), and target-position prediction between snapshots.
esp	Displays a transparent, click-through overlay with boxes around players found in the current snapshot.	Uses linear extrapolation between snapshots and remains subject to the client's PVS visibility limits.
Not implemented yet

The following items are planned, not currently available as implemented features:

Reading the authoritative server-side g_entities[] array.
A qcheat.toml configuration file.
A TUI (terminal user interface).
A doctor diagnostic command.
Structured logging.
A technical guard that enforces local-session-only use for interaction features.

See the Roadmap for planned development.

Architecture

The workspace contains four crates with separate responsibilities. Arrows represent dependencies.

                 cli (qcheat)
                /     |     \
              sdk  scanner  process
                     \       ^
                      +------+

Crate	Responsibility	Dependencies
sdk	#[repr(C)] mirrors of engine structures: Vec3, Trajectory, EntityState, PlayerState, Snapshot, and SnapshotHeader. Includes compile-time size and selected offset assertions. No Windows-specific APIs or I/O.	bytemuck
process	External-process discovery and memory access through ProcessHandle, including reading and writing memory and Toolhelp32-based module enumeration. Game-agnostic.	windows, thiserror
scanner	Generic, aligned memory scanning with caller-provided predicates and stride detection. Does not contain Quake-specific logic.	process
cli	The qcheat executable. Connects the lower-level crates and contains command implementations, including Windows input and overlay functionality.	All three lower-level crates

offsets.json records reference structure sizes, field offsets, and engine RVAs observed for the target build.

The intended dependency direction is from the CLI toward the lower-level crates. The SDK should remain independent of Windows APIs and process I/O.

Build and run
Requirements
Windows x86_64.
A compatible ioquake3 build.
Rust and Cargo.
A locally hosted test session for gameplay-related experiments.
Build the release executable
cargo build --release


The executable is generated at:

target\release\qcheat.exe

Run the tests
cargo test --workspace


Test coverage is currently limited. The scanner's stride-detection logic is tested, but the SDK layout and the live-process functionality do not yet have comprehensive automated coverage.

Show help
.\target\release\qcheat.exe --help


qcheat is a console application. Running it by double-clicking in Explorer may open and immediately close the console window. Run it from PowerShell or Command Prompt to inspect its output.

Typical workflow

With ioquake3 running and a map loaded in a locally hosted session with bots:

.\target\release\qcheat.exe find
.\target\release\qcheat.exe snapshot
.\target\release\qcheat.exe inspect 0x0613F728 --mode raw
.\target\release\qcheat.exe find-viewangles


The address above is only an example. Memory addresses can change between launches.

The following commands exercise the experimental interaction features. Use them only in a local test session:

.\target\release\qcheat.exe aim-mouse --toggle-key 0x2D
.\target\release\qcheat.exe esp --fov 90


0x2D is the Windows virtual-key code for Insert.

Useful defaults
Feature	Parameter	Default
Memory scan	Center address	0x07000000
Memory scan	Range	0x02000000
aim-mouse	--sensitivity	5.0
aim-mouse	--m-yaw	0.022
aim-mouse	--m-pitch	0.022
aim-mouse	--smooth	0.15
aim-mouse	--max-delta	60
aim-mouse	--interval-ms	8
esp	--interval-ms	8
esp	--fov	90

Mouse-related parameters must match the game's sensitivity, m_yaw, and m_pitch settings.

These defaults and example commands may change as development progresses. Use each command's --help output as the source of truth for its current options.

Addresses and build-specific offsets

Absolute addresses in the QVM heap may change every time the game starts. Engine RVAs are relative to the executable's module base and may remain stable for a given binary, but they can change when the game is rebuilt or replaced.

The hard-coded RVAs used by experimental commands, including those in aimbot.rs, are empirical values for the observed build. They are not yet managed through a unified configuration system.

How it works
Finding the snapshot

With cgame.qvm, the client game state lives in the QVM's memory heap, so its absolute address is not fixed.

Rather than relying on a hard-coded heap address, snapshot scans aligned memory locations and treats each candidate as a possible SnapshotHeader. It applies sanity checks to fields such as:

pm_type
clientNum
weapon
Player origin and health
Whether the player state appears populated

The key heuristic is a pair of plausible candidates separated by exactly sizeof(snapshot_t), expected to be 53,772 bytes for the targeted layout.

This spacing is consistent with the two elements of cg.activeSnapshots[2]. The candidate with the greater serverTime is selected as the likely active snapshot.

Once located, the snapshot is read in a single ReadProcessMemory call, and the entity list is inspected.

This is a heuristic, not a universal guarantee. Its reliability depends on the target build, structure layout, and the validity of the assumptions used to identify candidates.

The trBase detail

For player entities, the canonical trajectory position is pos.trBase, rather than origin.

Depending on the entity and interpolation path, origin may not contain the position expected by the reader. Using the trajectory base avoids relying on that field when it is not populated as expected.

Why aim-mouse exists

The experimental memory-writing approach in aimbot can target copies of view-angle data that do not directly control rendering.

aim-mouse takes a different approach: it computes an angular correction and sends relative mouse movement through the game's normal input path. This avoids depending entirely on the experimental memory addresses used by aimbot.

Because client snapshots arrive periodically rather than every rendered frame, target positions can change between updates. The command includes prediction and smoothing intended to handle those differences, although its behavior remains build- and configuration-dependent.

Why ESP is PVS-limited

Quake III uses a Potentially Visible Set (PVS) to determine which entities are transmitted to a client.

The client-side snapshot therefore does not contain every entity in the world. Entities excluded by the server's visibility and snapshot logic cannot be recovered from that snapshot alone.

The esp command can only display entities represented in the client data it reads. This is a limitation of the available client-side state, not simply a limitation of the drawing code.

Inspecting authoritative server-side state would require a separate server-side investigation. That work is outside the current implementation and remains on the roadmap.

Structure layout validation

The SDK mirrors important C structures using Rust #[repr(C)] definitions and compile-time assertions.

Examples include:

const _: () = assert!(core::mem::size_of::<EntityState>() == 208);
const _: () = assert!(core::mem::size_of::<PlayerState>() == 468);
const _: () = assert!(core::mem::size_of::<Snapshot>() == 53_772);


These values are expected for the targeted structure layout.

Compile-time assertions verify that the Rust structures match the reference sizes and offsets encoded in the project. They do not, by themselves, prove that every field matches the binary currently running.

Runtime checks and testing against the exact target build are still necessary. If the engine layout changes, the assertions can catch some mismatches at build time, but they cannot replace full compatibility testing.

Known limitations
Platform: Windows x86_64 only.
Target configuration: The current implementation targets ioquake3 using cgame.qvm; native DLL-based client-game configurations are not currently supported by the documented assumptions.
Build-specific offsets: Experimental commands rely on empirically observed RVAs and memory layouts that may change with a different executable.
Experimental aimbot: Memory-writing behavior is only partially reliable. The targeting logic is basic and does not yet provide comprehensive team or player-state filtering.
Local-session enforcement: The intended local-only scope is not yet enforced by a technical guard in the code.
Duplicated logic: Snapshot discovery is repeated across multiple commands and should be factored into a shared module.
Limited testing: The scanner has targeted tests, but the SDK layout, snapshot parsing, and live-process behavior require more extensive validation.
Reference data cleanup: offsets.json still contains references to older binaries and previous command names, including memory --bin dump-snapshot.
Configuration: Command parameters and build-specific offsets are not yet managed through a unified configuration system.

These limitations are expected to evolve as the project develops.

Responsible use

This project is intended for learning about process memory, reverse engineering, data layouts, and the internal state exposed by a locally running game.

Use it only on your own machine in a locally hosted session against bots.
Do not use it against real players or public servers.
Do not use it to bypass anti-cheat systems, conceal a process, or evade security controls.
Do not distribute precompiled binaries intended for use against other players.
Remember that the interaction commands can write to process memory, generate input, or display an overlay.

Development priority: Add a technical guard shared by aimbot, aim-mouse, and esp that refuses to run these features when the session cannot be verified as locally hosted.

Until that guard exists, local-only use is a documented restriction rather than an enforced technical guarantee.

Roadmap
Local-session guard: Add a shared technical check for the experimental interaction features.
Snapshot refactoring: Move snapshot discovery into a common module and remove duplicated implementations.
Offset management: Move build-specific RVAs into a structured reference or configuration system.
SDK testing: Add binary fixtures based on captured snapshots and replay them without a running game.
Configuration: Introduce qcheat.toml.
Diagnostics and logging: Add a qcheat doctor command and structured logging with tracing.
TUI: Explore a terminal user interface.
Server-side research: Investigate sv_snapshot.c and the location of authoritative g_entities[] state in a locally hosted server, with corresponding documentation.

The roadmap is tentative and may change as the project evolves.

References
ioquake3 source headers:
code/qcommon/q_shared.h
code/cgame/cg_public.h
code/client/client.h
code/server/sv_snapshot.c
windows-rs: Windows API bindings, including process memory access and input APIs.
bytemuck: Utilities for working with plain-data representations.
Project documentation:
docs/cahier-des-charges.md
docs/reverse-engineering.md
docs/comprendre-le-projet.md

Some documentation files describe ongoing or planned work and may not yet exist in the repository.

License

The code in this project is licensed under MIT.

ioquake3 is licensed under GPL-2.0. Its authors retain ownership of their work. This repository does not include or redistribute ioquake3 source code.
