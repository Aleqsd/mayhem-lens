# Architecture proposée

Statut : première implémentation expérimentale. La validation graphique et les mesures en partie restent à faire.

## Socle

Rust est retenu comme point de départ pour son contrôle des ressources et ses [bindings Microsoft Windows](https://github.com/microsoft/windows-rs). Le choix ne suppose aucune supériorité mesurée sur un programme C++ équivalent.

Un seul processus coordonne une boucle de messages Windows, un worker de données/API locale et un worker capture/OCR. Le snapshot et le catalogue préparé passent en mémoire par session ; le worker de reconnaissance n'effectue aucun appel réseau. L'affichage ne se redessine que lorsque ses badges changent. Le polling automatique initial est de 900 ms, configurable entre 400 et 5000 ms ; il ne constitue pas une mesure de latence.

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

Piste : `Windows.Graphics.Capture` avec [CreateForWindow](https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow), disponible depuis Windows 10 1903. Les frames arrivent sur un worker et les régions des titres sont recadrées avant reconnaissance.

Une détection légère surveille l'apparition ou le changement des cartes. L'OCR n'est pas lancé sur chaque frame. Conserver le moteur chaud, éviter les travaux simultanés obsolètes et ne publier que le résultat correspondant aux dernières cartes détectées.

Le prototype utilise [Windows.Media.Ocr](https://learn.microsoft.com/en-us/uwp/api/windows.media.ocr), qui exige officiellement une identité de package pour une application desktop. Le packaging MSIX apporte cette identité ; la langue OCR demandée est vérifiée avant le démarrage. Aucun moteur embarqué de secours n'est livré dans cette version.

## Cache et erreurs

Clé de données minimale : source, patch, champion, augmentation ; stade optionnel. Inclure la date du dataset, la date d'import et la population annoncée.

Les données du champion sont téléchargées au début de la partie, puis conservées en cache. Le réseau reste hors du parcours de choix. En cas d'échec, utiliser un cache compatible selon la politique retenue, ou indiquer l'indisponibilité. Un résultat absent ou incertain n'est jamais converti arbitrairement en mauvais tier.

Les captures sont traitées localement. Télémétrie, collecte de parties et conservation de screenshots ne font pas partie du périmètre demandé.

## Mises à jour

Un worker dédié consulte l'API GitHub publique au lancement et sur demande depuis l'icône système. Il sélectionne un MSIX x64 de version supérieure, vérifie son digest SHA-256 obligatoire et son identité native, puis transmet son chemin local au gestionnaire de packages Windows. Windows vérifie la confiance de la signature et reporte l'inscription lorsque le package est utilisé. Aucun remplacement manuel de l'EXE, arrêt forcé ou redémarrage du jeu n'est demandé. Aucune association App Installer n'est requise ; voir [le fonctionnement et les limites](mises-a-jour.md).

## Configuration retenue pour le prototype

Windows 11, jeu sans bordure sur l'écran principal, trois écrans, 2560 × 1440 pour la première preuve. La position des éléments dépend de la fenêtre du jeu et de son DPI, avec détection des changements. Le français et l'anglais sont pris en charge. MSIX étant accepté, Windows OCR reste une option à comparer avant le choix définitif.
