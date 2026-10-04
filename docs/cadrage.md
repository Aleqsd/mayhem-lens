# Cadrage produit

Date de référence : 4 octobre 2026. Les dix réponses sont consignées dans `questions.md` ; les détails d'implémentation restent à préciser par le prototype.

## Décisions utilisateur reçues

- Public cible : l'utilisateur et quelques amis. Le code et l'installateur sont publics, décision confirmée le 4 octobre.
- V1 : tiers d'augmentations et conseils d'objets/builds.
- Affichage pendant le choix : une lettre et une courte explication.
- Déclenchement automatique avec raccourci de secours.
- Tiers fournisseur spécifiques au champion et au stade lorsque disponibles.
- Règles de synergie tenant compte des augmentations précédentes, explicitement distinctes des statistiques.
- Reconnaissance et interface en français et anglais.
- LoL sans bordure, sur l'écran principal d'une installation à trois écrans ; 1440p pour la première validation, autres résolutions à supporter, DPI détecté automatiquement.
- Installation Windows classique ; MSIX accepté.
- Données du champion téléchargées au début de la partie, puis consultées depuis le cache.
- Vérification des mises à jour au lancement et accès manuel simple depuis le menu système. Préparation native MSIX en arrière-plan, application après fermeture ; Windows 11 22H2 ou plus récent.
- Améliorations demandées le 4 octobre : calibrage automatique, indication de confiance et masquage d'un tier incertain, détection du stade quand il est identifiable, fenêtre de réglages et suivi explicite des mises à jour jusqu'à confirmation après relance.
- Installateur personnalisé demandé le 4 octobre : EXE natif sombre et doré, contenant le MSIX signé et le certificat public ; approbation explicite du certificat si nécessaire, progression et lancement facultatif depuis l'écran de fin. Les confirmations de sécurité Windows restent gérées par Windows.

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

## Périmètre v1 cadré

Tiers des trois cartes avec une courte explication, règles de synergie sur les augmentations précédentes et conseils d'objets/builds font partie de la v1. Les données d'objets et routes de builds Mayhem ont été vérifiées dans les trois snapshots ARAMKit inspectés ; leur normalisation doit conserver les limites documentées dans `donnees.md`.

Détection du champion, cache par patch et source, reconnaissance locale automatique, raccourci de secours et accès aux réglages depuis la barre système constituent les moyens proposés. La cible initiale est le jeu sans bordure ; les zones utiles sont positionnées relativement à sa fenêtre, pas à une taille d'écran fixée en pixels.

La présentation des conseils d'objets/builds reste à préciser lors de la conception. Séparer le tier fournisseur de l'indication de synergie ; les règles ne recalculent pas silencieusement une prétendue statistique. L'identification des augmentations précédentes et du stade doit rester traçable, avec possibilité de correction via les réglages si la reconnaissance n'est pas fiable.

La collecte de parties et la télémétrie ne font pas partie du périmètre demandé. L'utilisateur a choisi un dépôt public et des installateurs publics ; les données tierces restent téléchargées séparément.

Une explication doit reposer sur des faits ou une règle identifiable ; la formule d'un tier fournisseur ne permet pas d'inventer une justification causale.

## Critères de réussite

- Données explicitement Mayhem, avec provenance, patch et date traçables.
- Classement spécifique au champion ; le stade n'est utilisé que s'il est disponible et identifié.
- Conseils d'objets/builds issus de routes Mayhem, avec distinction entre ordre d'achat observé et options de fin de build.
- Synergies documentées comme règles explicatives ou associations observées ; les deux ne sont pas présentées comme des effets causaux établis.
- Aucun téléchargement nécessaire entre la reconnaissance des cartes et l'affichage de leurs tiers.
- Aucun clic intercepté ou changement de focus au moment du choix.
- Badges retirés lorsqu'ils ne correspondent plus aux cartes présentes.
- FR/EN, trois écrans, position relative à la fenêtre, changement de résolution et DPI pris en compte.
- Mesures de latence, CPU, mémoire et précision sur la configuration cible avant toute promesse de performance.

## Inconnues à résoudre

La reconnaissance des titres français, les différentes résolutions et mises à l'échelle, le suivi des rerolls, l'identification fiable du stade et le meilleur moteur OCR requièrent un prototype sur des captures réelles. Les performances du langage ne suffisent pas à établir celles de la chaîne capture/OCR.

Les statistiques tierces présentent des limites de méthode et de licence. Un classement fournisseur doit conserver sa provenance ; les règles personnelles de synergie doivent être distinguées des résultats statistiques.

## Règles de plateforme

Architecture envisagée : API locale en lecture et capture d'écran locale. L'absence d'injection ne constitue pas une certification Riot ou une garantie de compatibilité anti-triche.

La [politique Riot](https://developer.riotgames.com/docs/lol#game-policy) interdit notamment l'affichage des win rates d'augmentations ; un affichage par lettres ne prouve pas à lui seul la conformité du produit. Le comportement final et les obligations d'enregistrement doivent être revérifiés avant diffusion. Les pourcentages de victoire ne sont pas prévus dans l'overlay.
