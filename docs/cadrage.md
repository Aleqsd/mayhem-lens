# Cadrage produit

Date de référence : 4 octobre 2026. Les propositions ci-dessous restent à confirmer par les dix réponses de cadrage.

## Besoin exprimé

- Une application Windows très rapide et légère.
- Un overlay simple pendant une partie de LoL ARAM Mayhem.
- Des tiers d'augmentations adaptés au champion joué.
- Des données récupérées en une fois, puis réutilisées localement pendant le choix.
- Un développement effectué avec des agents Codex.

## Parcours proposé

1. L'application repère une partie prise en charge et identifie le champion via l'API locale LoL.
2. Elle charge un snapshot Mayhem compatible avec le patch, ou utilise un cache dont l'âge et la provenance sont connus.
3. À l'apparition des cartes, elle reconnaît les noms d'augmentations dans une petite région de la fenêtre du jeu.
4. Elle associe les noms aux identifiants Mayhem et cherche leurs tiers pour ce champion.
5. Elle affiche les badges en laissant les clics et le focus au jeu.
6. Un reroll ou un nouveau choix invalide les badges précédents ; une lecture incertaine n'affiche pas une recommandation devinée.

## Périmètre proposé pour la v1

Tiers des trois cartes, détection du champion, cache par patch et source, reconnaissance locale, raccourci de secours et accès aux réglages depuis la barre système. Cible initiale proposée : jeu en fenêtre ou sans bordure.

La langue du jeu, la distribution, le déclenchement automatique, le détail de l'affichage et la prise en compte du stade attendent les réponses utilisateur. Les conseils d'objets, builds, combos et collecte de parties ne font pas encore partie d'un périmètre validé.

## Critères de réussite

- Données explicitement Mayhem, avec provenance, patch et date traçables.
- Classement spécifique au champion ; le stade n'est utilisé que s'il est disponible et identifié.
- Aucun téléchargement nécessaire entre la reconnaissance des cartes et l'affichage de leurs tiers.
- Aucun clic intercepté ou changement de focus au moment du choix.
- Badges retirés lorsqu'ils ne correspondent plus aux cartes présentes.
- Mesures de latence, CPU, mémoire et précision sur la configuration cible avant toute promesse de performance.

## Inconnues à résoudre

La reconnaissance des titres français, les différentes résolutions et mises à l'échelle, le suivi des rerolls, l'identification fiable du stade et le meilleur moteur OCR requièrent un prototype sur des captures réelles. Les performances du langage ne suffisent pas à établir celles de la chaîne capture/OCR.

Les statistiques tierces présentent des limites de méthode et de licence. Un classement fournisseur doit conserver sa provenance ; les règles personnelles de synergie doivent être distinguées des résultats statistiques.

## Règles de plateforme

Architecture envisagée : API locale en lecture et capture d'écran locale. L'absence d'injection ne constitue pas une certification Riot ou une garantie de compatibilité anti-triche.

La [politique Riot](https://developer.riotgames.com/docs/lol#game-policy) interdit notamment l'affichage des win rates d'augmentations ; un affichage par lettres ne prouve pas à lui seul la conformité du produit. Le comportement final et les obligations d'enregistrement doivent être revérifiés avant diffusion. Les pourcentages de victoire ne sont pas prévus dans l'overlay.
