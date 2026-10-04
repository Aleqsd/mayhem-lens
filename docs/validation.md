# Plan de validation

## État du prototype 1.2.0

Le code complet doit passer formatage, compilation, Clippy et tests sans lancer une fenêtre. Les tests métier utilisent des fixtures synthétiques. La validation locale du 4 octobre a également téléchargé les données Mayhem réelles de Jinx, Brand et Tahm Kench, vérifié la réutilisation du cache et lu les recommandations FR/EN avec un proxy HTTP invalide : la lecture depuis le cache ne dépend pas du fournisseur. Les 446 noms du catalogue FR/EN et leurs 446 variantes avec retour à la ligne sont associés au bon ID dans un test local sans capture.

Le mode `KIWI` a été observé dans l'API locale pendant une partie, par une requête en lecture seule. MakeAppx valide le manifeste et la construction du package. Une signature de développement prépare l'installation ; elle ne vaut pas installation ou confiance du certificat.

La validation initiale de la version 0.1.0 comprend 30 tests couvrant notamment la séparation des tiers global/champion/stade, le filtrage du pool KIWI, l'identité du joueur, les titres FR/EN ambigus, les doublons spatiaux, les règles personnelles et l'expiration de badges si l'OCR se bloque. Formatage, Clippy avec avertissements bloquants et compilation release passent sur cette version. L'import PE vérifié utilise seulement des bibliothèques système Windows, avec le runtime C lié statiquement. Ce décompte est historique ; les contrôles de la version courante couvrent également le module de mises à jour.

Cette preuve n'inclut pas le rendu Windows, le passage des clics, le focus, la précision OCR ou les ressources consommées en partie. Ces validations attendent la disponibilité de l'utilisateur : aucun contrôle clavier/souris, lancement d'overlay ou installation pendant son jeu.

La version 1.0.0 a passé localement 42 tests, le formatage, la compilation et Clippy sur toutes les cibles avec avertissements bloquants. Elle a ajouté les diagnostics de reconnaissance et d'affichage ; les tests couvrent aussi les conflits de raccourcis, la récupération d'un catalogue trop ancien et le rejet de données chargées pour une partie qui a changé. Son hébergement `.appinstaller` a ensuite présenté un type MIME incompatible avec le parcours prévu.

La version 1.0.1 passe le formatage, la compilation, Clippy avec avertissements bloquants et **47 tests**. Un test supplémentaire est ignoré dans le pipeline ordinaire : exécuté séparément sur le MSIX signé 1.0.0.0 archivé, il valide réellement les bindings du lecteur Windows et le refus d'un nom, éditeur ou numéro de version différents. Cette lecture n'installe rien et ne vérifie pas la confiance de la signature. Le téléchargement est testé sur plusieurs blocs, avec lectures courtes, corruption et erreurs d'entrée/sortie. La confiance du certificat sur un autre PC et la prise d'effet d'une mise à jour au lancement suivant restent à valider. Aucune installation ni validation graphique n'a été effectuée pour cette fonctionnalité.

## Améliorations 1.1.0 : contrôles sans fenêtre

La validation locale passe le formatage, la compilation, Clippy sur toutes les cibles avec avertissements bloquants et **69 tests** (aucun échec, un test de fixture MSIX ignoré comme précédemment). Aucun de ces contrôles ne lance la fenêtre de réglages, la capture, l'overlay ou un déploiement Windows.

Les fixtures synthétiques couvrent le calibrage à 720p, 1080p, 1440p et 4K, avec origine d'écran négative, changement de résolution, déplacement de colonnes et lecture manquée. Vérifier que la redécouverte rétablit une zone bornée, que le recadrage inclut l'en-tête et que la clé de réutilisation comprend ses offsets. La géométrie capturée doit être transmise au rendu ; un déplacement entre lecture et réception ne doit jamais réassocier les anciens badges à une nouvelle fenêtre.

Tester les noms exacts, approchés et insuffisants : un changement de cartes remet la stabilité à zéro, et une lecture incertaine ne doit exposer aucun tier ni conseil d'objets. Tester les marqueurs de stade FR/EN uniquement au-dessus de trois cartes, les marqueurs contradictoires, la disparition de l'en-tête et les rerolls. Les fixtures de libellés ne prouvent pas que ces formats sont présents dans le client actuel ; observer les véritables en-têtes pendant le prochain test. Niveau, temps et nombre de choix enregistrés ne doivent jamais fournir un stade de secours.

Tester la migration du JSON 1.0.1, les nombres invalides/NaN, les raccourcis équivalents en doublon et la sauvegarde de préférences pendant des changements de session. Les choix enregistrés et règles personnelles doivent être préservés ; un stade non édité ne doit pas rétablir un override devenu ancien. Tab, Maj+Tab, Entrée, Échap, combobox, erreurs de validation, page de mises à jour, DPI et rendu des textes restent à tester dans une vraie fenêtre avec l'utilisateur disponible.

La progression de téléchargement doit rester monotone et bornée. Une corruption ne doit jamais produire un état prêt. Un reçu accepté est conservé ; une confirmation exige la version active exacte et doit rester visible hors ligne, y compris si l'enregistrement de la date de confirmation échoue. Ces tests ne préparent aucune mise à jour Windows.

## Installateur 1.1.1 : contrôles distincts

La compilation et les tests du binaire `mayhem-lens-setup` ne lancent pas son interface. Les résultats finaux de cette version doivent être consignés après le pipeline complet ; le décompte de 69 tests ci-dessus décrit la version 1.1.0.

Les contrôles locaux de la version 1.1.1 passent le formatage, la compilation sur toutes les cibles, Clippy avec avertissements bloquants et **74 tests** (aucun échec, un test historique de fixture MSIX ignoré). Les nouveaux tests couvrent les pins de payload, les limites de plateforme, le refus de rétrogradation, la preuve d'inscription exacte et le signal OCR non bloquant. L'aperçu de l'installateur utilise son renderer GDI dans un bitmap mémoire, sans créer de fenêtre ni capturer l'écran ; cette vérification visuelle ne valide pas les interactions natives.

Vérifier sans installation que le Setup intègre le MSIX de la bonne identité/version/x64 et le certificat public seul, avec les SHA-256 attendus. Le script doit refuser un manifeste incohérent, une clé/PFX, un certificat différent du Publisher, un payload trop volumineux et une compilation échouée. Vérifier la restauration des variables d'environnement, y compris après une erreur. La CI utilise une clé éphémère en mémoire, exporte uniquement son certificat public et conserve des artefacts explicitement **UNSIGNED** ; elle ne modifie aucun magasin de confiance.

Sur une installation réelle, avec l'utilisateur disponible, vérifier l'interface sombre, le clavier et le DPI, le consentement explicite, l'annulation et le refus UAC, ainsi que l'absence de nouvelle demande d'élévation lorsque le certificat est déjà approuvé. Seul le helper de confiance doit être élevé ; le package doit appartenir à l'utilisateur initial. Vérifier ensuite l'installation signée, les erreurs de signature et le lancement exclusivement par le bouton ou le menu Démarrer. Une langue OCR manquante doit produire un avertissement, sans installation silencieuse de fonctionnalités Windows. Vérifier enfin le parcours manuel du MSIX et la conservation des réglages lors d'une mise à jour.

Les contrôles de packaging et la signature des fichiers ne prouvent pas que Windows acceptera le certificat ni que l'interface, l'installation et le lancement fonctionnent sur un autre PC. Ces validations restent en attente.

## Reconnaissance adaptative : validation du code courant

Les nouveaux contrôles doivent couvrir le préfiltre, la cadence et les caches sans fenêtre ni capture personnelle. Les décomptes des versions précédentes ci-dessus restent historiques ; consigner le résultat final après le pipeline complet du changement.

Le préfiltre `Candidate`/`Uncertain`/`Absent` n'est jamais une preuve de présence, d'absence ou d'identité des offres. Les fixtures synthétiques vérifient des titres dans trois colonnes à différentes tailles et hauteurs, les lignes décalées et titres sur plusieurs lignes, une région uniforme sombre ou claire, un seul texte, des rectangles, un contraste faible, des titres colorés et des buffers invalides. Un faux candidat ne doit pas produire de tier sans les validations OCR et catalogue habituelles. Un faux négatif doit garder la sonde OCR complète de secours, y compris avec `Absent`.

Vérifier que trois offres reconnues activent l'intervalle configuré, **900 ms par défaut**, borné de 400 à 5000 ms ; une lecture manquée ramène la surveillance à un minimum de **1500 ms** sans accélérer un réglage utilisateur plus lent. La sonde OCR complète devient due après **3 secondes** sans région apprise, ou **5 secondes** avec apprentissage. Ces conditions sont évaluées lors des polls ; tester également les configurations lentes sans annoncer un timer exact ni une latence garantie de 3 ou 5 secondes.

Le cache sépare l'en-tête et trois cellules larges couvrant les remplacements longs ou sur plusieurs lignes après reroll. Vérifier qu'une signature de pixels ou une géométrie modifiée invalide la lecture de cette région avant OCR, que les autres régions peuvent être réutilisées et qu'un en-tête changé ne conserve pas un ancien stade. Tester la purge sur nouvelle session, langue, fenêtre, changement de résolution/position, perte de premier plan et lecture manquée. La sonde complète périodique doit contourner les caches ; **Ctrl+Shift+M**, ou le raccourci configuré, doit aussi contourner la cadence et le préfiltre pour relire la zone large.

L'essai réel doit vérifier la première offre, les rerolls d'une carte, les titres FR/EN longs ou sur plusieurs lignes, les cartes sombres/colorées, les animations et une interface sans bouton de reroll disponible. Aucun bouton ni position exacte de titre n'est une condition de la détection. Mesurer séparément le coût de capture, préfiltre, OCR complet, lectures de régions et réutilisations ; aucune économie de CPU, de GPU ou de latence n'est déduite de la compilation ou des fixtures.

## Reconnaissance 1.2.0 : contrôles sans capture

La validation locale passe le formatage, la compilation sur toutes les cibles, Clippy avec avertissements bloquants et **94 tests** (aucun échec, une fixture MSIX historique ignorée). Les vingt nouveaux tests utilisent uniquement des pixels, observations et géométries synthétiques ; ils ne démarrent ni WGC, ni OCR Windows, ni fenêtre.

Ils couvrent les indices visuels faibles ou absents, les échéances de secours, la relecture forcée, les transitions de cadence, l'invalidation indépendante des cartes et de l'en-tête, les changements de géométrie et les titres longs sur deux lignes. Les cellules couvrent aussi un remplacement central de 40 % de largeur, admis par l'association des offres. Un fragment voisin ou un titre décentré ne peut pas devenir une offre régionale : une nouvelle recherche globale doit rétablir la géométrie. Après une vraie lecture globale, les quatre caches repartent des observations globales courantes.

Ces contrôles ne prouvent pas la précision OCR, la latence ni le coût CPU/GPU en partie. Le prochain test doit comparer les compteurs `ocrRegions`/`cachedRegions`, le temps de reconnaissance et les cartes réellement proposées, puis vérifier un reroll, la disparition du choix et Alt-Tab. Aucun logiciel n'a été installé, aucune confiance ajoutée et aucun contrôle d'écran effectué pour cette évolution.

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

`captureWork.visualGate` expose `candidate`, `uncertain` ou `absent`, sans fournir une probabilité OCR. `captureWork.mode` distingue `visualOnly`, `full`, `fullCache` et `regions` ; `captureWork.ocrRegions` compte les régions reconnues par OCR pendant cette capture, et `captureWork.cachedRegions` les lectures réutilisées. En mode `regions`, leur somme décrit l'en-tête et les trois cellules ; en mode `full`, une seule région large est relue. `scanIntervalMs` est l'intervalle appliqué avant la capture terminée et peut changer au scan suivant selon les offres reconnues. Interpréter ces compteurs avec le mode et la fraîcheur du snapshot, jamais comme une preuve de précision ou de rendu.

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
