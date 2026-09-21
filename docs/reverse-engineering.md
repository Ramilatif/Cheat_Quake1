# Reverse engineering de ioquake3 — du process Windows au cheat complet

Ce document retrace la démarche concrète depuis un seul
`ioquake3.x86_64.exe` qui tourne et la seule connaissance que "quelque
part en mémoire il y a une liste de joueurs", jusqu'à un aimbot et un
wallhack pilotés par un menu en jeu (`qcheat menu`) : lire **frame par
frame** la position et le HP de chaque joueur visible (§1-8), piloter
la visée sans jamais écrire en mémoire (§9-11), projeter cette même
donnée en 2D par-dessus le jeu (§12), et réunir le tout dans une seule
boucle robuste (§13). Et où se trouvent les limites que rien ne peut
contourner côté client (§7).

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

## 9. L'aimbot, tentative 1 : écrire l'angle en mémoire (échec)

Une fois `cg.snap` localisé (§5), l'étape suivante paraît triviale :
calculer `atan2(dy, dx)` vers l'ennemi le plus proche, puis écrire ce
yaw/pitch directement dans la copie qu'on lit. `WriteProcessMemory`
marche très bien techniquement — le problème, c'est que **rien ne lit
plus jamais cette valeur**.

### Les fausses pistes, dans l'ordre où elles ont été essayées

1. **`cl.snapshots[PACKET_BACKUP]`** (le ring buffer d'historique côté
   engine, 32 entrées espacées de `sizeof(clSnapshot_t) = 540 B`).
   Écrire dedans ne fait rien : en suivant "find out what writes to
   this address" dans Cheat Engine, l'écriture retombe systématiquement
   dans `msvcrt.memcpy` — c'est juste une copie brute de paquet réseau
   vers l'historique, jamais relue pour le rendu. Fréquence de
   correction : celle du tick serveur, pas celle du rendu.
2. **Une adresse sur la pile.** Même technique de traçage, cette fois
   le writer était une vraie fonction du jeu (prologue `push
   rbp/rdi/rsi/rbx` propre, pas `msvcrt`) — mais la destination était à
   quelques octets de `RSP` au moment du hit. Une variable locale
   temporaire, réutilisée à chaque appel de fonction suivant : écrire
   dedans est sans effet dès la frame suivante.
3. **Un miroir en tas (heap), trouvé par recoupement de deux scans
   "changed value" indépendants.** Cette adresse suit parfaitement
   l'angle réel et se corrige *immédiatement* si on la modifie — donc
   elle est bien lue quelque part et recalculée chaque frame. Mais même
   en la figeant avec **Freeze** dans Cheat Engine (force la valeur en
   continu, contourne le problème "une seule écriture s'efface"), la
   caméra ne bouge pas. Conclusion : c'est encore une copie miroir,
   pas la source consommée par le renderer.

### La vraie explication, trouvée dans le source

`code/client/cl_input.c`, `CL_MouseMove` :

```c
mx *= cl_sensitivity->value;
my *= cl_sensitivity->value;
mx *= cl.cgameSensitivity;
my *= cl.cgameSensitivity;

cl.viewangles[YAW]   -= m_yaw->value   * mx;
cl.viewangles[PITCH] += m_pitch->value * my;
```

Et `code/game/bg_pmove.c`, `PM_UpdateViewAngles` — appelée à **chaque
frame de prédiction client**, pas à chaque tick réseau
(`cg_predict.c: CG_PredictPlayerState`) :

```c
temp = cmd->angles[i] + ps->delta_angles[i];
ps->viewangles[i] = SHORT2ANGLE(temp);
```

Autrement dit : `cg.predictedPlayerState.viewangles` (donc aussi
`cg.snap.ps.viewangles`, sa copie) est **recalculé toutes les frames**
à partir de `cmd->angles`, lui-même dérivé de `cl.viewangles`, lui-même
piloté par le delta de souris brut. Écrire dans n'importe quelle copie
de l'angle est annulé à la frame suivante, que l'écriture réussisse ou
non — le moteur ne lit jamais notre valeur, il **recalcule** la
sienne depuis l'input.

C'est une limite de la même famille que le PVS (§7) : pas une
protection anti-triche explicite, une conséquence directe de
l'architecture (prédiction client-side pour masquer la latence
réseau). Contourner ça en modifiant encore plus de mémoire (patcher
`cl.viewangles`, ou `cmd->angles` avant qu'il soit consommé) revient à
courir après une valeur recalculée en boucle fermée toutes les ~8 ms —
fragile et pas nécessaire.

---

## 10. L'aimbot, tentative 2 : piloter la souris, pas la mémoire

Si le moteur recalcule l'angle depuis l'input souris, la solution est
de fournir cet input nous-mêmes plutôt que d'essayer de forcer sa
sortie. Windows expose `SendInput` (`user32.dll`, via
`windows::Win32::UI::Input::KeyboardAndMouse`) : un mouvement de
souris relatif synthétique, indiscernable pour le jeu d'un vrai
mouvement de souris physique, puisqu'il traverse exactement le même
chemin (`CL_MouseMove` lit le delta accumulé, quelle que soit sa
source).

```rust
let input = INPUT {
    r#type: INPUT_MOUSE,
    Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, dwFlags: MOUSEEVENTF_MOVE, .. } },
};
SendInput(&[input], size_of::<INPUT>() as i32);
```

Plus besoin de trouver la bonne adresse : on ne lit plus jamais que
`cg.snap.ps.origin`/`viewangles` (déjà localisés en §5) pour connaître
la position/l'angle actuels, et on pousse un delta de souris calculé à
partir de l'écart avec l'angle voulu.

### Piège de signe : `cl_input.c` inverse le yaw

Première version : la caméra tourne, mais **s'éloigne** de la cible au
lieu de s'en rapprocher. En relisant `CL_MouseMove` (§9) :

```c
cl.viewangles[YAW] -= m_yaw->value * mx;   // mx positif (souris à droite) DIMINUE le yaw
```

Le calcul du delta de souris à envoyer doit donc **inverser** le signe
de l'erreur de yaw calculée (alors que pour le pitch, `+=` correspond
directement à la convention "souris vers le bas augmente le pitch =
regarde vers le bas", pas d'inversion nécessaire). Un bug de signe
d'apparence anodine, mais qui pousse activement la visée dans la
mauvaise direction à chaque tick — largement plus trompeur qu'une
correction qui ne fait simplement rien.

### Deux bugs de ciblage, indépendants du calcul d'angle

- **Se cibler soi-même.** `cg.snap.entities[]` contient aussi
  l'entité du joueur local — sans l'exclure (comparaison sur
  `client_num == ps.client_num`), elle "gagne" toujours comme cible la
  plus proche, distance 0.
- **Cibler un cadavre.** `entityState_t.eFlags & EF_DEAD` (bit
  `0x00000001`, défini dans `bg_public.h`) indique un joueur mort.
  Sans ce filtre, l'aimbot continue de suivre un corps au sol.

---

## 11. Lisser le mouvement : le décalage entre tick serveur et boucle de poll

Une fois la visée fonctionnelle, la caméra tremble. Cause : le serveur
Quake III n'envoie un nouveau snapshot qu'au rythme de `sv_fps`
(~20-40 Hz), largement en dessous de la boucle de poll externe
(~120 Hz à 8 ms/tick). Recalculer une correction à chaque poll sur la
**même** donnée périmée revient à envoyer plusieurs fois la même
commande avant que son effet précédent ne soit visible — le classique
excès de correction (overshoot) d'une boucle asservie à laquelle on ne
laisse pas le temps de "voir" son propre effet.

La solution retenue est la même que celle que le moteur utilise
lui-même pour l'affichage (interpolation entre deux `clSnapshot_t`,
`LerpAngle` dans `cg_predict.c`) : **prédire** entre deux vraies mises
à jour plutôt que de rejouer l'ancienne donnée.

- **Position de la cible** : extrapolation linéaire par vélocité.
  À chaque vrai changement de `serverTime`, on calcule
  `vel = (nouvelle_position - ancienne_position) / dt_serveur`
  (`entityState_t` n'expose pas de vélocité utilisable directement
  pour un joueur — `trType` vaut `TR_INTERPOLATE`, pas
  `TR_LINEAR` — donc on la déduit nous-mêmes de deux échantillons
  successifs). À chaque tick de la boucle, on projette
  `position = dernière_position + vel × temps_écoulé_réel`.
- **Notre propre angle** : `cg.snap.ps.viewangles` reflète en réalité
  `cg.predictedPlayerState`, recalculé chaque frame de rendu (§9) — le
  relire à chaque tick serait donc déjà suffisant en théorie. Mais la
  version qui fonctionne intègre plutôt localement l'angle en
  additionnant les corrections déjà envoyées (`own_yaw -= deg_par_count
  × dx_envoyé`), et ne **resynchronise** sur la valeur lue qu'à chaque
  vrai nouveau `serverTime`. Ça évite tout effet de bord si la lecture
  a un cycle de retard, et ça reste cohérent avec l'extrapolation
  utilisée côté cible.

Effet : à chaque tick, l'erreur angle-cible évolue un peu (au lieu de
rester identique plusieurs polls de suite), donc chaque correction
envoyée est petite et cohérente avec la précédente — mouvement lissé,
sans jamais dépendre de la fréquence réseau du serveur.

---

## 12. Le wallhack : projeter le snapshot en 2D par-dessus le jeu

`cg.snap.entities[]` contient déjà la position de **tous** les
ennemis dans le PVS (§7), visibles ou non à l'écran — l'occlusion par
les murs est une décision de rendu, pas une propriété de la donnée.
Un ESP "boîtes à travers les murs" est donc juste un problème de
projection 3D → 2D, pas de nouvelle lecture mémoire.

### Projection : refaire `AngleVectors` à la main

Le moteur calcule sa base caméra (avant/droite/haut) depuis yaw/pitch
avec `AngleVectors` (`q_math.c`). En Rust, roll supposé nul :

```rust
let forward = Vec3::new(cp * cy, cp * sy, -sp);
let right   = Vec3::new(sy, -cy, 0.0);
let up      = Vec3::new(sp * cy, sp * sy, cp);
```

Puis projection perspective classique : projeter le vecteur
`cible - œil` sur cette base (`cx, cy, cz`), rejeter si `cz < 1`
(derrière la caméra), et mettre à l'échelle avec le FOV :

```rust
let scale = (largeur_écran * 0.5) / (fov_rad * 0.5).tan();
let sx = largeur/2.0 + cx * scale / cz;
let sy = hauteur/2.0 - cy * scale / cz;
```

Une boîte par joueur se construit en projetant deux points (pieds =
`pos.trBase`, tête = pieds + hauteur approx.) plutôt qu'un point unique
+ taille fixe — la boîte se met alors à l'échelle correctement avec la
distance sans calcul supplémentaire.

### Fenêtre overlay : transparente, cliquable-à-travers, toujours au-dessus

Une fenêtre Win32 classique (`WS_POPUP`), avec :

- `WS_EX_LAYERED` + `SetLayeredWindowAttributes(.., LWA_COLORKEY)` :
  une couleur clé (noir) devient transparente au rendu — on dessine le
  fond en noir puis les formes par-dessus en GDI (`Rectangle`,
  `TextOutW`), sans double buffering (léger scintillement accepté).
- `WS_EX_TRANSPARENT` : les clics souris traversent la fenêtre vers le
  jeu en dessous — indispensable pour ne pas gêner le gameplay.
- `WS_EX_TOPMOST` : reste au-dessus du jeu.

Piège découvert à l'usage : **Windows démet une fenêtre topmost avec
le temps** (le jeu reprend le focus, alt-tab, une notification passe)
si on ne demande `HWND_TOPMOST` qu'une fois à la création — l'overlay
finit par disparaître derrière le jeu après quelques minutes. Fix :
réaffirmer `SetWindowPos(.., HWND_TOPMOST, ..)` à **chaque tick**
(appel `SWP_NOMOVE|SWP_NOSIZE`, quasi gratuit).

La même extrapolation par vélocité que pour l'aimbot (§11) s'applique
ici pour que les boîtes suivent les ennemis sans à-coups entre deux
vrais snapshots, sur une `HashMap<client_num, Track>` (une entrée par
ennemi, purgée dès qu'il n'apparaît plus dans un vrai snapshot — mort,
déconnecté, sorti du PVS).

---

## 13. Tout réunir : un menu en jeu au lieu de relancer la CLI

Aimbot et wallhack tournaient d'abord comme deux commandes séparées,
chacune avec son propre scan mémoire et ses propres réglages figés au
lancement (`--sensitivity`, `--fov`, ...). Les fusionner en une seule
boucle (`qcheat menu`) apporte deux choses : un seul scan mémoire
partagé, et des réglages modifiables **en direct**, en jeu.

### Pourquoi le menu se pilote au clavier, pas à la souris

Le jeu capture et cache le curseur système en continu pour le
mouselook (et l'overlay ne peut rien y changer sans se battre avec le
moteur). `GetCursorPos` renverrait donc une position sans rapport avec
"où l'utilisateur regarde/pointe" pendant une partie active. Plus
simple et plus robuste : navigation entièrement clavier (haut/bas =
sélection, gauche/droite = ajustement), interrogée par polling
(`GetAsyncKeyState`, comme la touche de bascule de l'aimbot) — pas de
dépendance au focus de fenêtre ni au curseur.

### Une seule structure de réglages, un seul thread

```rust
struct Settings {
    aimbot_enabled: bool,
    sensitivity: f32,
    smooth: f32,
    max_delta: f32,
    esp_enabled: bool,   // affiché "Wallhack" dans le menu
    fov: f32,
}
```

Le menu, la correction d'aimbot et le dessin des boîtes tournent dans
la **même** itération de boucle, dans le même thread — pas de
`Arc<Mutex<..>>`, juste une struct mutée directement. Ça élimine toute
question de synchronisation entre "ce que le menu vient de changer" et
"ce que la frame suivante applique".

### Le compromis rescan : ne jamais bloquer sur un scan mémoire complet

Lancer l'outil **avant** de rejoindre une partie pose un problème
propre : `cg.activeSnapshots` n'existe pas encore, et un scan large
peut ponctuellement matcher un bloc de mémoire qui *ressemble* à un
snapshot (passe les filtres de plausibilité de `looks_like_snapshot`)
sans en être un vrai — les deux se traduisent par un cache qui reste
"valide" indéfiniment sans jamais correspondre à une vraie partie.

Première tentative : forcer un rescan complet toutes les ~5 secondes,
peu importe l'état du cache. Ça corrige bien le cas "lancé trop tôt",
mais un scan complet de la fenêtre mémoire prend ~1-2 secondes de
`ReadProcessMemory` — réintroduit exactement le symptôme qu'on
cherchait à éviter, sous forme de coupures périodiques de 2 secondes
pendant une partie par ailleurs saine.

Fix retenu : ne déclencher le rescan coûteux que si `serverTime` **n'a
pas avancé depuis plusieurs secondes** — signature exacte d'un
snapshot figé (pas encore en partie, ou faux positif), jamais observée
pendant une partie active où le serveur tique en continu. Ce check ne
coûte rien (on lit déjà `serverTime` à chaque frame) et ne déclenche le
scan lourd que quand c'est réellement nécessaire :

```rust
if snap.header.server_time != last_seen_server_time {
    last_seen_server_time = snap.header.server_time;
    last_change_wall = Instant::now();
} else if last_change_wall.elapsed() > Duration::from_secs(3) {
    // figé depuis trop longtemps pour être une partie active — rescanner
}
```

Le même principe (rafraîchir seulement quand une valeur cesse
d'évoluer, jamais sur une minuterie aveugle) s'applique à la taille et
la position de la fenêtre du jeu, pour rattraper le cas où l'outil est
lancé pendant un écran de chargement à une résolution différente de
celle de la partie.

---

## 14. Synthèse mise à jour

Ce qui s'ajoute aux leçons du §8 en poussant jusqu'à l'aimbot et
l'overlay :

7. **Une valeur qui se corrige toute seule n'est pas forcément la
   bonne source.** Elle peut être recalculée depuis une source encore
   plus en amont (ici : l'input souris) à laquelle il faut s'adresser
   directement plutôt que de continuer à chasser des copies.
8. **Piloter l'input plutôt que forcer l'état** est plus robuste
   qu'écraser une valeur en mémoire quand cette valeur est
   recalculée en boucle fermée par le moteur — ça élimine toute la
   classe de bugs "où est la vraie adresse".
9. **Un correcteur qui tourne plus vite que sa source de vérité doit
   prédire, pas répéter.** Rejouer la même erreur plusieurs fois avant
   d'en voir l'effet cause de l'oscillation, quelle que soit la
   qualité du calcul de correction lui-même.
10. **Un rescan de sécurité doit être déclenché par un symptôme réel
    (donnée figée), jamais par une minuterie aveugle** — sinon le
    remède coûte aussi cher que le problème qu'il corrige.

---

## 15. Pour aller plus loin

- Implémenter la lecture côté serveur (`g_entities[]`) pour les
  parties locales — voir limite §7. Reste la seule vraie façon de
  dépasser le PVS.
- Chams / vrai wallhack par patch du rendu (désactiver le depth-test
  ou le culling pour les joueurs) — nécessiterait de sortir de
  l'architecture 100% lecture externe (`ReadProcessMemory`/
  `SendInput`) suivie jusqu'ici, vers de l'injection de code.
- Sauvegarder/charger les `Settings` du menu dans un fichier
  (`qcheat.toml`) pour ne pas repartir des valeurs par défaut à chaque
  lancement.
- Double-buffering pour le dessin de l'overlay (actuellement du GDI
  direct sans back-buffer, léger scintillement visible).

Ces extensions ne changent pas la limite PVS (§7) ni le fait que le
moteur recalcule l'angle depuis l'input (§9) ; elles s'empilent sur
l'architecture lecture-externe + injection d'input déjà en place.
