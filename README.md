# Mayhem Lens

Overlay Windows léger pour afficher les tiers des augmentations de **League of Legends — ARAM Mayhem**, en fonction du champion joué.

**État : cadrage initial.** Le dépôt contient les conclusions de recherche et un squelette Rust compilable. L'overlay, la capture, l'OCR et les téléchargements ne sont pas encore implémentés.

## Objectif

Identifier le champion, reconnaître les trois augmentations proposées, puis afficher un badge discret près de chaque carte. Les données sont récupérées avant le choix et consultées localement pour garder le réseau hors du chemin critique.

La source doit mesurer **Mayhem**. Les statistiques ARAM classiques et les classements Arena ne remplacent pas les données Mayhem.

## Proposition technique

Rust + bindings Microsoft `windows`, fenêtres Win32 transparentes, rendu Direct2D/DirectWrite à la demande, capture Windows.Graphics.Capture et OCR local. Le moteur OCR et la distribution restent à choisir après cadrage et mesures.

## Documents

- [Cadrage produit](docs/cadrage.md)
- [Architecture proposée](docs/architecture.md)
- [Sources et qualité des données](docs/donnees.md)
- [Dix questions de cadrage](docs/questions.md)
- [Plan de validation](docs/validation.md)

## Commandes du squelette

```powershell
cargo fmt --check
cargo check --locked
cargo clippy --locked -- -D warnings
cargo run --locked
```

L'exécutable actuel indique seulement que le projet est en phase de cadrage. Ces commandes ne valident pas encore un fonctionnement dans LoL.

## Développement

Le travail est réalisé avec des agents Codex. Les choix produit sont consignés dans les documents du dépôt ; les décisions encore ouvertes restent explicitement provisoires.

Le dépôt démarre privé. Aucune licence de redistribution des datasets tiers n'est présumée.
