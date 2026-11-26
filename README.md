# Cheat_Quake1 — `qcheat`

**English** · [Français](#français)

External Rust toolkit for reading and steering a running **ioquake3** client on Windows x86_64. It attaches to the game process from the outside, locates the engine's data structures in memory, and builds on them: entity dumps, aim assistance, a screen overlay, and a comparison between what the server knows and what the client receives.

The project is a hands-on study of memory reverse-engineering: mirroring C structures byte for byte in Rust, finding them again by signature in a heap whose addresses change on every launch, and reading the client/server split of a real-time game.

> **Language note.** This README is in English, with a French translation at the bottom. The project documents ([`docs/`](docs/)) and the technical write-ups ([`ClientSide/`](ClientSide/), [`ServerSide/`](ServerSide/)) are written in French.

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
11. [References](#references)
12. [License](#license)

---

## Features

| Area | What it does | Commands |
|---|---|---|
| Recon | Find the process, list its modules, poll a single value | `find`, `modules`, `hp` |
| Memory | Interpret bytes as game structures, scan for them, dump the live snapshot | `inspect`, `scan`, `players`, `snapshot` |
| Server | Read the server's authoritative entity array and compare it with the client's view | `server-entities` |
| Aim | Locate the view-angle variable; steer the camera by memory write or by mouse input | `find-viewangles`, `aimbot`, `aim-mouse` |
| Overlay | Draw boxes over the players of the current snapshot; with `--server`, also over players the server filtered out of the client's view | `esp` |

All features ship in a single executable, `qcheat`, with one self-documented subcommand per operation.

---

## Requirements

- Windows 10/11, x86_64
- Rust 1.77 or newer (`rustup` stable is fine) and the MSVC build tools (Visual Studio Build Tools, "Desktop development with C++")
- ioquake3 (`ioquake3.x86_64.exe`) running a locally hosted map, with bots:
  ```
  /devmap q3dm17
  /addbot sarge
  ```
  A hosted game keeps client and server code in one process, which `server-entities` and `esp --server` rely on. All other commands only need a running client.
- For `esp`: the game in windowed or borderless mode (an exclusive full-screen game hides the overlay).

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
qcheat esp --fov 90 --debug       # overlay: red boxes = players in the client snapshot
qcheat esp --fov 90 --server --sv-rva 0x1C0DB30 --debug   # + green boxes = players hidden by the PVS
qcheat find-viewangles            # move the mouse while it runs
qcheat aim-mouse                  # starts enabled; Insert toggles it
```

Absolute addresses inside the game heap change on every launch. Addresses relative to the executable base (RVA) stay valid for a given ioquake3 build. The RVA used above (`0x1C0DB30`) was measured on the tested build; `server-entities` prints the value for another build.

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

Useful flags: `--tolerance` (max client/server position gap, default 150), `--stride` (force `sizeof(gentity_t)`), `--sv-rva` (RVA of `sv.gentities`, skips the scan on later runs), `--center` / `--range` (window for `g_entities[]`), `--snap-center` / `--snap-range` (window for the snapshot).

Measured on the tested build: `sizeof(gentity_t)` = 804 bytes, `sv.gentities` at `exe + 0x1C0DB30` (also recorded in `offsets.json`).

### Aim

| Command | Purpose |
|---|---|
| `find-viewangles` | Isolate the live view-angle variable by elimination over several rounds while the mouse moves. |
| `aimbot` | Aim at the closest player by writing view angles into game memory. Experimental: some written addresses are copies that do not feed the renderer. |
| `aim-mouse` | Compute the angular error to the closest living player and send relative mouse movement through `SendInput`. Smoothing, per-tick cap, toggle key, and target prediction between snapshots. Starts enabled; the toggle key (`--toggle-key`, default Insert, `0x2D`) switches it off and on. |

`aim-mouse` defaults: `--sensitivity 5.0 --m-yaw 0.022 --m-pitch 0.022 --smooth 0.15 --max-delta 60 --interval-ms 8 --toggle-key 0x2D`. The sensitivity values must match the game's `sensitivity`, `m_yaw`, and `m_pitch` cvars. Input goes to the foreground window. Targets are taken from the client snapshot only (no visibility test).

### Overlay

| Command | Purpose |
|---|---|
| `esp [--fov 90] [--interval-ms 8] [--debug]` | Transparent click-through window projecting snapshot players to screen coordinates (red boxes, `#n`), with linear extrapolation between snapshots. `--debug` prints one status line per second. |
| `esp --server [--sv-rva <hex>]` | Additionally reads `g_entities[]` and draws, in green (`SV#n`), the players the server knows but did not send to the client. Other flags: `--sv-center`, `--sv-range`, `--sv-tolerance`, `--sv-stride`. |

Set `--fov` to the game's `cg_fov` (horizontal). The camera is `ps.origin + viewheight`; boxes span `origin − 24` to `origin + 32`.

---

## How it works

**1. Process access.** `OpenProcess` returns a handle; `ReadProcessMemory` and `WriteProcessMemory` operate on the target from a separate process. No code runs inside the game.

**2. Structure mirrors.** `entityState_t`, `playerState_t`, and `snapshot_t` are redefined in Rust with `#[repr(C)]`. Compile-time assertions fix their sizes (`EntityState` = 208, `PlayerState` = 468, `Snapshot` = 53 772) and key field offsets, so a mistake in a mirror breaks the build instead of producing garbage at runtime. These assertions check the Rust mirror against reference values; they do not inspect the target binary.

**3. Finding the snapshot.** The cgame VM keeps `cg` in a dynamically allocated heap, so no fixed address exists. Each 4-byte-aligned offset is treated as a candidate `SnapshotHeader` and passed through range checks (`pm_type`, `clientNum`, `weapon`, origin, HP). Two candidates exactly 53 772 bytes apart identify `cg.activeSnapshots[0..2]`; the one with the larger `serverTime` is the active snapshot. Range checks alone accept false positives, so `esp` adds three controls: `serverTime` must be realistic, it must advance over time, and with `--server` the snapshot's player position must match the server entity of the same client.

**4. Entity positions.** For player entities the reliable position is `pos.trBase`; the plain `origin` field is zero for interpolated entities. It is the centre of the collision box, not the eye.

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
├── docs/                      project documents (French)
│   ├── cahier-des-charges.md      project specification, phases, status
│   ├── architecture.md            architecture plan, data flow, study path
│   └── comprendre-le-projet.md    concepts, glossary, typical questions
├── ClientSide/                client-side technical study (French)
│   └── reverse-engineering.md     from the first HP value to the full snapshot
├── ServerSide/                server-side study (French)
│   ├── README.md                  index: where the server-side code lives
│   └── server-reverse.md          g_entities[], PVS, in-game measurements
└── crates/
    ├── sdk/                   struct mirrors, constants, pure logic
    │   └── src/  vector, trajectory, entitystate, playerstate, snapshot,
    │             constants, server, plausibility
    ├── process/               Windows process access
    │   └── src/  discovery (Toolhelp32), handle (Read/WriteProcessMemory)
    ├── scanner/               generic memory-scan primitives
    │   └── src/  scan (aligned scan), stride (array detection)
    └── cli/                   the `qcheat` binary
        └── src/
            ├── main.rs        argument parsing and dispatch
            ├── util.rs        shared helpers (hex parsing, defaults)
            ├── locate.rs      shared snapshot and g_entities[] locators
            └── cmd/           one file per subcommand
                find, modules, hp, inspect, scan, players, snapshot,
                find_viewangles, aimbot, aim_mouse, esp, server_entities
```

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

Covered today: `sdk` (snapshot filters, server twin matching, stride inference; layout assertions run at compile time) and `scanner::stride`. Commands that need a live game are checked manually against a running ioquake3; the continuous-integration workflow builds and tests the workspace on Windows.

---

## Known limitations

- Windows x86_64 only; ioquake3 with the cgame VM (QVM).
- The server-side features (`server-entities`, `esp --server`) work only in a locally hosted game. Against a remote server, `g_entities[]` is not in the process memory.
- Several engine RVAs are tied to one ioquake3 build (`sv.gentities` in `offsets.json`; others hard-coded in `aimbot.rs` and not yet listed there).
- The live-snapshot locator (time progression, server cross-check) is implemented in `esp.rs` only. `aim-mouse`, `aimbot` and `find-viewangles` still use the simple locator in `locate.rs`; moving the improved one into `locate.rs` is the next planned refactor.
- `aimbot` (memory write) is partly ineffective because some target addresses are copies; `aim-mouse` is the reliable path. `aim-mouse` aims from the entity origin rather than the eye, targets the closest player without a visibility test, and has not been validated in game yet.
- No configuration file, interactive UI, or structured logging yet.

---

## Documentation

All files below are in French.

| File | Content |
|---|---|
| [`docs/cahier-des-charges.md`](docs/cahier-des-charges.md) | Project specification, phases, status |
| [`docs/architecture.md`](docs/architecture.md) | Architecture plan, data flow, how to add a command, study path |
| [`docs/comprendre-le-projet.md`](docs/comprendre-le-projet.md) | Concepts, glossary, typical questions |
| [`ClientSide/reverse-engineering.md`](ClientSide/reverse-engineering.md) | Client-side study: from the first HP value to the full snapshot |
| [`ServerSide/README.md`](ServerSide/README.md) | Server-side index: commands, source files, measured values |
| [`ServerSide/server-reverse.md`](ServerSide/server-reverse.md) | Server-side study: `g_entities[]`, PVS, in-game measurements |

---|---|
| [`docs/cahier-des-charges.md`](docs/cahier-des-charges.md) | Project specification, phases, status |
| [`docs/architecture.md`](docs/architecture.md) | Structure, data flow, how to add a command, study path |
| [`ClientSide/reverse-engineering.md`](ClientSide/reverse-engineering.md) | Client-side walkthrough: from the first HP value to the full snapshot |
| [`docs/comprendre-le-projet.md`](docs/comprendre-le-projet.md) | Concepts, glossary, typical questions |
| [`ServerSide/server-reverse.md`](ServerSide/server-reverse.md) | Server-side study: `g_entities[]`, PVS, in-game measurements, server-side ESP |

---

## References

- [ioquake3 source](https://github.com/ioquake/ioq3) — `code/qcommon/q_shared.h`, `code/game/g_public.h`, `code/cgame/cg_public.h`, `code/server/sv_snapshot.c`
- [windows-rs](https://github.com/microsoft/windows-rs) — `ReadProcessMemory`, Toolhelp32 bindings
- [bytemuck](https://github.com/Lokathor/bytemuck) — safe `Pod` casting

---

## License

MIT. Structure names, sizes, and offsets come from the public ioquake3 headers (GPL-2.0); no ioquake3 code is included or redistributed.

---

<a id="français"></a>

# Français

[English](#cheat_quake1--qcheat) · **Français**

Boîte à outils Rust externe pour lire et piloter un client **ioquake3** en cours d'exécution sous Windows x86_64. Elle s'attache au processus du jeu de l'extérieur, localise en mémoire les structures de données du moteur et s'appuie dessus : dump d'entités, aide à la visée, overlay à l'écran, et comparaison entre ce que le serveur connaît et ce que le client reçoit.

Le projet est une étude pratique du reverse-engineering mémoire : recopier des structures C octet par octet en Rust, les retrouver par signature dans un heap dont les adresses changent à chaque lancement, et lire la séparation client/serveur d'un jeu temps réel.

> **Note de langue.** Le README est en anglais, avec cette traduction française en bas de page. Les documents de projet ([`docs/`](docs/)) et les études techniques ([`ClientSide/`](ClientSide/), [`ServerSide/`](ServerSide/)) sont rédigés en français.

---

## Sommaire

1. [Fonctionnalités](#fonctionnalités)
2. [Prérequis](#prérequis)
3. [Démarrage rapide](#démarrage-rapide)
4. [Référence des commandes](#référence-des-commandes)
5. [Fonctionnement](#fonctionnement)
6. [Organisation du dépôt](#organisation-du-dépôt)
7. [Architecture](#architecture-1)
8. [Tests](#tests)
9. [Limites connues](#limites-connues)
10. [Documentation](#documentation-1)
11. [Références](#références)
12. [Licence](#licence)

---

## Fonctionnalités

| Domaine | Rôle | Commandes |
|---|---|---|
| Reconnaissance | Trouver le processus, lister ses modules, suivre une valeur | `find`, `modules`, `hp` |
| Mémoire | Interpréter des octets comme des structures du jeu, les scanner, afficher le snapshot courant | `inspect`, `scan`, `players`, `snapshot` |
| Serveur | Lire le tableau d'entités du serveur et le comparer à la vue du client | `server-entities` |
| Visée | Localiser la variable d'angle de vue ; piloter la caméra par écriture mémoire ou par entrée souris | `find-viewangles`, `aimbot`, `aim-mouse` |
| Overlay | Dessiner des boîtes sur les joueurs du snapshot courant ; avec `--server`, aussi sur ceux que le serveur a filtrés de la vue du client | `esp` |

Toutes les fonctionnalités sont regroupées dans un seul exécutable, `qcheat`, avec une sous-commande documentée par opération.

---

## Prérequis

- Windows 10/11, x86_64
- Rust 1.77 ou plus récent (`rustup` stable convient) et les outils de compilation MSVC (Visual Studio Build Tools, « Développement Desktop en C++ »)
- ioquake3 (`ioquake3.x86_64.exe`) lancé sur une carte hébergée localement, avec des bots :
  ```
  /devmap q3dm17
  /addbot sarge
  ```
  Une partie hébergée garde le code client et le code serveur dans un seul processus, ce dont dépendent `server-entities` et `esp --server`. Les autres commandes n'ont besoin que d'un client en cours d'exécution.
- Pour `esp` : le jeu en mode fenêtré ou sans bordure (le plein écran exclusif masque l'overlay).

---

## Démarrage rapide

```powershell
cargo build --release
.\target\release\qcheat.exe --help
```

`qcheat` est une application console : elle se lance depuis PowerShell ou cmd. Un double-clic dans l'Explorateur ouvre et ferme aussitôt une fenêtre.

Session type, jeu lancé et carte chargée :

```powershell
qcheat find                       # PID et base du module
qcheat snapshot                   # localise le snapshot vivant, affiche le joueur local et les entités visibles
qcheat server-entities            # compare la liste des joueurs du serveur avec le snapshot
qcheat esp --fov 90 --debug       # overlay : boîtes rouges = joueurs du snapshot client
qcheat esp --fov 90 --server --sv-rva 0x1C0DB30 --debug   # + boîtes vertes = joueurs cachés par le PVS
qcheat find-viewangles            # bouger la souris pendant l'exécution
qcheat aim-mouse                  # démarre activé ; Insert le bascule
```

Les adresses absolues dans le heap du jeu changent à chaque lancement. Les adresses relatives à la base de l'exécutable (RVA) restent valables pour un build d'ioquake3 donné. Le RVA utilisé ci-dessus (`0x1C0DB30`) a été mesuré sur le build testé ; `server-entities` affiche la valeur pour un autre build.

---

## Référence des commandes

Chaque commande accepte `--process <nom>` (par défaut `ioquake3.x86_64.exe`) et `--help`.

### Reconnaissance

| Commande | Rôle |
|---|---|
| `find` | Affiche le PID et l'adresse de base du module principal. |
| `modules [--filter <texte>]` | Liste les modules chargés (exe et DLL). |
| `hp <addr>` | Suit un entier 32 bits à une adresse fixe (HP côté moteur). |

### Mémoire

| Commande | Rôle |
|---|---|
| `inspect <addr> --mode origin\|raw\|vec3` | Affiche une adresse comme un `entityState_t`. |
| `scan <centre> [plage]` | Scanne une fenêtre (arguments hexadécimaux positionnels, demi-plage par défaut `0x80000`) à la recherche de blocs de forme `entityState_t` et détecte les tableaux par leur stride. |
| `players` | Scan de tout le heap filtré sur les entités `ET_PLAYER`. |
| `snapshot` | Localise `cg.activeSnapshots[2]` et affiche le joueur local et chaque entité visible. |

`players` et `snapshot` scannent par défaut la fenêtre `0x07000000 ± 0x02000000` ; `--center` et `--range` (hexadécimal) la remplacent.

### Serveur

| Commande | Rôle |
|---|---|
| `server-entities` | Localise `g_entities[]` dans une partie hébergée, déduit `sizeof(gentity_t)`, recoupe avec `sv.gentities` du moteur, puis liste chaque joueur côté serveur et signale ceux absents du snapshot client (retirés par le PVS). |

Options utiles : `--tolerance` (écart maximal de position client/serveur, 150 par défaut), `--stride` (impose `sizeof(gentity_t)`), `--sv-rva` (RVA de `sv.gentities`, évite le scan aux lancements suivants), `--center` / `--range` (fenêtre de `g_entities[]`), `--snap-center` / `--snap-range` (fenêtre du snapshot).

Mesuré sur le build testé : `sizeof(gentity_t)` = 804 octets, `sv.gentities` à `exe + 0x1C0DB30` (également noté dans `offsets.json`).

### Visée

| Commande | Rôle |
|---|---|
| `find-viewangles` | Isole la variable d'angle de vue en cours d'utilisation par élimination sur plusieurs rondes, souris en mouvement. |
| `aimbot` | Vise le joueur le plus proche en écrivant les angles dans la mémoire du jeu. Expérimental : certaines adresses écrites sont des copies qui n'alimentent pas le rendu. |
| `aim-mouse` | Calcule l'erreur angulaire vers le joueur vivant le plus proche et envoie des déplacements relatifs de souris via `SendInput`. Lissage, plafond par tick, touche de bascule, prédiction de cible entre deux snapshots. Démarre activé ; la touche de bascule (`--toggle-key`, Insert par défaut, `0x2D`) l'éteint et le rallume. |

Valeurs par défaut de `aim-mouse` : `--sensitivity 5.0 --m-yaw 0.022 --m-pitch 0.022 --smooth 0.15 --max-delta 60 --interval-ms 8 --toggle-key 0x2D`. Les valeurs de sensibilité doivent correspondre aux cvars `sensitivity`, `m_yaw` et `m_pitch` du jeu. L'entrée est envoyée à la fenêtre au premier plan. Les cibles viennent du snapshot client uniquement (pas de test de visibilité).

### Overlay

| Commande | Rôle |
|---|---|
| `esp [--fov 90] [--interval-ms 8] [--debug]` | Fenêtre transparente insensible aux clics qui projette les joueurs du snapshot en coordonnées écran (boîtes rouges, `#n`), avec extrapolation linéaire entre deux snapshots. `--debug` affiche une ligne d'état par seconde. |
| `esp --server [--sv-rva <hex>]` | Lit en plus `g_entities[]` et dessine en vert (`SV#n`) les joueurs que le serveur connaît mais n'a pas envoyés au client. Autres options : `--sv-center`, `--sv-range`, `--sv-tolerance`, `--sv-stride`. |

Régler `--fov` sur le `cg_fov` du jeu (horizontal). La caméra est `ps.origin + viewheight` ; les boîtes vont de `origin − 24` à `origin + 32`.

---

## Fonctionnement

**1. Accès au processus.** `OpenProcess` renvoie un handle ; `ReadProcessMemory` et `WriteProcessMemory` agissent sur la cible depuis un processus distinct. Aucun code ne s'exécute dans le jeu.

**2. Miroirs de structures.** `entityState_t`, `playerState_t` et `snapshot_t` sont redéfinies en Rust avec `#[repr(C)]`. Des assertions à la compilation fixent leurs tailles (`EntityState` = 208, `PlayerState` = 468, `Snapshot` = 53 772) et des offsets clés : une erreur dans un miroir casse la compilation au lieu de produire des données absurdes à l'exécution. Ces assertions comparent le miroir Rust à des valeurs de référence ; elles n'examinent pas le binaire cible.

**3. Trouver le snapshot.** La VM cgame garde `cg` dans un heap alloué dynamiquement : aucune adresse fixe n'existe. Chaque offset aligné sur 4 octets est traité comme un `SnapshotHeader` candidat et passe des tests de plages (`pm_type`, `clientNum`, `weapon`, origine, HP). Deux candidats distants d'exactement 53 772 octets désignent `cg.activeSnapshots[0..2]` ; celui dont le `serverTime` est le plus grand est le snapshot actif. Les tests de plages seuls acceptent des faux positifs ; `esp` ajoute donc trois contrôles : le `serverTime` doit être réaliste, il doit avancer dans le temps, et avec `--server` la position du joueur du snapshot doit correspondre à l'entité serveur du même client.

**4. Positions des entités.** Pour les joueurs, la position fiable est `pos.trBase` ; le champ `origin` est nul pour les entités interpolées. C'est le centre de la boîte de collision, pas l'œil.

**5. Côté serveur.** Le serveur n'envoie que les entités du PVS du client (ensemble potentiellement visible) : le snapshot est donc une vue filtrée. `g_entities[]`, géré par le module serveur, contient toutes les entités. Dans une partie hébergée, il se trouve dans le même processus. Il est retrouvé en faisant correspondre les joueurs du snapshot avec la mémoire (même numéro d'entité, position proche, bloc partagé cohérent), en déduisant le stride de deux correspondances (`stride = Δadresse / Δnuméro`) et en le confirmant par le pointeur moteur `sv.gentities`.

**6. Piloter la caméra.** `aimbot` écrit les angles directement. `aim-mouse` suit le chemin d'entrée du jeu (déplacement souris → angles de vue → rendu) et atteint donc la variable réellement utilisée par le rendu.

---

## Organisation du dépôt

```
Cheat_Quake1/
├── Cargo.toml                 définition du workspace et dépendances communes
├── offsets.json               tailles de structures, offsets de champs, RVA du moteur
├── README.md
├── LICENSE
├── .github/workflows/ci.yml   build + tests sous Windows
├── docs/                      documents de projet (français)
│   ├── cahier-des-charges.md      spécification du projet, phases, état
│   ├── architecture.md            plan d'architecture, flux de données, parcours d'étude
│   └── comprendre-le-projet.md    concepts, glossaire, questions types
├── ClientSide/                étude technique côté client (français)
│   └── reverse-engineering.md     de la première valeur de HP au snapshot complet
├── ServerSide/                étude côté serveur (français)
│   ├── README.md                  index : où se trouve le code serveur
│   └── server-reverse.md          g_entities[], PVS, mesures en jeu
└── crates/
    ├── sdk/                   miroirs de structures, constantes, logique pure
    ├── process/               accès aux processus Windows
    ├── scanner/               primitives génériques de scan mémoire
    └── cli/                   le binaire `qcheat`
        └── src/
            ├── main.rs        arguments et dispatch
            ├── util.rs        utilitaires communs
            ├── locate.rs      localisation partagée du snapshot et de g_entities[]
            └── cmd/           un fichier par sous-commande
```

---

## Architecture

Quatre crates avec des dépendances à sens unique : `cli` dépend de `sdk`, `scanner` et `process` ; `scanner` dépend de `process` ; `sdk` ne dépend d'aucune autre crate du projet et ne connaît ni Windows ni les E/S.

Règles :

- La logique pure va dans `sdk`, pour être testable sur n'importe quelle machine.
- Une sous-commande est un fichier de `cmd/` avec `Args` et `run` ; le code de scan partagé va dans `locate.rs`.
- Les bibliothèques renvoient des erreurs typées (`thiserror`) ; le CLI utilise `anyhow`.
- Les éléments publics des bibliothèques sont documentés (`#![warn(missing_docs)]`).

Détails et ordre de lecture pas à pas : [`docs/architecture.md`](docs/architecture.md).

---

## Tests

```powershell
cargo test --workspace
```

Couvert aujourd'hui : `sdk` (filtres de snapshot, correspondance du jumeau serveur, déduction du stride ; les assertions de layout s'exécutent à la compilation) et `scanner::stride`. Les commandes qui exigent un jeu en cours sont vérifiées manuellement sur ioquake3 ; le workflow d'intégration continue compile et teste le workspace sous Windows.

---

## Limites connues

- Windows x86_64 uniquement ; ioquake3 avec la VM cgame (QVM).
- Les fonctions côté serveur (`server-entities`, `esp --server`) ne marchent qu'en partie hébergée localement. Contre un serveur distant, `g_entities[]` n'est pas dans la mémoire du processus.
- Plusieurs RVA du moteur sont liés à un build d'ioquake3 (`sv.gentities` dans `offsets.json` ; les autres sont en dur dans `aimbot.rs` et pas encore listés).
- La localisation du snapshot vivant (progression du temps, recoupement avec le serveur) n'existe que dans `esp.rs`. `aim-mouse`, `aimbot` et `find-viewangles` utilisent encore la version simple de `locate.rs` ; la remonter dans `locate.rs` est le prochain refactor prévu.
- `aimbot` (écriture mémoire) est en partie inefficace car certaines adresses visées sont des copies ; `aim-mouse` est la voie fiable. `aim-mouse` vise depuis l'origine de l'entité et non depuis l'œil, cible le joueur le plus proche sans test de visibilité, et n'a pas encore été validé en jeu.
- Pas de fichier de configuration, d'interface interactive ni de journalisation structurée pour l'instant.

---

## Documentation

Tous les fichiers ci-dessous sont en français.

| Fichier | Contenu |
|---|---|
| [`docs/cahier-des-charges.md`](docs/cahier-des-charges.md) | Spécification du projet, phases, état |
| [`docs/architecture.md`](docs/architecture.md) | Plan d'architecture, flux de données, ajout d'une commande, parcours d'étude |
| [`docs/comprendre-le-projet.md`](docs/comprendre-le-projet.md) | Concepts, glossaire, questions types |
| [`ClientSide/reverse-engineering.md`](ClientSide/reverse-engineering.md) | Étude côté client : de la première valeur de HP au snapshot complet |
| [`ServerSide/README.md`](ServerSide/README.md) | Index côté serveur : commandes, fichiers sources, valeurs mesurées |
| [`ServerSide/server-reverse.md`](ServerSide/server-reverse.md) | Étude côté serveur : `g_entities[]`, PVS, mesures en jeu |

---|---|
| [`docs/cahier-des-charges.md`](docs/cahier-des-charges.md) | Spécification du projet, phases, état |
| [`docs/architecture.md`](docs/architecture.md) | Structure, flux de données, ajout d'une commande, parcours d'étude |
| [`ClientSide/reverse-engineering.md`](ClientSide/reverse-engineering.md) | Parcours côté client : de la première valeur de HP au snapshot complet |
| [`docs/comprendre-le-projet.md`](docs/comprendre-le-projet.md) | Concepts, glossaire, questions types |
| [`ServerSide/server-reverse.md`](ServerSide/server-reverse.md) | Étude côté serveur : `g_entities[]`, PVS, mesures en jeu, ESP côté serveur |

---

## Références

- [Sources d'ioquake3](https://github.com/ioquake/ioq3) — `code/qcommon/q_shared.h`, `code/game/g_public.h`, `code/cgame/cg_public.h`, `code/server/sv_snapshot.c`
- [windows-rs](https://github.com/microsoft/windows-rs) — liaisons `ReadProcessMemory`, Toolhelp32
- [bytemuck](https://github.com/Lokathor/bytemuck) — conversions `Pod` sûres

---

## Licence

MIT. Les noms de structures, tailles et offsets proviennent des en-têtes publics d'ioquake3 (GPL-2.0) ; aucun code d'ioquake3 n'est inclus ni redistribué.
