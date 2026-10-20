# Reverse côté serveur — `g_entities[]` et le PVS

Document de travail du volet serveur. Commande associée : `qcheat server-entities`.
Statut : **code écrit et testé sur mémoire simulée ; à valider en jeu** (voir §6).

## 1. La question

Le client ne reçoit que les entités que le serveur a décidé d'envoyer (PVS, `sv_snapshot.c`). Pour voir toutes les entités du monde, il faut l'état autoritatif du serveur. En **listen server** (`/devmap` + `/addbot`) client et serveur sont dans le même processus : le tableau `g_entities[MAX_GENTITIES = 1024]` du module `qagame` est donc lisible avec les outils déjà construits.

Ce n'est possible que parce que tu es l'hôte. Un client connecté à un serveur distant n'a jamais ce tableau en mémoire : c'est la frontière que le projet démontre.

## 2. Ce que le moteur sait d'une entité serveur

Le moteur ne connaît pas `gentity_t` en entier. Il exige seulement que le début soit, dans cet ordre, `entityState_t s` puis `entityShared_t r`, et il connaît la **taille totale** que le jeu lui communique via `trap_LocateGameData(gEnts, n, sizeof(gentity_t), ...)`. Conséquence : le tableau s'adresse `base + i × stride`, avec un stride connu seulement à l'exécution.

Point piégeux (vérifié dans les sources) : `entityShared_t` **commence par un `entityState_t` inutilisé** (conservé pour la compatibilité QVM). Tous les champs de `r` sont donc décalés de 208 octets :

| Champ | Offset dans `r` | Offset dans `gentity_t` |
|---|---|---|
| `linked` | 208 | 416 |
| `svFlags` | 216 | 424 |
| `currentOrigin` | 280 | 488 |
| `ownerNum` | 304 | 512 |

`sizeof(entityShared_t) = 308`, donc la « tête » `s + r` fait 516 octets. Jusque-là il n'y a que des champs de 4 octets, donc la disposition est identique en QVM (pointeurs 32 bits) et en module natif 64 bits ; les pointeurs commencent à l'offset 516 (`client`).

## 3. Méthode de localisation (sans adresse en dur)

1. **Vérité terrain côté client.** On lit le snapshot (`cg.snap`, déjà localisé) : les joueurs reçus donnent `number`, `clientNum` et `trBase`.
2. **Recherche du jumeau serveur.** On scanne la mémoire pour des blocs de 516 octets dont `s` correspond (même `number`/`clientNum`, position proche, tolérance car le client a une ou deux frames de retard) **et** dont le bloc `r` est cohérent : `linked == 1`, boîte englobante saine, `r.currentOrigin ≈ s.pos.trBase`. Ce dernier test élimine les copies côté client (`cg_entities[]`, tableau du snapshot), car les octets à cet offset n'y ont pas ce sens.
3. **Déduction du stride.** L'entité `n` est à `base + n × stride`. Deux jumeaux de numéros différents donnent `stride = Δadresse / Δnuméro` ; on vote sur toutes les paires et on exige une taille plausible (multiple de 4, entre 516 et 4096).
4. **Recoupement avec le moteur.** `sv.gentities` est un pointeur vers ce tableau, stocké dans le `.bss` de l'exe, avec `gentitySize` juste après. On cherche dans l'image de l'exe un `u64` égal à `base` : si l'entier à `+8` vaut le stride, c'est `sv.gentities` et son RVA est stable (à réutiliser avec `--sv-rva`, sans scan).

Pourquoi ça marche : c'est la même idée que pour le snapshot (signature structurelle + référence dynamique), avec le client comme étalon pour trouver le serveur.

## 4. Ce que la commande affiche

Pour chaque joueur du serveur : numéro de slot, type (hôte/bot), vivant/mort, position serveur, et surtout **s'il figure dans le snapshot du client**. Les joueurs absents du snapshot sont ceux que le serveur a filtrés (PVS) : c'est la démonstration directe de la limite du wallhack côté client.

## 5. Règle de sécurité intégrée

`server-entities` refuse d'afficher quoi que ce soit si plus d'un joueur **n'est pas** marqué `SVF_BOT` (`sdk::bots_only_verdict`). Le contrôle échoue côté sûr : si l'offset de `svFlags` était faux, les bots ne porteraient pas le drapeau, plusieurs « humains » apparaîtraient et la commande refuserait. Limite connue : un humain spectateur n'est pas lié au monde et n'est pas compté.

Cette règle protège la nouvelle commande. Elle n'est **pas encore** branchée sur `aimbot`, `aim-mouse` et `esp` (voir feuille de route).

## 6. À valider en jeu (hypothèses non confirmées)

- Offset `r.svFlags = 424` et valeur `SVF_BOT = 0x8` : le contrôle de §5 le teste indirectement (partie avec bots → doit afficher « OK »).
- Que la fenêtre par défaut (`0x07000000 ± 0x04000000`) contienne le heap de `qagame` ; sinon élargir avec `--center/--range`.
- Disposition de la fin de `server_t` (`gentities`, `gentitySize` à `+8`, `num_entities` à `+12`) : la ligne `MATCH sv.gentities` la confirme ; sinon la ligne `pointer only` donne le voisinage à étudier.
- Que le snapshot contienne bien l'entité du joueur local (sinon il faut ≥ 2 bots visibles, ou `--stride`).
- Alignement : le scanner générique (`scan_aligned`) lit via `from_bytes` sur un `Vec<u8>` ; correct en pratique mais non garanti par le type.

## 7. Questions de jury associées

**Pourquoi le serveur est-il lisible alors que le client ne l'est pas ?** Parce qu'en listen server les deux tournent dans le même processus ; contre un serveur distant, `g_entities` n'existe pas dans ta mémoire.

**Comment sais-tu que ce bloc est bien `g_entities` et pas une copie ?** Les copies client ne passent pas le test de cohérence du bloc `r` ; le stride déduit est recoupé avec l'entier `gentitySize` du moteur.

**Pourquoi un stride inconnu ?** Le moteur n'impose que la tête de `gentity_t` ; le reste appartient au mod et sa taille est donnée à l'exécution.

**Quelle leçon de conception réseau ?** Ce que le serveur n'envoie pas ne peut pas fuiter ; la sécurité vient de l'omission côté serveur, pas d'une vérification côté client.
