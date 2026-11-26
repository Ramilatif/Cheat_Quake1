# Comprendre le projet — explications et questions types

Ce document explique le fonctionnement de `qcheat` concept par concept. Chaque section renvoie au fichier du dépôt où l'observer. Le parcours d'étude pas à pas se trouve dans `docs/architecture.md`.

---

## 1. Principe général

Un processus Windows possède son propre espace d'adresses. Un autre processus peut, avec les droits nécessaires, ouvrir un handle (`OpenProcess`) puis lire (`ReadProcessMemory`) ou écrire (`WriteProcessMemory`) dans cet espace. Aucun code n'est injecté : `qcheat` reste un programme séparé qui observe de l'extérieur. Voir `crates/process/src/handle.rs`.

La difficulté n'est pas la lecture, mais de savoir **quoi lire et où** : la mémoire est une suite d'octets sans étiquettes.

## 2. De la structure C à la structure Rust

Le jeu définit `entityState_t`, `playerState_t` et `snapshot_t` en C. Leur interprétation demande de reproduire la même disposition en Rust avec `#[repr(C)]` : même ordre des champs, mêmes tailles, même alignement. Sans `repr(C)`, Rust peut réordonner les champs.

Les `const _: () = assert!(size_of::<T>() == N)` du SDK font échouer la **compilation** si la taille du miroir dérive. Elles établissent que le miroir est cohérent avec les valeurs relevées dans les en-têtes. Elles n'établissent pas que le binaire en cours d'exécution utilise ce layout : le filtre de plausibilité à l'exécution assure ce second contrôle. `bytemuck::Pod` permet de convertir des octets lus en structure sans `unsafe` côté appelant, à condition que le type ne contienne ni padding caché ni pointeur.

## 3. Pourquoi un scan par signature

`cgame` s'exécute dans une machine virtuelle (QVM) dont la mémoire est un tableau alloué dynamiquement : l'adresse de `cg` change à chaque lancement. Deux types d'adresses coexistent.

| Type | Exemple | Stabilité |
|---|---|---|
| RVA moteur (variables globales de l'exe, section `.bss`) | `0x7B40B8` (HP) | Stable tant que l'exe n'est pas recompilé ; adresse réelle = base du module + RVA |
| Adresse du heap QVM | snapshot | Change à chaque lancement : à retrouver à chaque fois |

Le scan par signature (`cli/src/locate.rs`, `sdk/src/plausibility.rs`) parcourt la fenêtre mémoire par blocs, lit chaque offset aligné sur 4 octets comme un `SnapshotHeader` candidat et applique des filtres de plausibilité : `pm_type` valide, `clientNum < MAX_CLIENTS`, `weapon` valide, origine finie, HP plausible. Un candidat isolé peut être un faux positif ; **deux candidats distants d'exactement 53 772 octets** correspondent à `cg.activeSnapshots[0]` et `[1]`, ce qui constitue une signature structurelle forte. Le candidat dont `serverTime` est le plus grand est retenu.

La détection de stride (`scanner/src/stride.rs`) généralise l'idée : des résultats séparés par un écart constant révèlent un tableau.

## 4. Le snapshot et `tr_base`

Le serveur envoie au client un `snapshot_t` à 20–40 Hz (`sv_fps`) : l'état du joueur local et les entités visibles. Pour une entité en mouvement (`trajectory_t`), le moteur stocke un point de départ `trBase`, un type de trajectoire, un temps et une vitesse ; le champ `origin` est souvent nul car recalculé à l'affichage. D'où l'usage de `pos.trBase` comme position.

## 5. Le PVS : limite du point de vue client

Le serveur calcule pour chaque client un **PVS** (Potentially Visible Set) à partir de la carte et n'inclut dans le snapshot que les entités de cet ensemble (`sv_snapshot.c`, `SV_AddEntitiesVisibleFromPoint`). Les autres ne sont jamais envoyées. L'overlay `esp` seul ne voit donc que ce que le client a reçu : des entités derrière un mur proche, pas l'ensemble de la carte. Avec `--server`, il lit en plus `g_entities[]` (voir §8).

L'état complet est dans `g_entities[]`, côté serveur. Il est accessible lorsque le serveur tourne dans le même processus (partie hébergée localement). Principe de conception réseau à retenir : **le serveur ne doit envoyer au client que ce dont il a besoin**.

## 6. Les trois approches de la visée

- **`aimbot` (écriture mémoire).** Écriture des angles dans l'adresse relevée et dans 32 copies d'un tableau (`cl.snapshots[]`, stride 0x21C = 540 o = `sizeof(clSnapshot_t)`). Ces copies sont de l'**historique** et non la variable qui alimente le rendu. Enseignement : une adresse « qui change quand la souris bouge » ne suffit pas ; il faut distinguer la source de vérité des copies.
- **`find-viewangles`.** Automatise le principe « valeur modifiée » de Cheat Engine avec une référence connue (le yaw du snapshot), plusieurs rondes de confirmation et l'intersection des candidats pour éliminer le bruit.
- **`aim-mouse`.** Emprunte le chemin d'entrée normal du jeu : `SendInput` envoie des déplacements relatifs de souris traités comme des entrées natives. La conversion d'un angle en « counts » suit `degrés par count = sensitivity × m_yaw`. La boucle de contrôle applique une correction proportionnelle (`--smooth` = fraction de l'erreur corrigée par tick), un plafond (`--max-delta`) et une prédiction entre deux snapshots (extrapolation linéaire de la cible, intégration locale de l'angle propre, resynchronisation à chaque nouveau `serverTime`). `SendInput` agit sur la fenêtre au premier plan.

## 7. L'overlay (`esp`)

Fenêtre « layered », toujours au premier plan, insensible aux clics (`WS_EX_LAYERED | WS_EX_TRANSPARENT`), avec une couleur clé transparente (noir). À chaque tick : lecture du snapshot, projection monde → écran à partir des angles du joueur et du FOV (`cg_fov`), dessin GDI de boîtes. Entre deux snapshots, la position est extrapolée avec la vitesse estimée pour supprimer l'effet d'escalier. La caméra de projection est `ps.origin + viewheight` (l'origine est le centre du joueur, pas son œil) et la boîte d'un joueur va de `origin − 24` à `origin + 32`. Il s'agit d'une fenêtre indépendante, sans injection. `--debug` affiche chaque seconde l'état de la lecture (snapshot utilisé, joueurs vus, boîtes dessinées).

## 8. Le volet serveur

`server-entities` retrouve `g_entities[]` en utilisant le snapshot client comme référence, déduit `sizeof(gentity_t)` de deux entités de numéros différents, puis recoupe avec le pointeur `sv.gentities` du moteur. `esp --server` réutilise cette localisation : il dessine en vert les joueurs présents dans `g_entities[]` mais absents du snapshot (filtrés par le PVS). Le bon snapshot y est reconnu en comparant la position du joueur local avec son entité côté serveur. Détails, mesures en jeu et limites : `ServerSide/server-reverse.md`.

---

## 9. Questions types

**Pourquoi ioquake3 ?** Code source ouvert, structures vérifiables dans les en-têtes, architecture QVM instructive, aucune protection tierce.

**Qu'est-ce qui garantit que le miroir d'une structure est correct ?** Les assertions de taille et d'offsets à la compilation (cohérence avec la référence), puis la plausibilité à l'exécution (filtres, paire de snapshots espacés de 53 772 octets, `serverTime` croissant). Les assertions seules ne valident pas le binaire cible.

**Pourquoi scanner la mémoire plutôt que coder des adresses ?** Le heap QVM est dynamique : une forme est recherchée, pas une adresse.

**Pourquoi 53 772 octets ?** C'est `sizeof(snapshot_t)` : `SnapshotHeader` (516 o) + 256 `entityState_t` de 208 o + deux entiers de fin. Deux copies consécutives dans `cg.activeSnapshots[2]` donnent un écart exact.

**Pourquoi l'overlay ne voit-il pas toutes les entités sans `--server` ?** Le PVS filtre avant l'envoi : lire davantage côté client ne change rien.

**Comment accéder à toutes les entités ?** En lisant l'état serveur dans le même processus (partie hébergée localement), via `g_entities[]` : c'est ce que fait `esp --server`.

**Pourquoi `aim-mouse` plutôt que l'écriture mémoire ?** Les écritures atteignaient des copies, pas la variable du rendu ; l'entrée souris suit le chemin natif du jeu et aboutit à la bonne variable sans en connaître l'adresse.

**Quelles protections la cible oppose-t-elle ?** Aucune côté client. La seule barrière est architecturale : le serveur n'envoie que les entités du PVS.

**Quelles sont les limites actuelles ?** RVA liés à un build, `aimbot` mémoire expérimental, `aim-mouse` pas encore branché sur la localisation fiable du snapshot, tests limités à `sdk` et `scanner::stride`, volet serveur limité aux parties hébergées localement.

**Quelles améliorations restent à faire ?** Remonter la localisation du snapshot vivant dans `locate.rs`, RVA dans la configuration, fixture de snapshot capturé pour tester hors ligne, fichier `qcheat.toml`, `qcheat doctor`.

**Que montre le projet sur la conception réseau d'un jeu ?** Tout état envoyé au client est lisible par ce client ; la protection vient de ce que le serveur n'envoie pas.

---

## 10. Glossaire

| Terme | Définition |
|---|---|
| RVA | Adresse relative à la base du module ; adresse réelle = base + RVA. |
| QVM | Machine virtuelle de Quake III exécutant le code de jeu (`cgame`, `qagame`). |
| Snapshot | État du monde envoyé par le serveur à un client à chaque tick. |
| PVS | Ensemble de zones potentiellement visibles depuis un point ; filtre côté serveur. |
| `trBase` | Point de départ d'une trajectoire ; position fiable des entités en mouvement. |
| Stride | Écart constant entre éléments consécutifs d'un tableau. |
| `Pod` | Type dont la mémoire peut être lue ou écrite comme de simples octets (`bytemuck`). |
| `SendInput` | API Windows qui injecte des événements clavier ou souris dans la file d'entrée du système. |
| Fenêtre layered | Fenêtre Windows pouvant être transparente et laisser passer les clics. |
