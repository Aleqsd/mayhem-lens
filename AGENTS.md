# Instructions du projet

Lire `README.md`, `docs/cadrage.md`, `docs/donnees.md` et `docs/questions.md` avant de développer. Les propositions et questions non répondues ne sont pas des décisions utilisateur.

## Produit et données

- Le mode est ARAM Mayhem. Ne pas utiliser des statistiques ARAM classiques ou Arena comme substitut.
- Conserver source, patch, dates et identité du champion/augment ; distinguer tiers globaux et spécifiques au champion.
- Un stade inconnu reste inconnu. Une augmentation absente reste sans classement.
- Distinguer une règle personnelle de synergie d'une statistique de parties.
- Ne pas publier de pourcentages de victoire d'augmentations dans l'overlay ; lire la politique Riot référencée dans le cadrage avant une modification du comportement ou une diffusion.
- Ne pas incorporer de datasets tiers sans droits établis. Secrets, credentials LCU, caches et screenshots personnels restent hors de Git et des logs.

## Implémentation

- Socle proposé : Rust natif Windows. Garder les dépendances et la structure proportionnées au produit.
- Isoler les appels Win32/COM et les blocs `unsafe` dans de petites fonctions documentées.
- Affichage non bloquant, clics traversants et focus conservé au jeu.
- Capture/OCR hors du thread d'affichage ; reconnaître seulement les régions utiles lorsque leur contenu change.
- Aucun appel réseau dans le chemin entre reconnaissance et affichage des tiers.
- Utiliser des captures et données synthétiques ou autorisées pour les tests ; ne pas exécuter de mutation du client LoL.
- Ne pas annoncer des performances, une compatibilité ou une approbation Riot à partir de la seule compilation.

## Validation

Pour les changements Rust : `cargo fmt --check`, `cargo check --locked`, puis `cargo clippy --locked -- -D warnings`. Ajouter des tests pour les cas métier susceptibles d'afficher un mauvais tier. La validation des interactions et performances Windows doit suivre `docs/validation.md`.
