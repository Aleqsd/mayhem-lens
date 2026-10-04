# Architecture proposée

Statut : première implémentation expérimentale. La validation graphique et les mesures en partie restent à faire.

## Socle

Rust est retenu comme point de départ pour son contrôle des ressources et ses [bindings Microsoft Windows](https://github.com/microsoft/windows-rs). Le choix ne suppose aucune supériorité mesurée sur un programme C++ équivalent.

Un seul processus coordonne une boucle de messages Windows, un worker de données/API locale et un worker capture/OCR. Le snapshot et le catalogue préparé passent en mémoire par session ; le worker de reconnaissance n'effectue aucun appel réseau. L'affichage ne se redessine que lorsque ses badges changent. Le polling actif utilise l'intervalle configuré, 900 ms par défaut et borné entre 400 et 5000 ms. Sans trois offres reconnues, la surveillance prend le maximum de cet intervalle et de 1500 ms. Une lecture manquée ramène la cadence au repos ; la relecture manuelle contourne cette attente. Ces réglages ne constituent pas une mesure de latence.

## Responsabilités

| Composant | Responsabilité |
| --- | --- |
| Jeu | Détecter la partie et identifier le champion ; exposer un état interne stable. |
| Données | Charger un snapshot Mayhem versionné ; normaliser les tiers sans perdre leur provenance. |
| Capture | Cibler la fenêtre du jeu, suivre taille/DPI, extraire uniquement les zones utiles à la reconnaissance. |
| Reconnaissance | Identifier les noms dans un catalogue fermé, produire une confiance et gérer les ambiguïtés. |
| Classement | Associer champion, augmentation et éventuellement stade au tier du snapshot. |
| Synergies | Appliquer des règles identifiables aux augmentations précédentes et garder le résultat séparé du tier fournisseur. |
| Builds | Présenter les routes et objets Mayhem disponibles pour le champion, avec leurs options et associations objet × augmentation. |
| Overlay | Positionner les badges, laisser passer les clics, préserver le focus et ne redessiner qu'à la demande. |
| Application | Barre système, réglages, raccourci, diagnostics et cycle de vie. |
| Mises à jour | Lire la release GitHub stable, télécharger et contrôler le MSIX, puis demander sa préparation différée à Windows. |

Ces responsabilités ne prescrivent pas autant de crates. Garder une organisation proportionnée à un petit produit.

## Affichage Windows

Des petites fenêtres `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`, placées au-dessus du jeu sans activation, constituent la piste initiale. Microsoft documente le passage des clics pour une [fenêtre layered transparente](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features#layered-windows).

Direct2D/DirectWrite peut produire une petite bitmap alpha, transmise par `UpdateLayeredWindow` seulement lorsque le contenu change. Microsoft recommande de garder les [fenêtres layered petites pour la performance](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-updatelayeredwindow). DirectComposition pourra être évalué si des besoins de rendu ou des mesures le justifient.

Le plein écran exclusif n'est pas une cible garantie. Les [optimisations plein écran Windows](https://devblogs.microsoft.com/directx/demystifying-full-screen-optimizations/) peuvent modifier le comportement ; la v1 sera validée dans le mode réellement utilisé.

## Capture et reconnaissance

Le code utilise `Windows.Graphics.Capture` avec [CreateForWindow](https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow), disponible depuis Windows 10 1903. Le worker démarre une courte session au poll courant, attend une frame fraîche, ferme la session, puis recadre la région de reconnaissance. Le pool et les ressources D3D sont réutilisés ; aucune session de capture ne reste active entre deux polls ou pendant l'OCR. Le premier plan et la géométrie sont contrôlés avant capture, après capture et après reconnaissance.

Le préfiltre pur `visual_gate` échantillonne le BGRA recadré sans copie ni allocation d'image supplémentaire. Une grille bornée recherche des transitions claires approximativement neutres dans trois colonnes et une bande verticale permissive. `Candidate` est une invitation à OCR ; `Uncertain` couvre notamment contenu non uniforme, contraste faible ou buffer invalide ; `Absent` est réservé à une quasi-uniformité échantillonnée. Ces indices ne constituent ni une reconnaissance d'offre ni une confiance OCR. Couleur, animation, mise à l'échelle ou position peuvent les tromper ; aucun bouton de reroll n'est requis.

Sans régions apprises ni offre active, `Candidate` autorise immédiatement l'OCR au poll ; les autres indices attendent une sonde complète de secours. Cette sonde devient due 3 secondes après le dernier OCR complet sans apprentissage, ou 5 secondes avec des régions apprises. Le premier appel, une invalidation ou une relecture forcée exige également une sonde. Les échéances sont évaluées au prochain poll disponible et ne sont pas des timers exacts. Avec une préférence plus lente, la sonde peut donc intervenir plus tard. Un groupe appris ou actif autorise l'OCR indépendamment du préfiltre ; les quatre régions séparées sont utilisées seulement après apprentissage. Le moteur OCR reste chaud et le worker ne publie que des observations du contexte courant.

Le prototype utilise [Windows.Media.Ocr](https://learn.microsoft.com/en-us/uwp/api/windows.media.ocr), qui exige officiellement une identité de package pour une application desktop. Le packaging MSIX apporte cette identité ; une langue OCR demandée mais absente est signalée sans bloquer l'accès aux réglages. Aucun moteur embarqué de secours n'est livré dans cette version.

## Cache et erreurs

La région OCR est apprise en mémoire après deux groupes de titres cohérents, avec projection entre les dimensions WGC et les pixels physiques. Le cache est divisé en quatre régions : en-tête séparé et trois cellules de cartes. Ces cellules ne sont pas les boîtes étroites du titre précédent : elles conservent une marge latérale et une enveloppe verticale pour une augmentation plus longue ou sur plusieurs lignes après reroll. Une sonde périodique repart de la zone centrale large, en plus de la redécouverte prévue par le calibrage.

La clé de chaque région inclut sa position physique, les offsets du recadrage, les dimensions de la frame et une signature de tous ses pixels, sans seuil de similarité visuelle. Une clé différente détruit la lecture antérieure avant le nouvel OCR ; les autres cellules peuvent conserver leurs lectures. Une sonde complète périodique contourne le cache, et la relecture forcée contourne également la cadence et le préfiltre. Des animations peuvent provoquer davantage d'OCR : leur coût reste à mesurer.

Nouvelle session, langue ou fenêtre, changement de géométrie, perte de premier plan et lecture manquée purgent les états correspondants de cache, calibrage, stabilité et stade. Les badges anciens sont retirés quand aucune offre valide n'est disponible. La géométrie associée aux badges est celle de la capture, conservée jusqu'au rendu : un déplacement ou resize intervenu avant réception interdit leur affichage.

Le moteur distingue similarité exacte, nom approché stable et lecture incertaine. Le stade automatique exige un ordinal explicite dans l'en-tête et deux observations cohérentes du même groupe. Un marqueur absent ou contradictoire efface immédiatement ce stade ; aucun niveau ou compteur de confirmations n'est utilisé comme substitut.

Clé de données minimale : source, patch, champion, augmentation ; stade optionnel. Inclure la date du dataset, la date d'import et la population annoncée.

Les données du champion sont téléchargées au début de la partie, puis conservées en cache. Le réseau reste hors du parcours de choix. En cas d'échec, utiliser un cache compatible selon la politique retenue, ou indiquer l'indisponibilité. Un résultat absent ou incertain n'est jamais converti arbitrairement en mauvais tier.

Les captures sont traitées localement. Télémétrie, collecte de parties et conservation de screenshots ne font pas partie du périmètre demandé.

`scan-status.json` expose `scanIntervalMs` et `captureWork`, avec `visualGate`, `mode`, `ocrRegions` et `cachedRegions`. Les modes sont `visualOnly` (aucun OCR effectué), `full` (zone large relue), `fullCache` (lecture de cette zone réutilisée) et `regions` (en-tête et cellules traités séparément). Les compteurs décrivent les appels ou réutilisations de régions de cette capture, pas le nombre d'offres ni les performances du jeu. Le rendu, la précision FR/EN et les ressources consommées restent à valider en partie.

## Mises à jour

Un worker dédié consulte l'API GitHub publique au lancement et sur demande depuis l'icône système. Il sélectionne un MSIX x64 de version supérieure, vérifie son digest SHA-256 obligatoire et son identité native, puis transmet son chemin local au gestionnaire de packages Windows. Windows vérifie la confiance de la signature et reporte l'inscription lorsque le package est utilisé. Aucun remplacement manuel de l'EXE, arrêt forcé ou redémarrage du jeu n'est demandé. Aucune association App Installer n'est requise ; voir [le fonctionnement et les limites](mises-a-jour.md).

## Configuration retenue pour le prototype

Windows 11, jeu sans bordure sur l'écran principal, trois écrans, 2560 × 1440 pour la première preuve. La position des éléments dépend de la fenêtre du jeu et de son DPI, avec détection des changements. Le français et l'anglais sont pris en charge. MSIX étant accepté, Windows OCR reste une option à comparer avant le choix définitif.
