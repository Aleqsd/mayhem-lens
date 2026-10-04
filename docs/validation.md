# Plan de validation

## État du prototype 0.1.0

Le code complet doit passer formatage, compilation, Clippy et tests sans lancer une fenêtre. Les tests métier utilisent des fixtures synthétiques. La validation locale du 4 octobre a également téléchargé les données Mayhem réelles de Jinx, vérifié la réutilisation du cache et lu les recommandations FR/EN avec un proxy HTTP invalide : la lecture depuis le cache ne dépend pas du fournisseur.

Le mode `KIWI` a été observé dans l'API locale pendant une partie, par une requête en lecture seule. MakeAppx valide le manifeste et la construction du package. Une signature de développement prépare l'installation ; elle ne vaut pas installation ou confiance du certificat.

Les 30 tests couvrent notamment la séparation des tiers global/champion/stade, le filtrage du pool KIWI, l'identité du joueur, les titres FR/EN ambigus, les doublons spatiaux, les règles personnelles et l'expiration de badges si l'OCR se bloque. Formatage, Clippy avec avertissements bloquants et compilation release passent localement. L'import PE vérifié utilise seulement des bibliothèques système Windows, avec le runtime C lié statiquement.

Cette preuve n'inclut pas le rendu Windows, le passage des clics, le focus, la précision OCR ou les ressources consommées en partie. Ces validations attendent la disponibilité de l'utilisateur : aucun contrôle clavier/souris, lancement d'overlay ou installation pendant son jeu.

## Première preuve technique

Sur la configuration de l'utilisateur, afficher des badges factices au-dessus du jeu sans bordure. Vérifier clics, focus, Alt-Tab, déplacement de fenêtre, DPI et disparition des badges lorsque le jeu quitte le premier plan. Mesurer les ressources consommées pendant cet affichage statique.

Première cible : Windows 11 Professionnel, RTX 5080, trois écrans, jeu sur l'écran principal en 2560 × 1440. Ajouter des cas 1080p et 4K et plusieurs mises à l'échelle à la matrice, sans déduire leur compatibilité d'un succès en 1440p.

## Reconnaissance

Comparer les moteurs OCR sur les mêmes captures de titres, avec les langues, résolutions et mises à l'échelle retenues. Mesurer erreurs, lectures ambiguës et latence. Inclure cartes animées, rerolls, titres longs et changements rapides. Les captures de test doivent être explicitement conservables ; les screenshots personnels restent hors du dépôt.

## Données

Tester avec de petites fixtures synthétiques : séparation tier global/tier champion, stade inconnu, nouveau patch, `null`, augmentation absente et cache ancien. Un test ne doit pas simplement reproduire le parseur ; il doit vérifier un cas pouvant conduire à un mauvais badge.

Vérifier également la séparation tier fournisseur/règle de synergie, les conflits entre règles, les choix précédents inconnus, ainsi que la présentation des routes de builds et options de fin de build sans fabriquer une statistique de build complet.

## Mesures en partie

Mesurer séparément capture, recadrage, OCR, association et affichage. Publier la médiane et les percentiles de latence avec taille d'échantillon et contexte. Mesurer CPU, mémoire et activité GPU au repos, pendant la surveillance, pendant la reconnaissance et lors des rerolls.

Une lecture fiable du snapshot doit fonctionner avec le réseau coupé une fois le cache chargé. Les badges ne doivent pas survivre à un reroll, une fin de partie ou un échec de reconnaissance.

Les budgets chiffrés et la matrice de compatibilité restent à fixer à partir des réponses et du premier prototype. Un gain de FPS ou un coût nul ne doit pas être annoncé sans mesures comparatives.
