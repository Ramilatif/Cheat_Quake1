# Reverse côté serveur — `g_entities[]` et le PVS

Document de travail du volet serveur. Commandes associées : `qcheat server-entities` et `qcheat esp --server`. Statut : **validé en jeu** sur une partie hébergée localement (voir §6 et §7).

## 1. La question

Le client ne reçoit que les entités que le serveur a décidé d'envoyer (PVS, `sv_snapshot.c`). Pour voir toutes les entités du monde, il faut l'état autoritatif du serveur. En **listen server** (`/devmap` + `/addbot`) client et serveur sont dans le même processus : le tableau `g_entities[MAX_GENTITIES = 1024]` du module `qagame` est donc lisible avec les outils déjà construits.

Cela n'est possible que dans une partie hébergée localement. Un client connecté à un serveur distant n'a jamais ce tableau en mémoire : c'est la frontière que le projet met en évidence.

## 2. Ce que le moteur sait d'une entité serveur

Le moteur ne connaît pas `gentity_t` en entier. Il exige seulement que le début soit, dans cet ordre, `entityState_t s` puis `entityShared_t r`, et il connaît la **taille totale** que le jeu lui communique via `trap_LocateGameData(gEnts, n, sizeof(gentity_t), ...)`. Conséquence : le tableau s'adresse `base + i × stride`, avec un stride connu seulement à l'exécution.

Point piégeux (vérifié dans les sources) : `entityShared_t` **commence par un `entityState_t` inutilisé** (conservé pour la compatibilité QVM). Tous les champs de `r` sont donc décalés de 208 octets :

|Champ|Offset dans `r`|Offset dans `gentity_t`|
|---|---|---|
|`linked`|208|416|
|`svFlags`|216|424|
|`currentOrigin`|280|488|
|`ownerNum`|304|512|

`sizeof(entityShared_t) = 308`, donc la « tête » `s + r` fait 516 octets. Jusque-là il n'y a que des champs de 4 octets, donc la disposition est identique en QVM (pointeurs 32 bits) et en module natif 64 bits ; les pointeurs commencent à l'offset 516 (`client`).

## 3. Méthode de localisation (sans adresse en dur)

1. **Vérité terrain côté client.** Lecture du snapshot (`cg.snap`, déjà localisé) : les joueurs reçus donnent `number`, `clientNum` et `trBase`.
2. **Recherche du jumeau serveur.** Scan de la mémoire pour des blocs de 516 octets dont `s` correspond (même `number`/`clientNum`, position proche, tolérance car le client a une ou deux frames de retard) **et** dont le bloc `r` est cohérent : `linked == 1`, boîte englobante saine, `r.currentOrigin ≈ s.pos.trBase`. Ce dernier test élimine les copies côté client (`cg_entities[]`, tableau du snapshot), car les octets à cet offset n'y ont pas ce sens.
3. **Déduction du stride.** L'entité `n` est à `base + n × stride`. Deux jumeaux de numéros différents donnent `stride = Δadresse / Δnuméro` ; chaque paire vote, et seule une taille plausible est retenue (multiple de 4, entre 516 et 4096).
4. **Recoupement avec le moteur.** `sv.gentities` est un pointeur vers ce tableau, stocké dans le `.bss` de l'exe, avec `gentitySize` juste après. Recherche, dans l'image de l'exe, d'un `u64` égal à `base` : si l'entier à `+8` vaut le stride, c'est `sv.gentities` et son RVA est stable (à réutiliser avec `--sv-rva`, sans scan).

Pourquoi ça marche : c'est la même idée que pour le snapshot (signature structurelle + référence dynamique), avec le client comme étalon pour trouver le serveur.

## 4. Ce que la commande affiche

Pour chaque joueur du serveur : numéro de slot, type (hôte/bot), vivant/mort, position serveur, et surtout **s'il figure dans le snapshot du client**. Les joueurs absents du snapshot sont ceux que le serveur a filtrés (PVS) : c'est la démonstration directe de la limite du wallhack côté client.

## 5. Validation de l'offset `svFlags`

`server-entities` s'appuie sur `r.svFlags` (offset 424) et sur `SVF_BOT` (`0x8`) pour distinguer les bots du joueur hôte. Si l'offset était faux, aucun joueur ne porterait le drapeau et la commande s'arrêterait sur une incohérence : l'affichage d'une liste cohérente (un hôte, des bots) valide donc l'offset indirectement. Un joueur spectateur n'est pas lié au monde et n'apparaît pas dans la liste. Logique pure et testée : `sdk::bots_only_verdict`.

## 6. Validation en jeu

Testé sur une partie hébergée localement (`/devmap`, plusieurs `/addbot`) :

- Offset `r.svFlags = 424` et valeur `SVF_BOT = 0x8` : confirmés. La commande distingue correctement les bots de l'hôte (« Local-game check » correct avec 4 bots et 1 humain).
- Fenêtre par défaut (`0x07000000 ± 0x04000000`) : suffisante, le tableau est trouvé dès le premier scan.
- Disposition de la fin de `server_t` : confirmée. La ligne `MATCH sv.gentities` apparaît systématiquement, avec `gentitySize` égal au stride déduit.
- `sizeof(gentity_t)` mesuré : **804 octets (0x324)**. Valeur stockée dans `offsets.json` (`gentity_stride`).
- RVA de `sv.gentities` mesuré sur le build testé : **exe+0x1C0DB30**. Stocké dans `offsets.json` (`sv_gentities_rva`), réutilisable avec `--sv-rva 0x1C0DB30` pour sauter le scan. Cette valeur est propre à ce build et doit être re-mesurée après une recompilation d'ioquake3.
- Cohérence sur l'ensemble du tableau : `number == index` vérifié sur la totalité des créneaux en usage (pas seulement sur les ancres de départ), à chaque lancement testé.
- Comparaison avec le snapshot client : confirmée. Des bots et même l'entité de l'hôte lui-même sont apparus filtrés par le PVS selon la position, ce qui illustre que le filtrage s'applique à toutes les entités sans exception, y compris celle du joueur qui héberge la partie.

Point encore ouvert :

- Alignement : le scanner générique (`scan_aligned`) lit via `from_bytes` sur un `Vec<u8>` ; correct en pratique mais non garanti par le type. La lecture de `g_entities[]` dans l'ESP utilise `pod_read_unaligned`.

## 7. Affichage dans l'ESP (`esp --server`)

L'ESP client ne dessine que les joueurs présents dans le snapshot (boîtes rouges, `#n`). Avec `--server`, il lit en plus les créneaux joueurs de `g_entities[]` (indices 0 à 63) à chaque nouveau snapshot et dessine en vert (`SV#n`) tout joueur que le serveur connaît mais que le client n'a pas reçu :

1. Le tableau est localisé une fois : soit par `--sv-rva` (lecture du pointeur `sv.gentities` à `exe + RVA` et du stride à `+8`), soit par le même scan que `server-entities`.
2. À chaque snapshot, un joueur serveur est retenu s'il est de type joueur, lié au monde (`r.linked`), vivant, différent du joueur local et **absent** du snapshot (`clientNum` non vu).
3. La position serveur est `r.currentOrigin`. Comme pour les joueurs du snapshot, la vitesse est déduite de deux lectures successives et la position est extrapolée entre deux ticks, ce qui garde un mouvement fluide.
4. Les deux ensembles sont indépendants : un joueur passe du vert au rouge quand il entre dans le PVS, et les boîtes rouges se comportent exactement comme sans `--server`.

Lancement type, après avoir mesuré le RVA avec `server-entities` :

```powershell
.\target\release\qcheat.exe esp --fov 90 --server --sv-rva 0x1C0DB30
```

L'option `--debug` affiche chaque seconde le snapshot utilisé, le nombre de joueurs vus par le client et par le serveur, le nombre de joueurs cachés et le nombre de boîtes dessinées.

### Localisation fiable du snapshot

Le premier essai affichait des boîtes aux mauvais endroits, voire aucune. Trois causes ont été identifiées avec `--debug` :

- **Faux positifs du filtre.** Des blocs de mémoire quelconques passent un filtre par plages. Des valeurs de `serverTime` comme `1082130432` sont en réalité le flottant `4.0` lu comme entier. L'ESP rejette désormais tout `serverTime` supérieur à 500 000 000 ms.
- **Copies figées.** Plusieurs structures valides en apparence ne changent jamais (restes de chargement de carte). L'ESP attend 200 ms et ne garde que les candidats dont `serverTime` avance ; si le temps se fige 1,5 s, il relance la recherche.
- **Données serveur prises pour un snapshot.** Un candidat vivant se trouvait à 8 octets du début de `g_entities[]` : le champ `trTime` de l'entité de l'hôte ressemble à un `serverTime` qui avance. Les candidats situés dans la plage de `g_entities[]` sont exclus.

Le test décisif utilise le serveur comme étalon : le bon snapshot est celui dont `ps.origin` est à moins de 150 unités de `r.currentOrigin` de l'entité serveur portant son `ps.clientNum`. Si aucun candidat vivant n'existe dans la fenêtre par défaut, un scan élargi (`0x1000000` à `0x40000000`) est lancé une fois.

### Alignement des boîtes

Deux décalages de hauteur s'additionnaient :

- `ps.origin` est le centre du joueur, pas la caméra. L'œil est à `viewheight` unités au-dessus (26 debout, 12 accroupi).
- `trBase` / `currentOrigin` est le centre de la boîte de collision : debout, elle va de −24 (pieds) à +32 (sommet de la tête) autour de l'origine.

La projection part donc de `origin + viewheight` et dessine la boîte de `origin − 24` à `origin + 32`.

## 8. Limites

- Le volet serveur ne fonctionne qu'en partie hébergée localement : contre un serveur distant, `g_entities[]` n'est pas dans la mémoire du processus.
- Le RVA de `sv.gentities` est propre à un build d'ioquake3 ; le re-mesurer avec `server-entities` après une mise à jour.
- Un joueur n'apparaît en vert que s'il est dans le champ de vision de la caméra : la boîte est une projection, pas un indicateur directionnel.
- La localisation du snapshot « vivant » est pour l'instant écrite dans `esp.rs` ; `aim-mouse`, `aimbot` et `find-viewangles` utilisent encore la version simple de `locate.rs`. À factoriser.

## 9. Questions de jury associées

**Pourquoi le serveur est-il lisible alors que le client ne l'est pas ?** Parce qu'en listen server les deux tournent dans le même processus ; contre un serveur distant, `g_entities` n'existe pas dans la mémoire locale.

**Comment établir que ce bloc est bien `g_entities` et pas une copie ?** Les copies client ne passent pas le test de cohérence du bloc `r` ; le stride déduit est recoupé avec l'entier `gentitySize` du moteur.

**Pourquoi un stride inconnu ?** Le moteur n'impose que la tête de `gentity_t` ; le reste appartient au mod et sa taille est donnée à l'exécution.

**Quelle leçon de conception réseau ?** Ce que le serveur n'envoie pas ne peut pas fuiter ; la sécurité vient de l'omission côté serveur, pas d'une vérification côté client.