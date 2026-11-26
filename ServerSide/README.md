# ServerSide

Tout ce qui concerne le volet serveur du projet : l'étude (`server-reverse.md`) et l'index du code correspondant. Le code Rust reste dans le workspace Cargo (`crates/`), car Cargo exige cette organisation ; ce dossier indique où le trouver.

## Documents

| Fichier | Contenu |
|---|---|
| [`server-reverse.md`](server-reverse.md) | Ce que sait le serveur, méthode de localisation de `g_entities[]`, validation en jeu, limites |

## Code

| Fichier | Rôle |
|---|---|
| `crates/sdk/src/server.rs` | Miroir de `entityShared_t` / `gentity_t` (tête), appariement client/serveur, déduction du stride, verdict « bots seulement ». Logique pure, testée. |
| `crates/sdk/src/plausibility.rs` | Filtres de plausibilité des snapshots, utilisés par les locateurs. |
| `crates/cli/src/locate.rs` | Locateurs partagés : snapshot client, puis `g_entities[]` (`locate_gentities`). |
| `crates/cli/src/cmd/server_entities.rs` | Commande `qcheat server-entities`. |
| `crates/cli/src/cmd/esp.rs` | Option `--server` de `qcheat esp` (lecture de `g_entities[]` pendant l'affichage). |

## Valeurs mesurées

Enregistrées dans `offsets.json` (bloc `server_side`) : `sizeof(gentity_t)` = 804 octets, `sv.gentities` à `exe + 0x1C0DB30` sur le build testé. Le RVA est propre à un build d'ioquake3 : le re-mesurer avec `qcheat server-entities` après une mise à jour.

## Prérequis d'utilisation

Partie hébergée localement (`/devmap` puis `/addbot`) : le code client et le code serveur doivent s'exécuter dans le même processus.
