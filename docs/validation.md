# Plan de validation

## État du prototype 1.0.0

Le code complet doit passer formatage, compilation, Clippy et tests sans lancer une fenêtre. Les tests métier utilisent des fixtures synthétiques. La validation locale du 4 octobre a également téléchargé les données Mayhem réelles de Jinx, vérifié la réutilisation du cache et lu les recommandations FR/EN avec un proxy HTTP invalide : la lecture depuis le cache ne dépend pas du fournisseur.

Le mode `KIWI` a été observé dans l'API locale pendant une partie, par une requête en lecture seule. MakeAppx valide le manifeste et la construction du package. Une signature de développement prépare l'installation ; elle ne vaut pas installation ou confiance du certificat.

La validation initiale de la version 0.1.0 comprend 30 tests couvrant notamment la séparation des tiers global/champion/stade, le filtrage du pool KIWI, l'identité du joueur, les titres FR/EN ambigus, les doublons spatiaux, les règles personnelles et l'expiration de badges si l'OCR se bloque. Formatage, Clippy avec avertissements bloquants et compilation release passent sur cette version. L'import PE vérifié utilise seulement des bibliothèques système Windows, avec le runtime C lié statiquement. Ce décompte est historique ; les contrôles de la version 1.0.0 doivent également inclure le module de mises à jour.

Cette preuve n'inclut pas le rendu Windows, le passage des clics, le focus, la précision OCR ou les ressources consommées en partie. Ces validations attendent la disponibilité de l'utilisateur : aucun contrôle clavier/souris, lancement d'overlay ou installation pendant son jeu.

La version 1.0.0 passe localement 38 tests, le formatage, la compilation et Clippy sur toutes les cibles avec avertissements bloquants. Elle ajoute la vérification de mises à jour au lancement et dans le menu, ainsi que les diagnostics de reconnaissance et d'affichage. L'installation par `.appinstaller`, la confiance du certificat sur un autre PC et la prise d'effet au lancement suivant ne sont pas validées par une compilation, des tests unitaires ou la création du MSIX. Aucune installation ni validation graphique n'a été effectuée pour cette fonctionnalité.

## Mises à jour

### Contrôles sans installation

Vérifier que le manifeste et le `.appinstaller` généré portent la même identité `Aleqsd.MayhemLens`, le même publisher, la même architecture x64 et la même version à quatre nombres. Le manifeste exige Windows 11 build 22621 minimum. Le `.appinstaller` utilise une URL HTTPS stable pour sa propre source et une URL de MSIX épinglée à la release ; sa version et celle du package doivent progresser à chaque nouvelle publication.

Les tests locaux doivent couvrir les résultats « disponible », « aucune mise à jour », « inconnu » et les erreurs, sans appel de déploiement réel. Vérifier également l'absence d'association App Installer, le refus d'une URL hors du canal attendu et l'absence de faux résultat « à jour » lorsque la vérification a échoué. L'erreur du module de mises à jour ne doit pas empêcher l'overlay de fonctionner.

Relire les options du déploiement : `DeferRegistrationWhenPackagesAreInUse=true`, `AllowUnsigned=false`, `ForceAppShutdown=false` et `ForceTargetAppShutdown=false`. Un déploiement préparé n'est pas une preuve que le processus actif exécute la nouvelle version.

Après publication, vérifier sans identifiants que les URL de la source et du MSIX sont accessibles, que les téléchargements correspondent aux fichiers signés et que l'hébergement fournit les types de contenu, longueurs et requêtes par plages nécessaires à App Installer. Ces contrôles réseau ne prouvent pas l'installation Windows.

### Validation native à effectuer avec l'utilisateur disponible

1. Installer une première version signée en ouvrant son `.appinstaller`, puis vérifier l'identité et l'association App Installer du package installé. Tester séparément une installation depuis un MSIX brut : la vérification seule doit signaler l'absence d'association et le chemin de préparation doit tenter l'association au canal stable.
2. Publier une version supérieure sur le même canal avec la même identité de package et un certificat approuvé. Lancer la première version et vérifier que le contrôle en arrière-plan ne bloque pas l'affichage ni le focus.
3. Vérifier la préparation de la mise à jour pendant que l'application reste ouverte : le processus conserve son identité/version active, les badges restent disponibles et aucun processus du jeu n'est arrêté.
4. Quitter normalement Mayhem Lens, le relancer et vérifier `Package.Id.Version` ainsi que la conservation des réglages et du cache. Le statut doit refléter cette version réellement active.
5. Répéter par le menu de vérification manuelle, en incluant les cas sans mise à jour et réseau indisponible. Tester les lancements par le menu Démarrer, l'alias, un raccourci et la barre des tâches.
6. Vérifier le refus d'un package à signature non approuvée, d'une identité/publisher différents et d'une version inférieure. Après un échec, l'ancienne version doit rester utilisable.

Conserver pour cette validation les versions source/cible, le build Windows, la version d'App Installer et les résultats de déploiement. Ne pas annoncer une mise à jour installée ou silencieuse en partie avant ces observations. Les [sources et limites des API](mises-a-jour.md) précisent la différence entre préparation différée et version active.

## Première preuve technique

### Suivi local du prochain test ARAM Mayhem

Avec l'utilisateur disponible, lancer le package installé puis lui demander de rejoindre une partie ARAM Mayhem. Le suivi lit uniquement les fichiers locaux de l'application, sans déplacer le focus, envoyer de clavier/souris ou conserver une capture. Ce protocole n'a pas encore été exécuté en partie.

`scan-status.json` et `display-status.json` sont des snapshots écrasés au maximum une fois par seconde, avec `timestampUnixMs` et `processId`. Pour le MSIX, le dossier est `%LOCALAPPDATA%\Packages\<famille du package>\LocalState\MayhemLens`, avec une famille commençant par `Aleqsd.MayhemLens_`, stable entre versions. L'exécutable seul utilise `%LOCALAPPDATA%\MayhemLens`. Vérifier leur fraîcheur et l'existence du processus correspondant avant d'interpréter les valeurs ; ils peuvent rester sur disque après l'arrêt.

Le snapshot de scan distingue les observations OCR, les titres associés au catalogue et le groupe de trois offres accepté. Il contient les IDs, noms FR/EN, scores de similarité, tiers du fournisseur, champion et patch, ainsi que `badgesRequestedCount`. Les noms ne sont pas le texte OCR brut et la similarité n'est pas une probabilité calibrée. Les listes de titres sont bornées à 16 entrées, avec un indicateur de troncature ; le nombre total reste disponible.

`captureOcrMs` mesure l'appel de capture/OCR natif, qui peut réutiliser une reconnaissance en mémoire si le contenu n'a pas changé. `associationMs` mesure l'association et le regroupement spatial ; `totalScanMs` inclut aussi la préparation et l'envoi des badges. Ces valeurs concernent la dernière lecture terminée, datée par `lastScanAtUnixMs`. Une phase `scanning` et un horodatage qui cesse de progresser aident à repérer une opération bloquée ; ces durées ne mesurent pas les pixels effectivement affichés.

Le snapshot de rendu est séparé : `visibleWindowCount`, les positions, `displayReason`, la géométrie du jeu et l'âge des derniers badges indiquent ce que les fenêtres Win32 déclarent. Comparer les badges demandés à ces fenêtres sans confondre visibilité native et rendu visible à l'écran, qui doit être confirmé par l'utilisateur.

Pendant un choix, vérifier l'identité du champion, le patch, trois offres distinctes et les tiers correspondants. Après un reroll ou Alt-Tab, vérifier le retrait des fenêtres. Lors d'une lecture manquée, `noOfferGroup` ou `captureOcrError` doit être distingué d'une demande de badges réussie. Les erreurs de scan conservent un contexte fixe et éventuellement un HRESULT, sans texte arbitraire provenant du jeu.

Les écritures se font sur un worker local borné ; un writer indisponible ne bloque pas l'OCR ni ne modifie les recommandations. Aucun historique, envoi externe ou screenshot n'est produit. Une lecture ponctuelle peut être faite avec :

```powershell
Get-ChildItem "$env:LOCALAPPDATA\Packages\Aleqsd.MayhemLens_*\LocalState\MayhemLens\scan-status.json" -File | ForEach-Object { Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json }
Get-ChildItem "$env:LOCALAPPDATA\Packages\Aleqsd.MayhemLens_*\LocalState\MayhemLens\display-status.json" -File | ForEach-Object { Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json }
```

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
