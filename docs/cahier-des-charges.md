# Cahier des charges — Cheat_Quake1

**Étude de la lecture et de la modification de l'état d'un processus tiers en Rust externe : reverse-engineering, miroir de structures C, scan mémoire par signature, interaction avec le jeu et étude du protocole client-serveur, sur un jeu open-source (ioquake3). Preuve de concept fonctionnelle en conditions réelles, utilisée en local.**

*Version révisée pour refléter l'état réel du dépôt.*

---

## 1. Contexte et motivation

Projet d'apprentissage individuel à finalité pédagogique : lire et modifier de l'extérieur la mémoire d'un processus sous Windows x86_64, et comprendre comment un jeu temps réel client-serveur distribue l'information (filtrage de visibilité PVS).

Deux problèmes techniques :
1. Retrouver en mémoire des structures précises **sans accès au binaire compilé mais avec le code source amont**.
2. Comprendre comment le serveur décide quelles entités envoyer à chaque client, et où ce filtrage peut être étudié.

**Cible :** ioquake3 (portage open-source GPL-2.0 de Quake III Arena), en partie hébergée localement.

## 2. Objectifs pédagogiques

- **Reverse-engineering structurel** : transposer des structures C en miroirs Rust `#[repr(C)]` avec assertions de taille et d'offsets à la compilation.
- **Programmation système Windows** : `OpenProcess`, `ReadProcessMemory`, `WriteProcessMemory`, snapshots Toolhelp32, `SendInput`, fenêtre superposée GDI.
- **Scan mémoire par signature** : scanner générique, détection de strides, filtrage de plausibilité, ancrage sur une référence connue.
- **Étude du protocole** : comprendre la décision d'inclusion d'une entité dans un snapshot (`sv_snapshot.c`) et les limites qui en découlent pour un client.
- **Architecture Rust** : workspace multi-crates, dépendances unidirectionnelles, gestion d'erreurs (`thiserror` / `anyhow`), documentation publique obligatoire.
- **Outillage** : CLI unifiée avec sous-commandes auto-documentées.
- **Documentation** : référence d'offsets, walk-through reverse, plans d'architecture.
- **Conception réseau** : comprendre ce qu'un client peut ou non lire, et pourquoi le serveur reste l'autorité (« ne jamais faire confiance au client »).

## 3. Périmètre fonctionnel

### 3.1 Fonctionnalités livrées (présentes dans le code)

| # | Fonctionnalité | Commande | Statut |
|---|---|---|---|
| 1 | Localiser le processus (PID, base du module) | `find` | Livré |
| 2 | Énumérer les modules chargés | `modules` | Livré |
| 3 | Suivre un entier 32 bits à une adresse (HP) | `hp` | Livré |
| 4 | Interpréter une adresse comme `entityState_t` | `inspect` | Livré |
| 5 | Scanner une fenêtre pour des `entityState_t` et détecter les strides | `scan` | Livré |
| 6 | Scan heap filtré sur `ET_PLAYER` | `players` | Livré |
| 7 | Localiser `cg.activeSnapshots[2]` (53 772 o) par signature et afficher la frame | `snapshot` | Livré, validé en jeu |
| 8 | Retrouver l'adresse du yaw de rendu par rondes de confirmation | `find-viewangles` | Livré |
| 9 | Retrouver `g_entities[]` côté serveur (listen server), déduire `sizeof(gentity_t)`, comparer avec le snapshot client (démonstration du PVS) | `server-entities` | Écrit, testé sur mémoire simulée ; **à valider en jeu** |

### 3.2 Fonctionnalités présentes mais expérimentales

Ces modules ont été développés au-delà du plan initial (qui ne prévoyait ni overlay ni injection d'entrée). Ils sont validés sur une partie hébergée localement (voir §8).

| Fonctionnalité | Commande | Description | État |
|---|---|---|---|
| Aimbot par écriture mémoire | `aimbot` | Écrit les angles vers le joueur le plus proche dans les adresses de vue et 32 copies de snapshot. | Expérimental ; les écritures atteignent des copies qui n'alimentent pas toujours le rendu. Pas de touche d'activation. |
| Aimbot par entrée souris | `aim-mouse` | Calcule l'erreur angulaire, envoie des déplacements relatifs via `SendInput`, avec lissage, plafond par tick, bascule par touche, prédiction de cible. | Fonctionnel, nécessite de régler `sensitivity`, `m_yaw`, `m_pitch`. |
| ESP / overlay | `esp` | Fenêtre transparente superposée dessinant des boîtes sur les joueurs du snapshot, avec extrapolation entre snapshots. | Fonctionnel ; limité par le PVS (le client ne reçoit pas les entités hors PVS). |

**Écarts assumés par rapport à la version précédente du cahier :**
- L'overlay graphique n'est plus exclu du périmètre (il existe).
- Le projet n'est plus en lecture seule : il écrit en mémoire (`aimbot`) et injecte des entrées (`aim-mouse`).
- La « localisation de `clientActive_t cl` par recoupement A == B » n'est **pas retrouvée** dans le code du dépôt. Ce qui existe est `find-viewangles` et des RVA moteur relevés avec Cheat Engine. Ce point doit être soit livré, soit retiré des revendications.

### 3.3 Fonctionnalités planifiées

- **Exploitation des données serveur** : liste complète des joueurs, vie, équipe, intégrées à l'ESP et au choix de cible.
- **Aimbot v2** : sélection de cible configurable (angle minimal, HP le plus faible), filtre d'équipe et de vivant, filtre de portée. *Les aspects de « discrétion » de l'ancienne version sont abandonnés : le projet étudie le contrôle et le protocole, pas l'évitement de détection.*
- **UX** : `qcheat.toml`, `qcheat doctor`, mode TUI, logging structuré (`tracing`).
- **Tests sur fixtures** : snapshot binaire capturé, rejoué hors-ligne pour tester le SDK et les parseurs en CI.
- **Volet serveur** : localisation de `g_entities[]` et démonstration du PVS : fait dans `server-entities` (à valider en jeu). Reste : lecture détaillée de `sv_snapshot.c`, exploitation des données serveur (ex. ESP complet en local), validation des hypothèses de `docs/server-reverse.md` §6.

## 4. Choix de la cible : ioquake3

1. **Source ouvert (GPL-2.0)** : les structures C sont lisibles et les offsets vérifiables contre une référence publique.
2. **Licence permissive pour l'étude** : aucun code de la cible n'est redistribué ; seuls noms de champs, tailles et offsets sont repris.
3. **Architecture intéressante** : le code de jeu s'exécute dans une machine virtuelle (QVM) ; les structures vivent dans un heap dynamique, ce qui rend la localisation par signature instructive. Le code serveur (`qagame`) tourne dans le même processus en partie hébergée.
4. **Pas de protection tierce** : aucun anti-cheat n'interfère ; contourner une telle protection n'aurait pas de valeur pédagogique et est hors périmètre.

Les tests s'effectuent exclusivement en local (`/devmap` + `/addbot`), sur la machine de développement.

## 5. Architecture technique

Workspace Cargo, quatre crates. Le graphe réel des dépendances est :

```
            cli (qcheat)
          /    |     \
      sdk   scanner   process
                \       ^
                 +------+
```

| Crate | Rôle | Dépend de |
|---|---|---|
| `sdk` | Miroirs `repr(C)` avec asserts de taille/offset ; sans dépendance Windows ni I/O. | `bytemuck` |
| `process` | `ProcessHandle` (lecture et écriture), Toolhelp32. Ne connaît pas le jeu. | `windows`, `thiserror` |
| `scanner` | Primitives de scan génériques (`scan_aligned`, `detect_repeating_stride`). Ne connaît pas le jeu. | `process` |
| `cli` | Binaire `qcheat`, une sous-commande par opération. Seule crate qui connaît le jeu **et** l'API Windows. | toutes |

**Garanties dans le code :** asserts de taille/offset sur toutes les structures miroir ; `#![warn(missing_docs)]` sur les trois bibliothèques ; erreurs via `thiserror` (libs) et `anyhow` (CLI).

**Précision importante :** les `const assert!` vérifient la cohérence du miroir Rust avec les valeurs de référence écrites en dur ; ils ne comparent rien au binaire cible. La validation à l'exécution repose sur le filtre de plausibilité et le recoupement des deux snapshots espacés de 53 772 octets.

**Extension prévue :** crates `config` et `tui`.

## 6. Livrables

| Livrable | Statut |
|---|---|
| Code source, workspace, `cargo build --release` → `qcheat.exe` | Présent |
| `offsets.json` (tailles, offsets, RVA) | Présent, à nettoyer (anciens noms de binaires) |
| `README.md` | Révisé pour refléter l'état réel |
| `docs/reverse-engineering.md` | Présent |
| `docs/cahier-des-charges.md` | Présent (ce document) |
| `docs/comprendre-le-projet.md` | Présent (explications, glossaire) |
| `docs/server-reverse.md` | Présent (méthode, layout vérifié, hypothèses à valider) |
| `qcheat.example.toml` | À produire (avec la couche config) |
| `docs/architecture.md` | Optionnel |

## 7. Méthodologie et état d'avancement

| Phase | Contenu | État |
|---|---|---|
| 1 | Bootstrap Windows (découverte de processus, premier HP avec Cheat Engine) | Terminé |
| 2 | SDK structurel : miroirs + asserts de layout | Terminé |
| 3 | Localisation de `cg.activeSnapshots` par signature | Terminé, validé en jeu |
| 4 | Refactor en 4 crates | Terminé |
| 5 | CLI unifiée `qcheat` | Terminé |
| 6 | Localisation de la variable d'angle de rendu (`find-viewangles`, RVA relevés) | Partiel — localisation de `cl` non retrouvée dans le code |
| 7 | Aimbot v1 (`aimbot` mémoire, `aim-mouse` entrée) | Fonctionnel côté `aim-mouse`, expérimental côté `aimbot` |
| 7bis | ESP overlay | Livré, limité par le PVS |
| 8 | Factorisation du locator (fait) ; RVA en configuration (à faire) | En cours |
| 9 | Aimbot v2 (sélection configurable, filtres) | Planifié |
| 10 | UX (`qcheat.toml`, `doctor`, TUI, `tracing`) | Planifié |
| 11 | Tests sur fixtures | Planifié |
| 12 | Volet serveur (`g_entities[]`, comparaison PVS) | Code écrit, à valider en jeu |
| 13 | Documentation finale | En cours |

Principe : chaque phase laisse le projet compilable et fonctionnel.

## 8. Environnement de test et contraintes techniques

**Environnement.** Les essais sont réalisés sur une partie hébergée localement (`/devmap` puis `/addbot`), sur la machine de développement. Cette configuration est requise par le volet serveur : le code client et le code serveur doivent s'exécuter dans le même processus pour que `g_entities[]` soit lisible.

**Cible.** ioquake3, sous licence GPL-2.0. Aucun code de la cible n'est inclus ; seuls les noms de champs, les tailles et les offsets des structures (en-têtes publics) sont repris.

**Distribution.** Le dépôt contient le code source uniquement ; aucun binaire précompilé n'est publié.

**Protections de la cible.** ioquake3 n'intègre pas de couche de protection côté client. La seule barrière rencontrée est architecturale : le serveur n'envoie que les entités du PVS, ce qui borne ce qu'un client peut lire.

**Contrôles internes.** `server-entities` vérifie la cohérence du tableau (numéro d'entité égal à l'indice, bloc partagé cohérent, drapeaux `SVF_BOT`) ; les autres commandes reposent sur les filtres de plausibilité du SDK.

## 9. Limites assumées

- Windows x86_64 uniquement ; ioquake3 avec cgame en QVM.
- SDK nommé pour Quake III (pas multi-jeu).
- **Limite PVS côté client** : le client ne reçoit jamais les entités hors PVS ; l'ESP actuel ne peut pas les afficher. C'est étudié via le volet serveur.
- RVA liés à un build précis du moteur.
- Pas de manipulation de paquets réseau ; le projet reste intra-machine.

## 10. Dette technique connue

1. ~~Logique de recherche de snapshot dupliquée~~ : factorisée (`cli/src/locate.rs`, filtres dans `sdk::plausibility`).
2. RVA en dur dans `aimbot.rs`, non référencés dans `offsets.json` ; nommage incohérent (`cg.refdefViewAngles` / `cl.snap.ps.viewangles`).
3. Tests : SDK (16) et `scanner::stride` (4) ; pas de test des commandes ni de fixture de snapshot réel.
4. `offsets.json` : références à d'anciens binaires (`memory --bin …`).
5. `README` et tables de commandes à maintenir en phase avec `main.rs`.
6. Licence : projet MIT reprenant des layouts de headers GPL-2.0 ; clarifier (note explicite ou passage en GPL-2.0).

## 11. Suite envisagée

**Court terme** : validation en jeu de `server-entities` ; RVA dans la configuration ; correction de la documentation.
**Moyen terme** : tests sur fixtures ; `qcheat.toml`, `doctor`, `tracing` ; aimbot v2 (sélection, filtres d'équipe/vivant) ; TUI.
**Long terme** : étude de `sv_snapshot.c` ; localisation de `g_entities[]` en listen server ; démonstration locale ; rédaction de `docs/server-reverse.md`.
