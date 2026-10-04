# Plan de validation

## État du prototype 1.0.1

Le code complet doit passer formatage, compilation, Clippy et tests sans lancer une fenêtre. Les tests métier utilisent des fixtures synthétiques. La validation locale du 4 octobre a également téléchargé les données Mayhem réelles de Jinx, Brand et Tahm Kench, vérifié la réutilisation du cache et lu les recommandations FR/EN avec un proxy HTTP invalide : la lecture depuis le cache ne dépend pas du fournisseur. Les 446 noms du catalogue FR/EN et leurs 446 variantes avec retour à la ligne sont associés au bon ID dans un test local sans capture.

Le mode `KIWI` a été observé dans l'API locale pendant une partie, par une requête en lecture seule. MakeAppx valide le manifeste et la construction du package. Une signature de développement prépare l'installation ; elle ne vaut pas installation ou confiance du certificat.

La validation initiale de la version 0.1.0 comprend 30 tests couvrant notamment la séparation des tiers global/champion/stade, le filtrage du pool KIWI, l'identité du joueur, les titres FR/EN ambigus, les doublons spatiaux, les règles personnelles et l'expiration de badges si l'OCR se bloque. Formatage, Clippy avec avertissements bloquants et compilation release passent sur cette version. L'import PE vérifié utilise seulement des bibliothèques système Windows, avec le runtime C lié statiquement. Ce décompte est historique ; les contrôles de la version courante couvrent également le module de mises à jour.

Cette preuve n'inclut pas le rendu Windows, le passage des clics, le focus, la précision OCR ou les ressources consommées en partie. Ces validations attendent la disponibilité de l'utilisateur : aucun contrôle clavier/souris, lancement d'overlay ou installation pendant son jeu.

La version 1.0.0 a passé localement 42 tests, le formatage, la compilation et Clippy sur toutes les cibles avec avertissements bloquants. Elle a ajouté les diagnostics de reconnaissance et d'affichage ; les tests couvrent aussi les conflits de raccourcis, la récupération d'un catalogue trop ancien et le rejet de données chargées pour une partie qui a changé. Son hébergement `.appinstaller` a ensuite présenté un type MIME incompatible avec le parcours prévu.

La version 1.0.1 passe le formatage, la compilation, Clippy avec avertissements bloquants et **47 tests**. Un test supplémentaire est ignoré dans le pipeline ordinaire : exécuté séparément sur le MSIX signé 1.0.0.0 archivé, il valide réellement les bindings du lecteur Windows et le refus d'un nom, éditeur ou numéro de version différents. Cette lecture n'installe rien et ne vérifie pas la confiance de la signature. Le téléchargement est testé sur plusieurs blocs, avec lectures courtes, corruption et erreurs d'entrée/sortie. La confiance du certificat sur un autre PC et la prise d'effet d'une mise à jour au lancement suivant restent à valider. Aucune installation ni validation graphique n'a été effectuée pour cette fonctionnalité.

## Mises à jour

### Contrôles sans installation

Vérifier la cohérence de la version Cargo, du tag de release, du nom de l'asset et de l'identité du manifeste : `Aleqsd.MayhemLens`, publisher `CN=Alexandre DO-O ALMEIDA`, architecture x64, version à quatre nombres. Le manifeste exige Windows 11 build 22621 minimum. Le MSIX généré n'est pas signé avant l'étape dédiée ; aucun fichier `.appinstaller` n'est nécessaire.

Les tests locaux doivent couvrir les métadonnées de release stable, la sélection d'une version supérieure et les cas sans mise à jour, sans téléchargement ni déploiement réels. Refuser brouillons, préversions, tags ou noms d'asset invalides, absence de digest SHA-256, tailles invalides, URL hors du dépôt/tag attendu et redirections non autorisées. Vérifier les échecs réseau, dont la limite d'API GitHub, et l'absence de faux résultat « à jour » lorsque la vérification échoue. L'erreur du module de mises à jour ne doit pas empêcher l'overlay de fonctionner.

Tester séparément le calcul du hash, le refus d'un fichier tronqué ou modifié et la validation native de l'identité d'un MSIX de test. Le téléchargement est borné à 128 Mio, doit correspondre à la taille annoncée et au digest obligatoire ; il accepte seulement des redirections HTTPS vers les hôtes GitHub autorisés. Ces tests ne doivent pas appeler le déploiement Windows.

Relire les options du déploiement : `DeferRegistrationWhenPackagesAreInUse=true`, `AllowUnsigned=false`, `ForceAppShutdown=false` et `ForceTargetAppShutdown=false`. Un déploiement préparé n'est pas une preuve que le processus actif exécute la nouvelle version.

Après publication, lire l'API GitHub sans identifiants et vérifier la release stable, la présence du MSIX exact, son URL, sa taille et son digest `sha256:…`. Vérifier que le téléchargement correspond au MSIX signé publié. Un téléchargement en `application/octet-stream` convient à ce parcours local ; ces contrôles réseau ne prouvent pas l'installation Windows.

### Validation native à effectuer avec l'utilisateur disponible

1. Approuver le certificat public de développement dans `LocalMachine\TrustedPeople`, puis télécharger et ouvrir le MSIX signé d'une première version utilisant ce mécanisme (1.0.1 ou ultérieure). Vérifier l'identité du package installé et son démarrage depuis **Mayhem Lens**. Aucune association App Installer n'est nécessaire.
2. Publier une version supérieure sur le même canal avec la même identité de package et un certificat approuvé. Lancer la première version et vérifier que le contrôle en arrière-plan ne bloque pas l'affichage ni le focus.
3. Vérifier la préparation de la mise à jour pendant que l'application reste ouverte : le processus conserve son identité/version active, les badges restent disponibles et aucun processus du jeu n'est arrêté.
4. Quitter normalement Mayhem Lens, le relancer et vérifier `Package.Id.Version` ainsi que la conservation des réglages et du cache. Le statut doit refléter cette version réellement active.
5. Répéter par le menu de vérification manuelle, en incluant les cas sans mise à jour et réseau indisponible. Tester les lancements par le menu Démarrer, l'alias, un raccourci et la barre des tâches.
6. Vérifier le refus d'un asset à digest absent, d'un fichier tronqué ou modifié, d'une identité/publisher/architecture différents, d'une version inférieure et d'une signature non approuvée. Après un échec, l'ancienne version doit rester utilisable.

Conserver pour cette validation les versions source/cible, le build Windows, l'asset/digest vérifié et les résultats de déploiement. Ne pas annoncer une mise à jour installée ou silencieuse en partie avant ces observations. Les [sources et limites des API](mises-a-jour.md) précisent la différence entre préparation différée et version active.

## Première preuve technique

### Suivi local du prochain test ARAM Mayhem

Avec l'utilisateur disponible, lancer le package installé puis lui demander de rejoindre une partie ARAM Mayhem. Le suivi lit uniquement les fichiers locaux de l'application, sans déplacer le focus, envoyer de clavier/souris ou conserver une capture. Ce protocole n'a pas encore été exécuté en partie.

`scan-status.json` et `display-status.json` sont des snapshots écrasés au maximum une fois par seconde, avec `timestampUnixMs` et `processId`. Pour le MSIX, le dossier est `%LOCALAPPDATA%\Packages\<famille du package>\LocalState\MayhemLens`, avec une famille commençant par `Aleqsd.MayhemLens_`, stable entre versions. L'exécutable seul utilise `%LOCALAPPDATA%\MayhemLens`. Vérifier leur fraîcheur et l'existence du processus correspondant avant d'interpréter les valeurs ; ils peuvent rester sur disque après l'arrêt.

Le snapshot de scan distingue les observations OCR, les titres associés au catalogue et le groupe de trois offres accepté. Il contient les IDs, noms FR/EN, scores de similarité, tiers du fournisseur, champion et patch, ainsi que `badgesRequestedCount`. Les noms ne sont pas le texte OCR brut et la similarité n'est pas une probabilité calibrée. Les listes de titres sont bornées à 16 entrées, avec un indicateur de troncature ; le nombre total reste disponible.

`captureOcrMs` mesure l'appel de capture/OCR natif, qui peut réutiliser une reconnaissance en mémoire si le contenu n'a pas changé. `associationMs` mesure l'association et le regroupement spatial ; `totalScanMs` inclut aussi la préparation et l'envoi des badges. Ces valeurs concernent la dernière lecture terminée, datée par `lastScanAtUnixMs`. Une phase `scanning` et un horodatage qui cesse de progresser aident à repérer une opération bloquée ; ces durées ne mesurent pas les pixels effectivement affichés.

Le snapshot de rendu est séparé : `visibleWindowCount`, les positions, `displayReason`, la géométrie du jeu et l'âge des derniers badges indiquent ce que les fenêtres Win32 déclarent. Comparer les badges demandés à ces fenêtres sans confondre visibilité native et rendu visible à l'écran, qui doit être confirmé par l'utilisateur.

`hotkeyWarnings` indique les raccourcis indisponibles avec leur code numérique. Un conflit doit laisser démarrer le tray et les badges ; vérifier la relecture et l'arrêt depuis le menu. La confirmation des choix reste dépendante des raccourcis 1/2/3, dont l'indisponibilité est signalée.

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
