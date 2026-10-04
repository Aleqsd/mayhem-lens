# Mayhem Lens

Overlay Windows léger pour afficher les tiers des augmentations de **League of Legends — ARAM Mayhem**, en fonction du champion joué.

**État : version expérimentale 1.0.1.** Cache Mayhem, reconnaissance locale FR/EN, classement, conseils d'objets, overlay natif et mises à jour sont implémentés. La compilation et les tests métier sont séparés de la validation en jeu : focus, rendu, précision OCR, performances et application effective des mises à jour restent à vérifier sur Windows, avec l'utilisateur disponible. Le numéro de version ne signifie pas que ces validations sont achevées.

## Objectif

Identifier le champion, reconnaître automatiquement les trois augmentations proposées, puis afficher une lettre et une courte explication près de chaque carte. Ajouter des règles de synergie avec les choix précédents et des conseils d'objets/builds Mayhem. Les données du champion sont récupérées au début de la partie et consultées localement pour garder le réseau hors du chemin critique.

Public initial : utilisateur et quelques amis. Langues : français et anglais. Cible initiale : Windows 11, jeu sans bordure sur l'écran principal en 1440p ; autres résolutions et mises à l'échelle à supporter. Installation classique, MSIX accepté.

La source doit mesurer **Mayhem**. Les statistiques ARAM classiques et les classements Arena ne remplacent pas les données Mayhem.

## Implémentation

Rust + bindings Microsoft `windows`, petites fenêtres Win32 transparentes, Direct2D/DirectWrite à la demande, capture Windows.Graphics.Capture et Windows OCR. L'application demande une identité MSIX et la fonctionnalité OCR de la langue choisie ; l'EXE non installé peut servir aux commandes de données, mais son lancement ne contourne pas cette exigence Windows.

- L'API locale en lecture identifie le joueur et filtre strictement `KIWI`. Les modes `ARAM`, `CHERRY` et `KIWI_JADE` sont exclus.
- ARAMKit fournit les tiers **du champion**, leurs splits de choix et les routes de builds Mayhem. CommunityDragon fournit les noms FR/EN et le pool KIWI, au même patch.
- Le téléchargement se fait au chargement de la partie. Le worker OCR utilise uniquement le snapshot en mémoire ; aucun appel fournisseur pendant un choix.
- Trois titres distincts, alignés et suffisamment espacés sont nécessaires pour afficher les badges. Ambiguïté, Alt-Tab, changement de géométrie et lecture manquée retirent les badges.
- La sélection précédente est confirmée par raccourci. Le stade est explicitement renseigné via le menu système ou les réglages ; il n'est pas déduit du niveau.
- Les règles personnelles de synergie sont configurables et restent distinctes du tier. **Aucune base de combos mécaniques non vérifiés n'est embarquée.** Les associations objet × augmentation du fournisseur restent observationnelles.
- Une vérification des releases GitHub démarre en arrière-plan au lancement. Le MSIX téléchargé est contrôlé par SHA-256 et identité de package, puis Windows vérifie sa signature et prépare son inscription différée. Le menu système permet aussi une recherche manuelle ; l'application ne demande aucun arrêt forcé.
- Aucune télémétrie externe, conservation de captures, lecture mémoire du jeu, injection, clic automatique ni affichage de win rates d'augmentations.

## Documents

- [Cadrage produit](docs/cadrage.md)
- [Architecture proposée](docs/architecture.md)
- [Sources et qualité des données](docs/donnees.md)
- [Dix questions de cadrage](docs/questions.md)
- [Plan de validation](docs/validation.md)
- [Installation et mises à jour](docs/installation.md)
- [Fonctionnement des mises à jour](docs/mises-a-jour.md)

## Compiler et préparer l'installation

```powershell
cargo fmt --check
cargo check --locked
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
./scripts/package.ps1
```

`package.ps1` produit un MSIX **non signé** dans `dist/`, avec validation MakeAppx, images propres et licences des dépendances. La cible est Windows 11 22H2 ou plus récent. Il ne l'installe pas, n'approuve aucun certificat et ne démarre pas l'overlay. Voir [l'installation](docs/installation.md) pour la signature et la mise en service ultérieure.

La CI Windows exécute ces contrôles et conserve le MSIX non signé comme artefact. La clé de développement reste hors Git et hors du package.

Après installation signée, ouvrir **Mayhem Lens** depuis Windows, ou utiliser l'alias `mayhem-lens.exe run`. Il attend une partie Mayhem ; l'icône système donne accès à la relecture, au stade, aux réglages et à l'arrêt.

Pour installer la [dernière version](https://github.com/Aleqsd/mayhem-lens/releases/latest), télécharger le certificat public et le MSIX signé. Approuver le certificat de développement dans `LocalMachine\TrustedPeople`, puis ouvrir le MSIX et installer Mayhem Lens. Ce parcours demande des droits administrateur pour le certificat et ne nécessite aucune association App Installer. Une version préparée par l'application doit prendre effet après fermeture et relance ; sa version active reste à vérifier.

| Action | Raccourci |
| --- | --- |
| Relire immédiatement les cartes | Ctrl+Shift+M |
| Confirmer son choix gauche / milieu / droite | Ctrl+Shift+1 / 2 / 3 |
| Quitter | Ctrl+Shift+Q |

La confirmation enregistre une augmentation pour les conseils de l'application : elle n'agit pas dans LoL. Un seul choix est enregistré par offre reconnue. Dans le package MSIX, les réglages et le cache sont dans `%LOCALAPPDATA%\Packages\<famille du package>\LocalState\MayhemLens`, un emplacement stable entre versions. L'exécutable seul utilise `%LOCALAPPDATA%\MayhemLens`. Un nouveau contexte de partie remet les choix et le stade à zéro.

Si un autre logiciel réserve un raccourci, l'application continue à fonctionner et indique la combinaison indisponible dans son menu. Relecture et arrêt restent accessibles dans ce menu ; la confirmation d'un choix dépend du raccourci correspondant.

## Observer le prochain test en partie

Après lancement, des snapshots locaux remplacés au maximum une fois par seconde permettent un suivi en lecture seule dans le même dossier de données : `%LOCALAPPDATA%\Packages\<famille du package>\LocalState\MayhemLens` pour le MSIX, `%LOCALAPPDATA%\MayhemLens` pour l'exécutable seul. La famille installée commence par `Aleqsd.MayhemLens_`.

- `scan-status.json` : champion/patch, premier plan du jeu, nombres de lignes OCR et de titres/offres reconnus, noms du catalogue, IDs, similarité et tiers, durées, erreurs et nombre de badges demandés.
- `display-status.json` : fenêtres natives demandées/visibles, positions, expiration, motif d'affichage ou de masquage et raccourcis indisponibles. La visibilité Win32 n'est pas une preuve des pixels vus par le joueur.

Vérifier l'âge du snapshot et son `processId` : un fichier ancien peut rester après la fermeture. Les noms proviennent du catalogue, pas du texte OCR brut ; aucune capture ni identité de joueur n'est conservée. Ce dispositif prépare le prochain test ARAM Mayhem ; il n'est pas une preuve que le rendu ou les performances en partie ont déjà été validés. Voir [le protocole de suivi](docs/validation.md#suivi-local-du-prochain-test-aram-mayhem).

## Données sans fenêtre

```powershell
cargo run --locked -- --help
cargo run --locked -- sync 222
cargo run --locked -- recommend Jinx "Goliath" "Jeweled Gauntlet"
cargo run --locked -- config stage 2
cargo run --locked -- config selected 1045
cargo run --locked -- config language en
cargo run --locked -- diagnose
cargo run --locked -- update check
```

`recommend` est strictement hors ligne : utiliser `sync` une première fois. `--cache <dossier>` et `--config <fichier>` permettent un environnement de test séparé. Le cache affiche patch et dates ; un ancien patch associé à un nouveau catalogue est refusé. En cas de téléchargement indisponible, l'overlay peut reprendre un cache compatible avec mention explicite.

Les trois objets d'une route observée ne constituent pas une preuve de build complet à six objets. Départs, bottes et options ultérieures sont présentés séparément par le moteur. Le panneau initial de l'overlay présente la première route ; les autres détails sont consultables par `recommend`.

## Développement

Le travail est réalisé avec des agents Codex. Les choix produit sont consignés dans les documents du dépôt ; les décisions encore ouvertes restent explicitement provisoires.

Le code et les installateurs sont publics à la demande de l'utilisateur. Aucune licence de redistribution des datasets tiers n'est présumée : ils ne sont pas inclus dans les versions publiées.
