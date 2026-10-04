# Plan de validation

## État initial

Le squelette Rust doit passer formatage, compilation et Clippy. Cela prouve uniquement la validité du squelette, pas celle de l'overlay ni son fonctionnement dans LoL.

## Première preuve technique

Sur la configuration de l'utilisateur, afficher des badges factices au-dessus du jeu sans bordure. Vérifier clics, focus, Alt-Tab, déplacement de fenêtre, DPI et disparition des badges lorsque le jeu quitte le premier plan. Mesurer les ressources consommées pendant cet affichage statique.

## Reconnaissance

Comparer les moteurs OCR sur les mêmes captures de titres, avec les langues, résolutions et mises à l'échelle retenues. Mesurer erreurs, lectures ambiguës et latence. Inclure cartes animées, rerolls, titres longs et changements rapides. Les captures de test doivent être explicitement conservables ; les screenshots personnels restent hors du dépôt.

## Données

Tester avec de petites fixtures synthétiques : séparation tier global/tier champion, stade inconnu, nouveau patch, `null`, augmentation absente et cache ancien. Un test ne doit pas simplement reproduire le parseur ; il doit vérifier un cas pouvant conduire à un mauvais badge.

## Mesures en partie

Mesurer séparément capture, recadrage, OCR, association et affichage. Publier la médiane et les percentiles de latence avec taille d'échantillon et contexte. Mesurer CPU, mémoire et activité GPU au repos, pendant la surveillance, pendant la reconnaissance et lors des rerolls.

Une lecture fiable du snapshot doit fonctionner avec le réseau coupé une fois le cache chargé. Les badges ne doivent pas survivre à un reroll, une fin de partie ou un échec de reconnaissance.

Les budgets chiffrés et la matrice de compatibilité restent à fixer à partir des réponses et du premier prototype. Un gain de FPS ou un coût nul ne doit pas être annoncé sans mesures comparatives.
