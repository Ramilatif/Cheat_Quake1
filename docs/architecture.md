# Architecture et parcours d'étude

Ce document décrit l'organisation du dépôt, le chemin d'une commande de bout en bout, la marche à suivre pour ajouter une sous-commande, et un parcours de lecture progressif pour maîtriser le projet étape par étape.

---

## 1. Vue d'ensemble

```
                 ioquake3 (processus cible)
                          ▲
                          │ ReadProcessMemory / WriteProcessMemory / SendInput
                          │
 ┌────────────────────────┴───────────────────────────┐
 │ process   : accès au processus (handle, modules)   │
 │ scanner   : primitives de scan génériques          │
 │ sdk       : structures du jeu + logique pure       │
 │ cli       : commandes qcheat (assemble le tout)    │
 └────────────────────────────────────────────────────┘
```

Principe directeur : chaque couche ne connaît que ce dont elle a besoin.

| Crate | Connaît le jeu ? | Connaît Windows ? | Testable hors Windows ? |
|---|---|---|---|
| `sdk` | Oui (structures, constantes) | Non | Oui |
| `process` | Non | Oui | Non |
| `scanner` | Non | Via `process` | Non |
| `cli` | Oui | Oui | Non |

Conséquence pratique : toute logique qui peut être exprimée sur des structures déjà lues (filtres de plausibilité, correspondance serveur/client, inférence du stride) est placée dans `sdk`, où elle est vérifiée par des tests unitaires.

---

## 2. Arborescence commentée

```
crates/
├── sdk/src/
│   ├── vector.rs         Vec3
│   ├── trajectory.rs     Trajectory (trType, trBase, trDelta…)
│   ├── entitystate.rs    EntityState (208 o), EntityType
│   ├── playerstate.rs    PlayerState (468 o), indices de stats
│   ├── snapshot.rs       SnapshotHeader, Snapshot (53 772 o)
│   ├── constants.rs      MAX_CLIENTS, MAX_GENTITIES, …
│   ├── server.rs         [ajout] SharedEntity (tête de gentity_t), jumeau serveur, stride
│   └── plausibility.rs   [ajout] filtres de snapshot (Loose / Strict)
├── process/src/
│   ├── discovery.rs      find_by_name, list_modules (Toolhelp32)
│   └── handle.rs         ProcessHandle : read, read_into, write
├── scanner/src/
│   ├── scan.rs           scan_aligned::<T> avec prédicat
│   └── stride.rs         detect_repeating_stride
└── cli/src/
    ├── main.rs           parsing clap + dispatch
    ├── util.rs           parse_hex, nom de processus par défaut
    ├── locate.rs         [ajout] localisation du snapshot (partagée)
    └── cmd/              une sous-commande par fichier
        ├── find.rs, modules.rs, hp.rs
        ├── inspect.rs, scan.rs, players.rs, snapshot.rs
        ├── find_viewangles.rs, aimbot.rs, aim_mouse.rs
        ├── esp.rs
        └── server_entities.rs   [ajout] volet serveur
```

Les éléments marqués `[ajout]` constituent le volet serveur et la factorisation du locator ; le reste de l'arborescence est inchangé.

---

## 3. Règles de rangement

| Situation | Emplacement |
|---|---|
| Nouvelle structure du jeu (miroir `repr(C)`) | `sdk/src/` + assertion de taille |
| Règle de validation ou calcul sur des structures déjà lues | `sdk/src/` + tests |
| Nouvelle primitive de scan sans connaissance du jeu | `scanner/src/` |
| Nouvelle opération sur le processus (API Windows) | `process/src/` |
| Nouvelle commande | `cli/src/cmd/` (un fichier) |
| Code de recherche partagé entre commandes | `cli/src/locate.rs` |
| Valeur d'offset ou de taille à documenter | `offsets.json` |

Conventions : erreurs typées avec `thiserror` dans les bibliothèques, `anyhow` dans `cli` ; documentation obligatoire sur les éléments publics des bibliothèques ; pas de `unwrap()` ni de `panic!()` dans les bibliothèques.

---

## 4. Chemin d'une commande : `qcheat snapshot`

1. `main.rs` : clap lit les arguments et appelle `cmd::snapshot::run`.
2. `process::find_by_name` retrouve le PID et la base du module ; `ProcessHandle::open` ouvre le processus.
3. `locate::find_snapshot_candidates` parcourt la fenêtre mémoire par blocs de 4 096 octets. Chaque offset aligné sur 4 octets est lu comme un `SnapshotHeader` et passé à `sdk::looks_like_snapshot` (filtre `Strict`).
4. Deux candidats distants de 53 772 octets (`sizeof(snapshot_t)`) désignent `cg.activeSnapshots[0..2]` ; celui dont `serverTime` est le plus grand est le snapshot actif.
5. Le snapshot complet est lu en un seul appel, puis les entités sont parcourues jusqu'à `num_entities`.
6. L'affichage utilise `pos.trBase` comme position des joueurs.

Les commandes `aimbot`, `aim-mouse`, `esp` et `find-viewangles` reprennent les étapes 2 à 4 via `locate.rs`, puis ajoutent leur logique propre.

---

## 5. Ajouter une sous-commande

1. Créer `cli/src/cmd/ma_commande.rs` avec :
   - `pub struct Args` dérivée de `clap::Args` (flags, valeurs par défaut) ;
   - `pub fn run(args: Args) -> anyhow::Result<()>`.
2. Déclarer le module dans `cmd/mod.rs` (`pub mod ma_commande;`).
3. Ajouter la variante dans l'énumération `Cmd` de `main.rs`, avec un commentaire `///` (il devient le texte d'aide), et la branche correspondante dans le `match`.
4. Si la commande contient de la logique pure, la placer dans `sdk` et l'y tester.
5. Mettre à jour le README (tableau de la commande) et, si besoin, `offsets.json`.

---

## 6. Parcours d'étude

Chaque étape indique quoi lire, quoi lancer, et ce qu'il faut pouvoir expliquer ensuite. Le jeu tourne avec une carte hébergée localement (`/devmap`, `/addbot`).

| # | Lire | Lancer | À pouvoir expliquer |
|---|---|---|---|
| 1 | `process/src/handle.rs`, `discovery.rs` | `qcheat find`, `qcheat modules` | Ce qu'est un handle de processus ; pourquoi lire la mémoire d'un autre processus est possible ; ce qu'est la base d'un module. |
| 2 | `sdk/src/vector.rs`, `entitystate.rs`, `snapshot.rs` | `cargo test -p sdk` | Pourquoi `#[repr(C)]` ; ce que vérifient (et ne vérifient pas) les assertions de taille. |
| 3 | `scanner/src/scan.rs`, `stride.rs` | `qcheat scan <adresse>` | Scan par blocs alignés ; détection d'un tableau par écart constant. |
| 4 | `sdk/src/plausibility.rs`, `cli/src/locate.rs` | `qcheat snapshot` | Pourquoi une signature structurelle (deux candidats à 53 772 octets) remplace une adresse fixe. |
| 5 | `docs/reverse-engineering.md` | `qcheat inspect <adresse> --mode raw` | Chemin du premier HP trouvé à la snapshot complète ; rôle de `trBase`. |
| 6 | `docs/server-reverse.md`, `sdk/src/server.rs` | `qcheat server-entities` | Pourquoi le serveur voit plus que le client (PVS) ; comment le stride de `gentity_t` est déduit. |
| 7 | `cli/src/cmd/find_viewangles.rs` | `qcheat find-viewangles` | Isolation d'une variable par rondes de confirmation. |
| 8 | `cli/src/cmd/aimbot.rs`, `aim_mouse.rs` | `qcheat aim-mouse` | Différence entre écrire dans la mémoire et passer par le chemin d'entrée du jeu ; boucle de lissage. |
| 9 | `cli/src/cmd/esp.rs` | `qcheat esp` | Projection monde → écran ; fenêtre transparente ; extrapolation entre deux snapshots. |

Ordre conseillé : 1 → 6 pour la compréhension du moteur et du protocole, 7 → 9 pour les modules d'interaction.

---

## 7. Compilation, tests, intégration continue

```powershell
cargo build --release       # target\release\qcheat.exe
cargo test --workspace      # tests de sdk et de scanner
```

Le workflow `.github/workflows/ci.yml` compile et teste le workspace sous Windows à chaque push et pull request. Les tests de `sdk` s'exécutent aussi sous Linux ; `process`, `scanner` et `cli` nécessitent Windows.

---

## 8. Points à consolider

| Point | Piste |
|---|---|
| RVA en dur dans `cmd/aimbot.rs` (`0x7B409C`, `0x7B4098`, `0x7B9988`) | Les déplacer dans `offsets.json` ou une configuration, avec le build d'ioquake3 associé. |
| Nommage incohérent des variables d'angle (`cg.refdefViewAngles`, `cl.snap.ps.viewangles`) | Harmoniser les noms dans le code et dans `offsets.json`. |
| Tests limités à `sdk` et `scanner::stride` | Ajouter une fixture de snapshot capturé en jeu pour tester les parseurs hors ligne. |
| Réglages passés par flags à chaque lancement | Fichier `qcheat.toml` (config) puis `qcheat doctor`. |
| Hypothèses du volet serveur non encore confirmées en jeu | Voir `docs/server-reverse.md` §6. |
