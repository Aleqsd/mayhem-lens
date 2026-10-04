# Architecture proposée

Statut : proposition issue de la recherche, à confirmer après cadrage et prototype.

## Socle

Rust est retenu comme point de départ pour son contrôle des ressources et ses [bindings Microsoft Windows](https://github.com/microsoft/windows-rs). Le choix ne suppose aucune supériorité mesurée sur un programme C++ équivalent.

Un seul processus coordonnerait une boucle de messages Windows et des tâches de données, capture et reconnaissance hors du thread d'affichage. Le runtime, les dépendances et les threads réellement nécessaires seront choisis lors de l'implémentation.

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

Ces responsabilités ne prescrivent pas autant de crates. Garder une organisation proportionnée à un petit produit.

## Affichage Windows

Des petites fenêtres `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`, placées au-dessus du jeu sans activation, constituent la piste initiale. Microsoft documente le passage des clics pour une [fenêtre layered transparente](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features#layered-windows).

Direct2D/DirectWrite peut produire une petite bitmap alpha, transmise par `UpdateLayeredWindow` seulement lorsque le contenu change. Microsoft recommande de garder les [fenêtres layered petites pour la performance](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-updatelayeredwindow). DirectComposition pourra être évalué si des besoins de rendu ou des mesures le justifient.

Le plein écran exclusif n'est pas une cible garantie. Les [optimisations plein écran Windows](https://devblogs.microsoft.com/directx/demystifying-full-screen-optimizations/) peuvent modifier le comportement ; la v1 sera validée dans le mode réellement utilisé.

## Capture et reconnaissance

Piste : `Windows.Graphics.Capture` avec [CreateForWindow](https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow), disponible depuis Windows 10 1903. Les frames arrivent sur un worker et les régions des titres sont recadrées avant reconnaissance.

Une détection légère surveille l'apparition ou le changement des cartes. L'OCR n'est pas lancé sur chaque frame. Conserver le moteur chaud, éviter les travaux simultanés obsolètes et ne publier que le résultat correspondant aux dernières cartes détectées.

Windows OCR et un moteur embarqué seront comparés sur les mêmes captures. [Windows.Media.Ocr](https://learn.microsoft.com/en-us/uwp/api/windows.media.ocr) exige officiellement une identité de package pour une application desktop ; les langues OCR installées doivent aussi être vérifiées. Un programme portable doit employer une autre voie supportée, par exemple un moteur OCR livré avec ses ressources.

## Cache et erreurs

Clé de données minimale : source, patch, champion, augmentation ; stade optionnel. Inclure la date du dataset, la date d'import et la population annoncée.

Les données du champion sont téléchargées au début de la partie, puis conservées en cache. Le réseau reste hors du parcours de choix. En cas d'échec, utiliser un cache compatible selon la politique retenue, ou indiquer l'indisponibilité. Un résultat absent ou incertain n'est jamais converti arbitrairement en mauvais tier.

Les captures sont traitées localement. Télémétrie, collecte de parties et conservation de screenshots ne font pas partie du périmètre demandé.

## Configuration retenue pour le prototype

Windows 11, jeu sans bordure sur l'écran principal, trois écrans, 2560 × 1440 pour la première preuve. La position des éléments dépend de la fenêtre du jeu et de son DPI, avec détection des changements. Le français et l'anglais sont pris en charge. MSIX étant accepté, Windows OCR reste une option à comparer avant le choix définitif.
