# Reverse engineering de ioquake3 — du process Windows à la liste des joueurs

Ce document retrace la démarche concrète qui mène à `dump-snapshot` :
comment, à partir d'un seul `ioquake3.x86_64.exe` qui tourne et de la
seule connaissance que "quelque part en mémoire il y a une liste de
joueurs", on en arrive à lire **frame par frame** la position et le HP
de chaque joueur visible. Et où se trouve la limite que rien ne peut
contourner côté client.

---

## 1. Point de départ

On dispose de trois choses :

1. Le **process cible** : `ioquake3.x86_64.exe` qui tourne, PID
   accessible via Toolhelp32.
2. Une **adresse trouvée à la main** dans Cheat Engine : la valeur du
   HP du joueur local change exactement avec le HUD. RVA noté dans
   `offsets.json` : `0x7B40B8`.
3. Le **code source ioquake3** sur GitHub, GPL, qui définit toutes les
   structures que le binaire utilise.

L'objectif initial : transformer "je sais lire un `i32`" en "je sais
lire la liste de tous les joueurs visibles avec leurs positions". La
règle qu'on se fixe : aucune injection, aucun hook, uniquement de la
lecture externe via `ReadProcessMemory`.

---

## 2. Pourquoi le HP seul ne suffit pas

L'adresse `0x7B40B8` est isolée. Lire les bytes autour ne donne ni la
position du joueur, ni la liste des ennemis : le HP vit dans une
structure (`playerState_t.stats[STAT_HEALTH]`) mais on ne sait pas
encore *quelle copie* de cette structure on a sous la main — il y en a
au moins trois :

- `cl.snap.ps.stats[]` côté **engine** (dans le `.exe`, RVA fixe)
- `cg.snap->ps.stats[]` côté **cgame** (dans le heap, runtime)
- les snapshots de backup `cl.snapshots[PACKET_BACKUP]`

Pour aller plus loin il faut lire le source et établir la **forme** de
ce qu'on cherche, pas l'adresse.

---

## 3. Cartographier les structures depuis le source

Quatre headers ioquake3 portent tout ce qui nous intéresse :

| Header | Ce qu'il définit |
|---|---|
| `code/qcommon/q_shared.h` | `vec3_t`, `trajectory_t`, `entityState_t`, `playerState_t` |
| `code/cgame/cg_public.h` | `snapshot_t` (la vue cgame d'un frame) |
| `code/client/client.h` | `clSnapshot_t`, `clientActive_t cl` (vue engine) |
| `code/cgame/cg_local.h` | `cg_t cg`, `cg_entities[MAX_GENTITIES]` |

On les **transpose en Rust** dans le crate `sdk/` avec `#[repr(C)]` et
des `const _: () = assert!(size_of::<T>() == N)`. Si l'engine change un
champ, la build casse — pas de lecture silencieuse de bytes décalés.

Tailles importantes calculées sur x86_64 Windows :

```
vec3_t          = 12 B
trajectory_t    = 36 B   (trType + trTime + trDuration + trBase + trDelta)
entityState_t   = 208 B
playerState_t   = 468 B
clSnapshot_t    = 540 B  (engine)
snapshot_t      = 53 772 B (cgame — contient entities[256] en clair)
```

Pour `playerState_t` par exemple, le calcul à la main donne :

```
stats commence à offset 184 dans playerState_t
ps commence à offset 44 dans snapshot_t (cgame) ou 60 dans clSnapshot_t (engine)
```

Donc `cg.snap.ps.stats[STAT_HEALTH = 0]` est à `snapshot_t + 44 + 184`
= **offset 228** du début du snapshot.

---

## 4. Première fausse piste : cl.parseEntities[]

Le crate `memory/bin/scan_entities.rs` cherche dans le heap des blocs
de bytes qui *ressemblent* à des `entityState_t` valides — filtre :
`number ∈ 0..1024`, `eType ∈ 0..=12`, `clientNum ∈ -1..64`, origine
finie, weapon ∈ 0..=15, etc.

Lancé autour de `0x06800000 ± 24 MiB`, il trouve un bloc compact :

```
0x05F88040  number=77   e_type=2  client_num=0  origin=(1120, 2316, 48)
0x05F88110  number=81   e_type=2  client_num=0  origin=( 672, 1288, 16)
0x05F881E0  number=85   e_type=2  client_num=0  origin=( 668, 2096, 16)
0x05F882B0  number=86   e_type=2  client_num=0  origin=( 384, 1472, 16)
...
```

Strides : `0x110 - 0x040 = 0xD0 = 208` bytes. Exactement
`sizeof(entityState_t)`. Le tableau existe.

On pense d'abord à `cl.parseEntities[MAX_PARSE_ENTITIES = 8192]` (un
ring buffer dans la struct engine `clientActive_t cl`). Mais ce
tableau est **dans le `.exe`**, à une RVA fixe — pas dans le heap à
`0x05F8xxxx`. La mémoire de l'exe va de `0x400000` à environ
`0x2044000`, et `0x05F88040` est largement au-delà.

Donc ce qu'on a trouvé n'est **pas** l'engine. C'est du **cgame**, et
le cgame tourne en **QVM** (`cgame.qvm`, bytecode VM, heap alloué
dynamiquement par le moteur). `modules.txt` confirme : aucun
`cgamex86_64.dll` chargé, c'est bien la VM.

---

## 5. Bonne piste : cg.activeSnapshots[2]

Dans `cg_local.h` :

```c
typedef struct {
    ...
    snapshot_t *snap;          // pointeur sur la frame active
    snapshot_t *nextSnap;      // pointeur sur la frame suivante
    snapshot_t activeSnapshots[2];  // les deux frames stockées en clair
    ...
} cg_t;
```

Le cgame garde **deux** `snapshot_t` complètes côte-à-côte, et `cg.snap`
pointe sur l'une des deux. Chaque snapshot pèse 53 772 B et contient
`entities[256]` en clair — tout pile ce dont on a besoin pour un ESP.

L'idée pour les localiser :

1. Scanner le heap, lire 516 bytes à chaque offset aligné sur 4
2. Réinterpréter comme `SnapshotHeader` (snap_flags, ping, serverTime,
   areamask, ps, num_entities)
3. Filtrer strict : `client_num ∈ 0..64`, `pm_type ∈ 0..=8`, `weapon ∈
   0..=15`, `num_entities ∈ 0..=256`, `command_time > 0`, origine
   finie, pas tout-à-zéro
4. Garder les candidats qui passent
5. Chercher la **paire à exactement 53 772 bytes d'écart** — c'est la
   signature de `cg.activeSnapshots[0..2]`, rien d'autre dans le
   process n'a cette propriété
6. Des deux, prendre celui avec le plus grand `serverTime` = `cg.snap`,
   l'autre = `cg.nextSnap`

Le filtre initial laxiste matchait des blocs vides ; après durcissement
(au moins un de `{origin, velocity, viewangles, HP > 0}` non-nul +
`command_time > 0`), seules les vraies snapshots passent. En jeu, le
scan trouve `~50 candidats`, dont la paire unique à 53 772 B d'écart.
Confirmé sur une vraie session :

```
=> Confirmed cg.activeSnapshots pair: 0x067FBE88 / 0x06809094 (Δ = 53772 bytes).
   serverTime: [067FBE88]=968700  [06809094]=968750  → reading the newer one
```

---

## 6. Lire les positions des joueurs — le piège tr_base

Premier dump après la localisation :

```
slot  client  weapon  position
1     1       2       (0.0, 0.0, 0.0)
3     3       3       (0.0, 0.0, 0.0)
68    1       0       (0.0, 0.0, 0.0)
```

Trois joueurs visibles, **toutes les positions à zéro**. Étrange,
parce que les items dans le même scan avaient des positions correctes
(`(1120, 2316, 48)`). Pourquoi les joueurs non ?

Retour au source. `entityState_t` contient deux représentations de
position :

```c
vec3_t origin;        // position finale, pour le rendu
trajectory_t pos;     // trBase + trDelta + trType
```

`pos.trType` est l'enum qui dit comment la position évolue dans le
temps : `TR_STATIONARY` (figé), `TR_LINEAR` (vitesse constante),
`TR_INTERPOLATE` (interpolé entre deux snapshots), etc.

Conventions serveur :

- **Items** (`ET_ITEM`, `TR_STATIONARY`) → le serveur écrit la même
  valeur dans `origin` ET dans `pos.trBase`. Position lisible dans
  l'un ou l'autre.
- **Joueurs** (`ET_PLAYER`, `TR_INTERPOLATE`) → le serveur écrit
  uniquement `pos.trBase` à chaque frame. `origin` est laissé à zéro
  parce que le client est censé l'interpoler entre `pos.trBase` du
  snapshot précédent et celui du suivant.

Donc pour les joueurs, lire `entityState_t.origin` donne `(0, 0, 0)`
mais lire `entityState_t.pos.trBase` donne la vraie position. Un
one-liner corrigé et :

```
slot  client  weapon  position (tr_base)
2     2       3       (195.0, 2171.0, 36.0)
```

ESP fonctionnel.

---

## 7. La limite qu'on ne peut pas franchir : le PVS

À ce stade on lit `cg.snap.entities[0..numEntities]`. Mais cette liste
ne contient **jamais tous les joueurs de la partie**. Elle ne contient
que ceux que le serveur a décidé de nous envoyer cette frame.

### Comment ça marche

Le BSP de la map est partitionné en *areas* visuellement séparées
(couloirs, salles, étages). Pour chaque area, le compilateur de map
précalcule le **PVS** (Potentially Visible Set) — la liste des autres
areas qu'on peut potentiellement voir depuis celle-ci.

À chaque frame côté serveur :

```c
// snippet schématique de sv_snapshot.c
for (chaque entité dans le monde) {
    if (entité.area est dans le PVS de joueur_local.area) {
        ajouter_au_snapshot(entité);
    } else {
        // pas envoyée — le client ne la verra jamais
    }
}
```

Concrètement :
- Un bot dans le même couloir que toi, même caché derrière un mur →
  dans ton PVS → dans `cg.snap.entities[]` → ESP marche
- Un bot deux salles plus loin avec une porte fermée entre → pas dans
  ton PVS → **pas envoyé par le serveur** → invisible pour le client

### Pourquoi on ne peut rien faire côté client

Le PVS est appliqué **avant** que le packet réseau soit transmis. Quand
le packet arrive sur ta machine, il ne contient pas l'info des entités
filtrées — elles n'existent simplement pas dans ton process Quake.
Aucune mémoire à scanner, aucun pointeur à suivre, rien à
décompresser : la donnée n'est pas là.

C'est différent d'autres jeux où le client reçoit tout et n'affiche
qu'une partie. Dans Quake III, le filtrage est **autoritaire et
serveur-side**.

Toutes les structures qu'on a vues — `cg.snap`, `cl.parseEntities[]`,
`cl.snapshots[PACKET_BACKUP]`, `entityBaselines[]` — ne sont que des
représentations différentes des **mêmes** données filtrées par le PVS.
Lire l'une ou l'autre ne change rien : elles ont toutes la même
information manquante.

### Ce qui marche quand même

1. **Le PVS de Q3 est assez généreux.** Sur la plupart des maps DM, tu
   reçois la grande majorité des joueurs même cachés derrière des
   géométries proches. L'ESP basé sur snapshot capture ~80-95 % des
   ennemis en pratique.
2. **Le HP, l'armor, le weapon des joueurs visibles** sont dans
   l'`entityState_t` envoyé. Pas besoin d'aller chercher ailleurs.
3. **La latence est nulle** côté lecture : `cg.snap` est mis à jour à
   chaque frame de rendu, on lit toujours du frais.

### Le seul vrai bypass : être le serveur

Quand tu héberges (`/devmap`, listen server, partie locale), **ton
process exécute aussi le code serveur**. Le serveur, lui, possède
`g_entities[MAX_GENTITIES = 1024]` — le tableau autoritaire de **toutes
les entités du monde**, PVS ou pas. Il est dans `qagame.qvm` ou
`qagamex86_64.dll` selon l'install, dans le même process.

Lire `g_entities[]` te donne tout, sans filtrage. C'est pour ça que les
"cheats LAN / singleplayer" peuvent voir tout le monde et que les
cheats sur serveur distant butent toujours sur le PVS.

Cette implémentation reste à faire ici — `dump-snapshot` couvre
uniquement la voie client.

---

## 8. Synthèse de la démarche

Ce qu'on a appris en pratique en allant du HP à la liste des joueurs :

1. **Un offset isolé ne suffit pas.** Trouver `0x7B40B8` ne dit pas ce
   qu'il y a autour. Il faut lire le source pour savoir *quoi*
   chercher avant de chercher *où*.
2. **Mirrorer les structs avec assertions au build** détecte
   immédiatement une mauvaise interprétation. C'est plus rapide que de
   débugger un dump bizarre.
3. **Une signature topologique vaut mieux qu'un pointeur.** L'écart de
   53 772 bytes entre les deux snapshots est plus stable qu'une RVA
   absolue : il survit aux changements d'ASLR, de heap layout, de
   build.
4. **Le filtre du scan doit refléter la réalité du jeu** (pm_type
   normal → HP > 0, etc.) sinon les blocs vides matchent autant que les
   vrais.
5. **Les champs C ne se lisent pas comme on le pense.** `origin = 0`
   pour les joueurs n'est pas un bug du cheat — c'est une convention du
   réseau Quake. Sans regarder `trajectory_t.trBase`, on conclut à
   tort que la liste est cassée.
6. **Le PVS est une limite par construction du protocole**, pas une
   protection ajoutée. Aucun reverse client-side ne la contourne. Pour
   passer outre, il faut changer de point de vue (lire le serveur).

---

## 9. Pour aller plus loin

- Implémenter la lecture côté serveur (`g_entities[]`) pour les
  parties locales — voir limite §7.
- Boucler `dump-snapshot` à 10 Hz pour avoir un radar live.
- Calculer le `pitch/yaw` vers chaque ennemi (`atan2(dy, dx)` et
  `atan2(dz, dist_xy)`) — première brique d'un aim-helper.
- Cacher l'adresse de `cg.snap` trouvée pour éviter le scan complet à
  chaque run, avec re-validation par `serverTime` croissant.

Ces extensions ne changent pas la limite PVS ; elles s'empilent toutes
sur les données déjà visibles dans le snapshot.
