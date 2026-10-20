# Cheat_Quake1 (`qcheat`)

Boîte à outils Rust **externe** (hors processus) pour étudier l'état d'un client **ioquake3** en cours d'exécution sous Windows x86_64 : position, HP, angles de vue et liste complète des entités de la frame courante. Projet d'apprentissage autour du reverse-engineering, du scan mémoire et de la correspondance octet par octet entre structures C et structures Rust.

ioquake3 est sous licence GPL-2.0. Ce dépôt ne redistribue aucun de ses codes.

> **Périmètre.** Recherche pédagogique sur une partie **locale** (`/devmap` + `/addbot`), contre des **bots**, sur la même machine. Le projet n'est pas conçu pour des serveurs en ligne et ne doit pas y être utilisé. Voir [Usage responsable](#usage-responsable).

---

## Ce que fait réellement le projet

Le binaire unique `qcheat` expose 12 sous-commandes. Elles se répartissent en trois familles.

### Lecture seule (étude)

| Commande | Rôle |
|---|---|
| `find` | Trouve le processus, affiche PID et base du module principal. |
| `modules [--filter]` | Liste les modules (exe + DLL) chargés. |
| `hp <addr>` | Interroge un entier 32 bits à une adresse fixe (ex. HP côté moteur). |
| `inspect <addr> --mode origin\|raw\|vec3` | Interprète une adresse comme `entityState_t`. |
| `scan <center> [--range]` | Scanne une fenêtre pour des motifs `entityState_t` et détecte les arrays par stride. |
| `players` | Scan du heap filtré sur les entités `ET_PLAYER`. |
| `snapshot` | Localise `cg.activeSnapshots[2]` (signature : deux candidats espacés de 53 772 octets), puis affiche le joueur local et toutes les entités visibles. |
| `server-entities` | **Listen server uniquement.** Retrouve `g_entities[]` (état autoritatif du serveur) en prenant les joueurs du snapshot client comme référence, déduit `sizeof(gentity_t)`, recoupe avec `sv.gentities` du moteur, puis liste les joueurs du serveur et marque ceux que le client n'a **pas** reçus (filtrés par le PVS). Refuse de tourner si plus d'un joueur n'est pas un bot. |
| `find-viewangles` | Retrouve par élimination (valeur connue + rondes de confirmation pendant que tu bouges la souris) l'adresse du yaw de rendu. |

### Interaction avec le jeu 

| Commande | Rôle | Remarque |
|---|---|---|
| `aimbot` | Vise le joueur le plus proche en **écrivant** les angles (`WriteProcessMemory`) dans les adresses de vue et dans 32 copies de snapshot. | Expérimental : les adresses écrites ne sont pas toutes celles qui alimentent le rendu (voir [limites](#limites-et-points-connus)). Pas de touche d'activation, tourne en boucle. |
| `aim-mouse` | Calcule l'erreur angulaire vers le joueur le plus proche et envoie des déplacements de souris relatifs via `SendInput`. Lissage, plafond par tick, bascule par touche (Insert par défaut), prédiction de la cible entre deux snapshots. | N'écrit pas en mémoire : passe par le chemin d'entrée normal du jeu. Les entrées partent vers la fenêtre au premier plan. |
| `esp` | Overlay transparent « click-through » dessinant des boîtes sur les joueurs de la snapshot, avec extrapolation linéaire entre deux snapshots. | Voir la limite PVS ci-dessous. |

### Hors périmètre actuel
Configuration `qcheat.toml`, TUI, `doctor`, logging structuré : **planifiés, non présents dans le code**. `server-entities` est écrit et testé sur mémoire simulée, **à valider en jeu** (voir `docs/server-reverse.md`).

---

## Architecture

Quatre crates ; chaque flèche signifie « dépend de ».

```
            cli (qcheat)
          /    |     \
      sdk   scanner   process
                \       ^
                 +------+
```

| Crate | Responsabilité | Dépend de |
|---|---|---|
| `sdk` | Miroirs `#[repr(C)]` : `Vec3`, `Trajectory`, `EntityState`, `PlayerState`, `Snapshot`/`SnapshotHeader`, `SharedEntity`/`EntityShared` (tête de `gentity_t`). Filtres de plausibilité communs (`plausibility`) et logique serveur pure (jumeau serveur, inférence du stride, règle « bots seulement »). Assertions de taille et d'offsets à la compilation. Aucune dépendance Windows, aucun I/O. | `bytemuck` |
| `process` | Accès au processus externe : `ProcessHandle` (lecture **et écriture**), énumération Toolhelp32. Ne connaît pas Quake. | `windows`, `thiserror` |
| `scanner` | Scan par chunks alignés avec prédicat fourni par l'appelant, détection de strides. Ne connaît pas Quake. | `process` |
| `cli` | Binaire `qcheat`. Module `locate` : localisation du snapshot **commune** à toutes les commandes. Seule crate qui connaît à la fois Quake et l'API Windows (SendInput, GDI). | toutes |

`offsets.json` regroupe les tailles de structures, les offsets de champs et les RVA du moteur observés.

---

## Build et utilisation

```powershell
cargo build --release          # target\release\qcheat.exe
cargo test --workspace         # sdk (16 tests : filtres, stride, règle bots) + scanner::stride (4)
.\target\release\qcheat.exe --help
```

`qcheat` est une application console : lance-la depuis PowerShell ou cmd (un double-clic ferme la fenêtre immédiatement).

Flux typique, jeu lancé, carte chargée, bots présents :

```powershell
qcheat find
qcheat snapshot
qcheat inspect 0x0613F728 --mode raw     # adresse d'exemple : change à chaque lancement
qcheat find-viewangles                   # puis bouger la souris pendant les rondes
qcheat aim-mouse --toggle-key 0x2D       # Insert pour activer/désactiver
qcheat esp --fov 90
```

Valeurs par défaut utiles : fenêtre de scan `0x07000000 ± 0x02000000` ; `aim-mouse` : `--sensitivity 5.0 --m-yaw 0.022 --m-pitch 0.022 --smooth 0.15 --max-delta 60 --interval-ms 8` ; `esp` : `--interval-ms 8 --fov 90`. Les paramètres de souris doivent correspondre à tes cvars `sensitivity`, `m_yaw`, `m_pitch`.

> Les adresses absolues (heap QVM) changent à chaque lancement. Les RVA (relatifs à la base de l'exe) sont stables tant que le binaire ioquake3 n'est pas recompilé, et ceux du dépôt correspondent au build observé.

---

## Comment ça marche

**Pourquoi un scan par signature ?** Avec `cgame.qvm`, la structure `cg` vit dans le heap de la machine virtuelle QVM, dont l'adresse est dynamique. On ne peut pas coder une adresse en dur ; on cherche une forme.

**La signature du snapshot.** Chaque offset aligné sur 4 octets est traité comme un `SnapshotHeader` candidat et passe un filtre de plausibilité (`pm_type`, `clientNum`, `weapon`, origine finie, HP plausible…). Deux candidats distants d'exactement `sizeof(snapshot_t) = 53 772` octets sont presque sûrement `cg.activeSnapshots[0]` et `[1]` ; on retient celui dont `serverTime` est le plus grand (le snapshot actif). Une fois trouvé, il est lu en un seul `ReadProcessMemory`.

**Le piège `tr_base`.** Pour une entité joueur, la position fiable est `pos.trBase`, pas `origin` (laissé à zéro pour les entités interpolées).

**Pourquoi `aim-mouse` existe.** Les écritures mémoire de `aimbot` retombaient sur des copies (historique de snapshots) qui n'alimentent pas le rendu. Plutôt que de chercher la bonne adresse, `aim-mouse` suit le chemin d'entrée du jeu (`CL_MouseEvent` → `cl.viewangles` → rendu). Les corrections sont intégrées localement et resynchronisées à chaque nouveau snapshot, car le serveur n'en envoie que ~20-40 par seconde.

**Pourquoi l'ESP est limité (PVS).** Le serveur ne transmet à un client que les entités de son PVS (Potentially Visible Set). Ce que lit `esp` est donc exactement ce que le client reçoit : c'est une limite du protocole, pas du code. Pour voir toutes les entités il faut lire l'état autoritatif du serveur (`g_entities[]`), accessible seulement si **tu héberges** la partie. Ce volet est planifié (voir `docs/server-reverse.md` et la commande `server-entities`).

**Validation de layout.** Chaque miroir porte des `const _: () = assert!(...)` sur la taille et des offsets clés (ex. `EntityState` = 208, `PlayerState` = 468, `Snapshot` = 53 772). Ces assertions vérifient la cohérence de **ton miroir Rust** avec les valeurs de référence ; la validation face au binaire réellement exécuté repose sur le filtre de plausibilité à l'exécution.

---

## Limites et points connus

- **Windows x86_64 uniquement**, ioquake3 avec `cgame.qvm` (interpréteur), pas de DLL cgame.
- **RVA propres à un build.** Les valeurs de `aimbot.rs` (`0x7B409C`, `0x7B4098`, `0x7B9988`, stride `0x21C`) sont des constantes en dur observées empiriquement ; elles ne sont pas dans une couche de configuration et changeront avec un autre build.
- **`aimbot` mémoire : fiabilité partielle**, d'où `aim-mouse`. Ciblage simple (le plus proche, sans filtre d'équipe, sans test de vivant dans `aimbot`).
- **Garde-fou « partie locale » seulement sur `server-entities`** (`aimbot`, `aim-mouse` et `esp` n'en ont pas encore).
- Tests : le SDK est testé (filtres, stride, règle bots) mais pas les commandes `cli` ; pas encore de fixture de snapshot réel.
- `server-entities` : hypothèses à confirmer en jeu (offset `svFlags`, fenêtre de scan, fin de `server_t`) ; voir `docs/server-reverse.md` §6.
- `offsets.json` contient encore des références à d'anciens binaires (`memory --bin dump-snapshot`).

---

## Usage responsable

- À exécuter **uniquement** sur ta machine, dans une partie que tu héberges, contre des bots.
- Ne pas utiliser contre des joueurs réels ni sur des serveurs publics ; ce serait contraire aux règles de ces serveurs et à l'esprit du projet.
- Pas de contournement d'anti-cheat, pas de masquage du processus, pas de diffusion d'un binaire précompilé.
- **Fait pour `server-entities`** : la commande refuse d'afficher quoi que ce soit si plus d'un joueur n'est pas un bot.
- **Engagement de développement (priorité 1) :** étendre une vérification technique qui refuse les commandes d'écriture mémoire, d'injection d'entrée et d'overlay quand la session n'est pas une partie hébergée localement. Tant que ce garde-fou n'existe pas, le respect de ce périmètre repose sur l'utilisateur.

---

## Roadmap

1. Garde-fou « partie locale » (partagé par `aimbot`, `aim-mouse`, `esp`).
2. ~~Factoriser la localisation du snapshot~~ (fait : `cli/src/locate.rs`, filtres dans `sdk`). Reste : déplacer les RVA de `aimbot.rs` dans `offsets.json`/config.
3. Tests sur fixtures (snapshot binaire capturé, rejoué hors jeu) pour le SDK.
4. `qcheat.toml`, `qcheat doctor`, logging `tracing`, puis TUI.
5. Volet serveur : valider `server-entities` en jeu, étudier `sv_snapshot.c` en détail, compléter `docs/server-reverse.md`.

---

## Références

- ioquake3 : `code/qcommon/q_shared.h`, `code/cgame/cg_public.h`, `code/client/client.h`, `code/server/sv_snapshot.c`
- `windows-rs` (ReadProcessMemory, WriteProcessMemory, Toolhelp32, SendInput, GDI)
- `bytemuck`
- Documentation du projet : `docs/cahier-des-charges.md`, `docs/reverse-engineering.md`, `docs/comprendre-le-projet.md`

## Licence

Code du projet : MIT. Les noms de champs, tailles et offsets de structures proviennent des headers ioquake3 (GPL-2.0), qui reste la propriété de ses auteurs ; aucun code ioquake3 n'est inclus.
