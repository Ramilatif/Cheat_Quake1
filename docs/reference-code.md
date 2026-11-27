# Référence du code — Cheat_Quake1

Document de référence sur le code : ce que fait chaque crate, chaque
sous-commande, et les mécanismes transversaux qui reviennent partout
(localisation par signature, extrapolation de vélocité, projection
3D→2D, fenêtre overlay). Ce document complète [architecture.md](architecture.md) (organisation du dépôt, parcours d'étude) ; le volet serveur (`server-entities`, `esp --server`) est détaillé dans [../ServerSide/server-reverse.md](../ServerSide/server-reverse.md). Pour la démarche narrative — pourquoi chaque
choix a été fait, les fausses pistes, les bugs trouvés — voir
[reverse-engineering.md](../ClientSide/reverse-engineering.md). Ce document-ci est
la coupe transversale : qu'est-ce qui existe, où, et comment ça
s'articule.

---

## 1. Vue d'ensemble

`qcheat` est un unique binaire Rust, workspace Cargo à cinq crates,
qui lit (et dans deux cas précis, pilote) l'état d'un process
`ioquake3.x86_64.exe` en cours d'exécution sur la même machine,
exclusivement par API Windows externes (`ReadProcessMemory`,
`WriteProcessMemory`, `Toolhelp32`, `SendInput`, GDI). Aucune
injection de code, aucun hook, aucun patch du binaire cible.

```
crates/
  sdk/       miroirs #[repr(C)] des structures ioquake3, zéro dépendance Windows
  process/   accès process externe (ProcessHandle, discovery)
  scanner/   primitives de scan génériques (scan_aligned, stride)
  config/    qcheat.toml : chargement + fusion avec les flags CLI
  cli/       binaire qcheat — seule crate à connaître Quake ET Windows
```

Dépendances strictement unidirectionnelles :

```
      sdk ───────────────┐
       ▲                 │
       │                 │
   scanner   config      │
       ▲       ▲         │
       │       │         │
    process ◄──┴─────────┤
       ▲                 │
       │                 │
      cli ◄───────────────┘
```

`sdk` ne sait rien de Windows ni du process externe — juste des
structures C mises en miroir. `process` ne sait rien de Quake —
juste comment lire/écrire dans un process externe. `scanner` ne
connaît ni l'un ni l'autre — juste comment scanner un buffer et
détecter des motifs. `config` ne connaît que TOML et des nombres.
`cli` est la seule crate qui importe tout le monde et où Quake et
Windows se rencontrent.

---

## 2. `sdk` — miroirs des structures ioquake3

Chaque structure C d'ioquake3 dont on a besoin est retranscrite en
Rust `#[repr(C)]`, avec des `const _: () = assert!(...)` sur la
taille totale et les offsets de champs clés. Si l'engine change un
champ, **la compilation casse** — aucune lecture silencieuse de bytes
décalés.

| Module | Type | Taille | Rôle |
|---|---|---|---|
| `vector` | `Vec3` | 12 B | `vec3_t` — x=avant, y=gauche, z=haut |
| `trajectory` | `Trajectory` | 36 B | `trajectory_t` — trType + trTime + trDuration + trBase + trDelta |
| `entitystate` | `EntityState` | 208 B | `entityState_t` — état réseau de n'importe quelle entité du monde |
| `playerstate` | `PlayerState` | 468 B | `playerState_t` — vue complète du joueur local (origin, velocity, viewangles, stats[]) |
| `snapshot` | `SnapshotHeader` / `Snapshot` | 516 B / 53 772 B | `snapshot_t` — une frame cgame complète (header + jusqu'à 256 `EntityState`) |

Points notables :

- **`EntityState.pos` est une `Trajectory`, pas juste un `Vec3`.**
  Pour un joueur (`TR_INTERPOLATE`), c'est `pos.trBase` qui contient
  la vraie position — `EntityState.origin` reste à zéro, laissé à
  l'interpolation client. Piège découvert et documenté en détail dans
  [reverse-engineering.md §6](../ClientSide/reverse-engineering.md).
- **`SnapshotHeader` est lu seul (516 B) pour valider un candidat**
  avant de payer la lecture complète de 53 772 B — `looks_like_snapshot()`,
  dupliquée dans chaque commande qui scanne (voir §7).
- **`EntityType`** (dans `entitystate.rs`) expose les discriminants
  `eType` en constantes `i32` plutôt qu'un `enum` Rust, pour qu'une
  valeur inconnue (un mod qui ajoute un type d'entité) reste lisible
  au lieu de undefined behavior.
- **Pas de champ HP par ennemi.** `PlayerState.stats[STAT_HEALTH]`
  n'existe que pour *soi-même* — `entityState_t` ne transporte jamais
  le HP d'un autre joueur. C'est la raison documentée pour laquelle
  le mode de ciblage "HP le plus faible" (prévu au cahier des charges)
  n'a pas été implémenté dans `qcheat menu`.

---

## 3. `process` — accès au process externe

Deux modules :

- **`discovery`** : `find_by_name(name) -> Result<Process, ProcessError>`
  énumère les process via un snapshot Toolhelp32 (`CreateToolhelp32Snapshot`
  + `Process32First`/`Next`) et retourne PID + adresse de base du
  module principal. `list_modules(pid)` fait la même chose pour les
  DLL chargées.
- **`handle`** : `ProcessHandle` encapsule un `HANDLE` Windows ouvert
  via `OpenProcess(PROCESS_VM_READ | PROCESS_VM_WRITE | PROCESS_QUERY_INFORMATION, ..)`,
  fermé automatiquement au `Drop`. Expose :
  - `read<T: Pod>(&self, address) -> Result<T, ProcessError>` — copie
    typée d'une valeur depuis l'espace d'adressage cible.
  - `read_into(&self, address, buf: &mut [u8])` — lecture brute, pour
    le scan par chunks où la taille n'est connue qu'à l'exécution.
  - `write<T: Pod>(&self, address, value)` / `write_f32(..)` —
    `WriteProcessMemory` typé. Conservé pour `qcheat aimbot` (voir
    §7) mais **inutilisé par le chemin qui fonctionne réellement**
    (`aim-mouse`/`menu`, qui pilotent l'input plutôt que d'écrire).

Toute erreur (échec d'`OpenProcess`, lecture partielle, etc.) remonte
comme `ProcessError` via `thiserror` — jamais de `panic!`/`unwrap()`
en dehors des tests.

---

## 4. `scanner` — primitives de scan génériques

Deux briques, testées unitairement, sans aucune connaissance de Quake :

- **`scan_aligned<T, F>(handle, start, end, alignment, predicate) -> Vec<Hit<T>>`**
  (`scan.rs`) — parcourt `[start, end)` par blocs de 4 KiB (taille
  d'une page Windows), réinterprète chaque offset aligné comme un
  `T: Pod`, et garde ceux qui passent le prédicat fourni par
  l'appelant. C'est le moteur derrière toute localisation par
  signature du projet.
- **`detect_repeating_stride(addresses, min_run, max_stride) -> Option<StrideMatch>`**
  (`stride.rs`) — étant donné des adresses de hits triées, trouve la
  plus longue série séparée par un écart constant. Une vraie array
  de `T` produit une chaîne d'écarts tous égaux à `sizeof(T)` ; c'est
  statistiquement quasiment jamais du bruit.

Les commandes de plus haut niveau (`snapshot`, `menu`, `esp`,
`doctor`, ...) réimplémentent une variante spécialisée de ce même
principe en dur (`find_snapshot_candidates` + `looks_like_snapshot`,
dupliqué par fichier plutôt que factorisé — voir §9) plutôt que
d'appeler `scan_aligned` directement, pour pouvoir arrêter le scan dès
qu'un candidat passe le filtre sans repasser par une closure générique
à chaque offset.

---

## 5. `config` — `qcheat.toml`

Ajouté pour `qcheat menu`, qui a fini par accumuler trop de réglages
pour rester confortable en flags CLI retapés à chaque lancement.

- **`MenuConfig`** — struct `serde` avec `#[serde(default)]` sur
  chaque champ : process, interval_ms, m_yaw/m_pitch, menu_key/
  aimbot_key (touches, stockées en string `"0x70"` ou décimal),
  aimbot_enabled, sensitivity, smooth, max_delta, target_mode
  (`"closest"` | `"smallest_angle"`), esp_enabled, fov. Une clé
  absente du fichier retombe silencieusement sur `MenuConfig::default()`.
- **`load(path)`** / **`load_or_default(explicit_path)`** — cherche
  `./qcheat.toml` si aucun chemin explicite n'est donné ; absence de
  fichier = defaults silencieux, mais un `--config <path>` explicite
  qui échoue est une vraie erreur (ne doit pas être ignoré en silence).
- **`parse_vkey(s)`** — accepte `"0x70"`/`"0X70"` (hex) ou `"112"`
  (décimal), mêmes formes que `--menu-key`/`--aimbot-key` côté CLI.

`qcheat.example.toml` (racine du repo) documente chaque clé ; un test
(`example_toml_parses_and_matches_documented_defaults`) le parse et
vérifie qu'il correspond exactement à `MenuConfig::default()` — le
fichier ne peut pas silencieusement se désynchroniser du code.

Priorité de résolution partout dans `qcheat menu` : **flag CLI >
`qcheat.toml` > défaut intégré**. Concrètement, tous les champs
CLI concernés dans `menu::Args` sont des `Option<T>` sans
`default_value` — le vrai défaut vit dans `MenuConfig::default()`.

---

## 6. `cli` — le binaire `qcheat`

`main.rs` déclare un `enum Cmd` (`clap::Subcommand`) et dispatche vers
`cmd::<nom>::run(args)`. Chaque sous-commande est un fichier sous
`crates/cli/src/cmd/`, avec le même contrat : `pub struct Args`
(`clap::Args`) + `pub fn run(args: Args) -> anyhow::Result<()>`.

`util.rs` porte deux utilitaires partagés : `DEFAULT_PROCESS`
(`"ioquake3.x86_64.exe"`) et `parse_hex` (parseur clap pour les
adresses/touches en `0x..`).

### 6.1 Outils de lecture seule

| Commande | Rôle |
|---|---|
| `find` | Localise le process, retourne PID + adresse de base. |
| `modules` | Énumère les DLL chargées. |
| `hp` | Poll d'un `i32` à une adresse fixe (HP côté engine). |
| `inspect` | Interprète une adresse arbitraire comme `EntityState`, modes `origin\|raw\|vec3`. |
| `scan` | Scan brut d'une fenêtre mémoire pour des motifs `EntityState`, détection de stride. |
| `players` | Scan heap complet filtré sur `EntityType::PLAYER`. |
| `snapshot` | Localise `cg.activeSnapshots[2]` par signature (paire à exactement 53 772 B d'écart), dump le joueur local + toutes les entités visibles. |

`snapshot` est la brique fondatrice : c'est sa logique de localisation
(candidats par `SnapshotHeader` filtré + confirmation par la paire à
53 772 B) qui est reprise, quasi identique, dans `aim-mouse`, `esp`,
`menu` et `doctor`.

### 6.2 Diagnostic

**`qcheat doctor`** — contrôle d'environnement en une passe : process
trouvé, `OpenProcess` accordé en lecture+écriture, fenêtre du jeu
localisable, une vraie paire `cg.activeSnapshots` actuellement
présente (pas juste un candidat), `SendInput` accepté par l'OS (un
événement à delta nul, donc non perturbant), `qcheat.toml` valide.
Sortie : une ligne `[OK]`/`[WARN]`/`[FAIL]` par check + un résumé.
Pensé pour être la première commande à lancer après une mise à jour
d'ioquake3, ou quand quelque chose qui marchait s'arrête de marcher.

### 6.3 Aimbot — deux générations, une seule qui marche

- **`qcheat aimbot`** (conservé pour référence) — approche naïve :
  calcule l'angle vers le joueur le plus proche, `WriteProcessMemory`
  dans les copies de `cl.snapshots[]`. Compile, s'exécute, **n'a
  aucun effet sur la caméra** : `PM_UpdateViewAngles` (`bg_pmove.c`)
  recalcule l'angle depuis `cmd->angles` à chaque frame prédite,
  côté client, peu importe ce qui a été écrit dans n'importe quelle
  copie en mémoire. Détail complet des fausses pistes (buffer
  d'historique, adresse de pile, miroir en tas) dans
  [reverse-engineering.md §9](../ClientSide/reverse-engineering.md).
- **`qcheat find-viewangles`** — scanner de diagnostic utilisé pendant
  la chasse à une adresse d'angle inexistante : réduit les candidats
  flottants par comparaison à une référence connue lue depuis le
  snapshot.
- **`qcheat aim-mouse`** — l'approche qui marche : injecte le delta
  d'angle calculé comme mouvement de souris relatif via `SendInput`,
  ne touche jamais à la mémoire du jeu. Contient toute la mécanique de
  lissage (voir §7.3) : extrapolation de la position de la cible par
  vélocité, intégration locale du propre angle du bot entre deux
  vrais snapshots, resynchronisation sur `serverTime`.

### 6.4 Wallhack

**`qcheat esp`** — projette `pos.trBase` de chaque ennemi vivant en
coordonnées écran (§7.4) et dessine une boîte dans une fenêtre GDI
transparente, cliquable-à-travers, toujours au-dessus (§7.5). Même
extrapolation de vélocité que l'aimbot pour un suivi fluide entre
deux vrais snapshots — une `HashMap<client_num, Track>` par ennemi,
purgée dès qu'il disparaît d'un vrai snapshot.

### 6.5 `qcheat menu` — tout réuni

Point d'entrée principal. Fusionne aimbot + wallhack sur **une seule**
lecture de snapshot partagée par tick, plus un menu overlay piloté au
clavier (le jeu capture le curseur système pour le mouselook — la
souris n'est jamais une option fiable ici).

- **`Settings`** — struct mutée directement dans la boucle
  (`aimbot_enabled`, `sensitivity`, `smooth`, `max_delta`,
  `target_mode: TargetMode`, `esp_enabled`, `fov`). Un seul thread,
  pas d'`Arc<Mutex<..>>` : le menu, le calcul de correction et le
  dessin tournent dans la même itération.
- **`MenuItem`** — trois variantes pilotant chacune une ligne du menu :
  `Toggle` (case à cocher), `Slider` (valeur continue, min/max/step),
  `Cycle` (liste fixe d'options — utilisé pour `TargetMode`). Chaque
  variante porte des pointeurs de fonction `get`/`set` vers un champ
  de `Settings`, ce qui permet à `menu_items()` de rester une simple
  liste déclarative.
- **`TargetMode`** — `Closest` (distance) ou `SmallestAngle` (angle
  minimal depuis la visée actuelle, via `angle_error_magnitude`). Le
  troisième mode prévu au cahier des charges, "HP le plus faible",
  n'a pas été implémenté — cf. §2, la donnée n'existe pas côté client.
- Chargement de config : voir §5. Chaque flag CLI de `Args` est un
  `Option<T>`, fusionné avec `config::MenuConfig` avant construction
  de `Settings`.

---

## 7. Mécanismes transversaux

Ces cinq techniques reviennent, sous une forme quasi identique, dans
plusieurs commandes — elles ne sont volontairement pas factorisées
dans un module partagé (chaque commande garde sa propre copie), pour
rester lisible indépendamment sans devoir suivre une indirection vers
un crate commun. C'est un choix de lisibilité, pas un oubli.

### 7.1 Localisation par signature

`find_snapshot_candidates(handle, start, end)` scanne une fenêtre
mémoire par blocs de 4 KiB, réinterprète chaque offset 4-aligné comme
`SnapshotHeader`, et garde ceux qui passent `looks_like_snapshot`
(filtre cross-champ strict : `num_entities ∈ 1..=256`, `server_time`
plausible, `client_num`/`weapon`/`pm_type` dans leurs bornes, origine
finie et dans les limites de la map, HP plausible). Le signal décisif
est de trouver **deux** candidats séparés d'exactement
`sizeof(Snapshot) = 53 772` octets — c'est `cg.activeSnapshots[0..2]`,
rien d'autre dans le process n'a cette propriété. Le plus grand
`server_time` des deux = `cg.snap` (la frame active).

### 7.2 Cache de snapshot + détection de figement

Refaire ce scan à chaque tick coûterait ~1-2 s de
`ReadProcessMemory` — inacceptable à ~120 Hz. `aim-mouse`, `esp` et
`menu` mettent donc en cache les adresses trouvées et ne revalident
que par lecture directe (`SnapshotHeader` + `looks_like_snapshot`)
à chaque tick. Le rescan complet ne se déclenche que sur un vrai
symptôme :

- **Lecture échouée** → cache invalidé immédiatement.
- **`server_time` figé depuis plusieurs secondes** (`menu.rs`
  uniquement, le raffinement le plus récent) → signature exacte d'un
  lancement avant d'être en partie, ou d'un faux positif verrouillé
  sur de la mémoire sans rapport. Une première version rescannait sur
  une minuterie aveugle toutes les ~5 s ; corrigé après avoir
  reproduit des coupures de 2 s périodiques en jeu — voir
  [reverse-engineering.md §13](../ClientSide/reverse-engineering.md).

### 7.3 Extrapolation de vélocité + intégration d'angle local

Le serveur ne tique qu'à `sv_fps` (~20-40 Hz), largement sous la
boucle de poll (~120 Hz). Recalculer une correction sur la même
donnée plusieurs fois avant d'en voir l'effet cause de l'oscillation.
Solution, identique dans `aim-mouse`, `esp` et `menu` :

- **Cible** : à chaque vrai changement de `server_time`,
  `vel = (nouvelle_position - ancienne_position) / dt_serveur`: à
  chaque tick, `position_affichée = dernière_position + vel × temps_écoulé`.
  Une `HashMap<client_num, Track { pos, vel, wall: Instant }>` par
  ennemi côté `esp`/`menu` ; une seule cible suivie côté `aim-mouse`.
- **Soi-même** (aimbot uniquement) : `own_yaw`/`own_pitch` sont
  intégrés localement (`own_yaw -= deg_par_count × dx_envoyé`) plutôt
  que relus depuis le snapshot à chaque tick, et resynchronisés
  seulement à chaque vrai `server_time`. Le wallhack, lui, relit
  `ps.origin`/`viewangles` frais à chaque tick sans intégration — ce
  champ reflète `cg.predictedPlayerState`, recalculé chaque frame de
  rendu par la prédiction client, donc déjà à jour.

### 7.4 Injection d'input (`SendInput`) au lieu d'écriture mémoire

`send_mouse_delta(dx, dy)` construit un `INPUT { type: INPUT_MOUSE, .. }`
et l'envoie via `SendInput` (`Win32::UI::Input::KeyboardAndMouse`).
Le moteur ne distingue pas ce mouvement d'un vrai mouvement de souris
physique — il traverse le même `CL_MouseMove`. Point de signe
important, retrouvé dans `cl_input.c` :
`cl.viewangles[YAW] -= m_yaw * mx` (mx positif = souris à droite =
yaw qui **diminue**) — le delta calculé doit inverser le signe pour
le yaw, pas pour le pitch (`+=` direct).

### 7.5 Projection 3D → 2D (wallhack)

`angle_vectors(yaw, pitch)` reproduit `AngleVectors` du moteur (roll
supposé nul) pour obtenir la base caméra (avant/droite/haut).
`world_to_screen(world, eye, forward, right, up, largeur, hauteur, fov)`
projette un point sur cette base, rejette si `cz < 1` (derrière la
caméra), et met à l'échelle avec le FOV. Une boîte par joueur se
construit en projetant deux points (pieds = `pos.trBase`, tête =
pieds + hauteur approx.) plutôt qu'un point + taille fixe, pour une
mise à l'échelle correcte avec la distance.

### 7.6 Fenêtre overlay (GDI, Win32)

`create_overlay_window` crée une fenêtre `WS_POPUP` avec :

- `WS_EX_LAYERED` + `SetLayeredWindowAttributes(.., LWA_COLORKEY)` :
  couleur clé (noir) rendue transparente ; fond noir puis formes
  dessinées par-dessus en GDI direct (`Rectangle`, `TextOutW`), sans
  double buffering (léger scintillement accepté).
- `WS_EX_TRANSPARENT` : clics traversent vers le jeu — retiré
  temporairement (`set_click_through(overlay, false)`, via
  `GetWindowLongPtrW`/`SetWindowLongPtrW` sur `GWL_EXSTYLE`) tant que
  le menu de `qcheat menu` est ouvert, pour ne pas déclencher d'action
  en jeu par erreur.
- `WS_EX_TOPMOST`, réaffirmé à **chaque tick** via `SetWindowPos(..,
  HWND_TOPMOST, .., SWP_NOMOVE|SWP_NOSIZE)` — Windows démet une
  fenêtre topmost au bout d'un moment (focus repris par le jeu,
  alt-tab, notification) si on ne le redemande qu'une fois à la
  création.
- Position/taille resynchronisées périodiquement sur la fenêtre du
  jeu (`game_window_rect`, toutes les ~1 s dans `menu.rs`), pour
  rattraper un lancement pendant un écran de chargement à une
  résolution différente de la partie.

---

## 8. Tests

- `scanner::stride` — 4 tests unitaires sur `detect_repeating_stride`
  (run trouvé, rejet sous le seuil, rejet au-dessus de `max_stride`,
  entrée vide).
- `config` — 4 tests : round-trip TOML sur les defaults, clés
  manquantes qui retombent sur les defaults, parsing des touches
  hex/décimal, et `qcheat.example.toml` qui doit parser et matcher
  exactement `MenuConfig::default()` (garde-fou anti-désynchronisation
  doc/code).
- Assertions à la compilation dans `sdk` (§2) — pas des tests
  d'exécution, mais la même fonction : détecter une dérive de layout
  au build plutôt qu'au runtime.

Aucun test end-to-end contre un vrai process ioquake3 (nécessiterait
le jeu lancé en CI) — hors de portée actuelle du projet, listé comme
piste à moyen terme (fixtures rejouées hors-ligne) dans le cahier des
charges.

---

## 9. Choix de conception assumés

- **Duplication délibérée plutôt que factorisation prématurée.**
  `find_snapshot_candidates`/`looks_like_snapshot`/`Chunk` existent en
  quasi-copie dans `aim_mouse.rs`, `esp.rs`, `menu.rs`, `doctor.rs`,
  `find_viewangles.rs`. Chaque commande reste lisible et modifiable
  seule ; le coût est une dérive possible si le filtre change dans un
  fichier et pas les autres.
- **Pas de threads.** `qcheat menu` fait tout (poll mémoire, calcul
  aimbot, dessin overlay, lecture clavier) dans une seule boucle
  séquentielle. Plus simple à raisonner qu'une synchronisation
  `Arc<Mutex<..>>`, au prix d'un tick qui bloque si une étape est
  lente (raison directe du bug de rescan aveugle, §7.2).
- **Menu clavier, jamais souris.** Le jeu capture/cache le curseur
  système pour le mouselook ; `GetCursorPos` n'aurait aucun rapport
  fiable avec "où l'utilisateur pointe" pendant une partie active.
- **Deux générations d'aimbot gardées dans l'arbre.** `qcheat aimbot`
  (mort) n'est pas supprimé malgré son inutilité pratique — c'est la
  preuve tangible d'un résultat de reverse-engineering négatif mais
  instructif, cohérent avec la finalité pédagogique du projet.

---

## 10. Pour aller plus loin

Backlog technique (détail et justification dans
[reverse-engineering.md §15](../ClientSide/reverse-engineering.md) et dans la
comparaison avec le cahier des charges) :

- Wallhack **serveur-side** (`g_entities[]`, vrai bypass du PVS) — le
  volet le plus ambitieux prévu, non entamé.
- Sélection de cible par filtre de FOV/angle max, trigger configurable
  (hold / clic-droit), limite de vitesse de rotation en degrés/s
  explicite.
- `qcheat.toml` : persistance automatique des adresses trouvées entre
  deux lancements (auto-discovery), avec re-validation par
  `serverTime` croissant.
- Logging structuré (`tracing`, `--log-level`) à la place des
  `println!` actuels.
- Fixtures de test (snapshot binaire capturée, rejouée hors-ligne)
  pour tester les parseurs sans le jeu actif, en CI.
