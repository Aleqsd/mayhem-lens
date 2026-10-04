# Sources et qualité des données

Recherche vérifiée dans la session du 4 octobre 2026. Ces observations constituent un état daté ; les contrats, accès et données peuvent évoluer.

## Choix proposé

**ARAMKit** est la source principale candidate. **ARAMGG** est une alternative dotée d'une API documentée. **CommunityDragon** fournit les identifiants, noms français, raretés et ressources du catalogue ; il ne fournit pas de statistiques de performance.

| Source | Apport vérifié | Limite |
| --- | --- | --- |
| [ARAMKit](https://aramkit.com/en-US) | Tiers champion × augmentation, samples, patch et splits par stade dans les JSON inspectés. | Méthode de calcul des tiers et corpus non reproductibles ; pas de licence ouverte établie. |
| [ARAMGG API](https://data.dtodo.cn/api/v1/en/docs/cf-data-api.md) | Accès documenté avec clé, routes par champion et quotas ; cache explicitement prévu. | Grades fournisseur et statistiques clients distincts ; population annoncée ambiguë ; redistribution non établie. |
| [CommunityDragon](https://raw.communitydragon.org/16.19/plugins/rcp-be-lol-game-data/global/fr_fr/v1/augment-lists.json) | Pool KIWI/Mayhem et correspondance avec le catalogue traduit. | Catalogue statique uniquement ; le catalogue partagé comprend plusieurs modes. |
| [MayhemStats](https://mayhemstats.com/about/) | Collecteur et agrégations communautaires inspectables. | Trop peu de données sur le patch courant observé pour la source principale. |
| [iesdev lié à l'infrastructure Blitz](https://data.v2.iesdev.com/api/v1/query_objects/prod/lol/aram_mayhem_champion?champion_id=222) | Grades par champion accessibles dans le snapshot inspecté. | Effectifs, méthode, contrat d'API et licence non vérifiés. |

## Preuves ARAMKit

Le [manifest](https://data.aramkit.com/data/versions.json) inspecté annonçait la version `16.19`, un dataset daté du 29 septembre 2026 et 28 837 349 parties globales. Ce chiffre est une déclaration fournisseur, pas un corpus indépendamment audité.

Les JSON Jinx (222), Brand (63) et Tahm Kench (223) ont été téléchargés et analysés : 351 lignes champion × augmentation au total, dont les identifiants correspondaient tous au pool KIWI/Mayhem de CommunityDragon 16.19. Les tiers spécifiques au champion et les tiers globaux sont des champs différents ; les splits par stade peuvent encore différer.

Exemple de [snapshot Jinx inspecté](https://data.aramkit.com/data/16.19-20260929-7ed2deef470c/stats/all/champion-details/222.json). Le segment `stats/all` représente les brackets de joueurs ; il ne signifie pas « tous les modes ».

Ces trois champions ne constituent pas une validation exhaustive du dataset. La fraîcheur observée ne prouve pas une mise à jour quotidienne garantie. L'ordre des tiers n'est pas simplement celui des win rates.

Les [conditions ARAMKit](https://aramkit.com/en-US/terms) n'établissent pas une licence de redistribution des données et encadrent l'automatisation. Un accès JSON public ne suffit pas à autoriser un import massif, un miroir ou un bundle redistribué.

## Objets et builds Mayhem vérifiés

Les trois JSON ARAMKit inspectés contiennent `items.{filtered,unfiltered}.all`, des classements par emplacement dans `slots`, puis des archétypes, profils et routes dans `builds`. Les routes comprennent un `purchaseOrder`, les objets de départ, bottes et options suivantes. Les [pages champion](https://aramkit.com/en-US/champions/jinx) présentent ces données comme des builds Mayhem.

Des champs `augmentSpecific` apportent des associations objet × augmentation, avec effectif et `synergyDelta` dans certains objets. Ils ne prouvent pas un combo optimal de plusieurs augmentations ni un bénéfice causal.

Une route observée de trois objets et des recommandations conditionnelles de fin de build ne sont pas un unique build de six objets dont le résultat aurait été mesuré. Les taux de sélection des archétypes, profils et routes ont des dénominateurs différents. L'option `filtered` retire les éléments liés aux augmentations de la présentation ; elle ne prouve pas une nouvelle population statistique de parties filtrées.

Le cache par champion peut donc fournir les tiers d'augmentations et les conseils de builds sans importer les builds ARAM classiques. Les droits de partage restent à clarifier pour une distribution à des amis.

## API ARAMGG

La documentation inspectée indique une clé gratuite avec 200 crédits par jour et 60 requêtes par minute. Le config public annonçait `16.19.3`, généré le 2 octobre 2026. L'accès à une route de données sans clé a retourné 401 ; aucune clé n'a été créée dans cette recherche.

Les grades de recommandation et les samples anonymisés de clients ont des provenances différentes. Conserver cette distinction et vérifier la population avant comparaison. L'annonce WORLD dans la documentation et les mentions CN sur le site ne permettent pas d'affirmer une population mondiale homogène.

## Sources qui ne suffisent pas

- [Mobalytics Mayhem](https://mobalytics.gg/lol/champions/jinx/mayhem-builds) indique combiner des données ARAM avec des ratings d'augmentations : cela ne remplace pas des statistiques de parties Mayhem.
- Les tiers Arena et les builds ARAM classiques ne sont pas des substituts acceptables.
- Un site affichant « Mayhem » sans méthode ou provenance vérifiable reste une source à qualifier.
- Les snapshots de plusieurs fournisseurs ne doivent pas être mélangés comme s'ils mesuraient la même population et la même date.

## API Riot et contexte local

Les matchs Mayhem ne sont pas une base de collecte garantie via Match-v5 public. Le [ticket developer-relations 1109](https://github.com/RiotGames/developer-relations/issues/1109#issuecomment-3671793822) indique que leur caractère privé est attendu.

La [Live Client Data API](https://developer.riotgames.com/docs/lol#game-client-api_live-client-data-api) peut identifier le champion pendant une partie. Aucun champ documenté pour les trois offres d'augmentations n'a été établi dans la recherche : la capture/OCR est donc la piste envisagée. Les routes LCU éventuelles restent une intégration à qualifier, sans contrat de stabilité présumé.

## Invariants de normalisation

- Filtrer le pool du mode KIWI/Mayhem, sans se fier uniquement au préfixe du nom.
- Séparer catalogue, grades, effectifs et statistiques ; préserver les champs source.
- Ne pas remplacer un tier champion × augmentation par le tier global du même augment.
- Ajouter un stade seulement s'il existe dans la source et est identifié dans la partie.
- Absence ou `null` signifie inconnue, pas zéro ni dernier tier.
- Distinguer date du dataset, date d'import et patch du jeu ; gérer explicitement un cache périmé.
- Garder les credentials, dumps et caches hors de Git ; aucun dataset tiers n'est embarqué dans ce dépôt.
- Clarifier les droits d'utilisation et de distribution avant un produit partagé ou un import complet.
