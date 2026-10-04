# Installation Windows

La version 1.1.0 est expérimentale. L'installation se fait en téléchargeant et ouvrant le MSIX signé. Le script de packaging produit un MSIX non signé ; la signature de développement est une étape séparée. Ces scripts n'installent rien, ne modifient pas le magasin de certificats et ne lancent pas l'overlay.

## Préparer

Le package cible Windows 11 22H2 ou plus récent (build 22621). Pour le construire, utiliser Rust MSVC et un Windows SDK contenant `MakeAppx.exe`. La cible initiale de validation reste le jeu sans bordure et SDR ; le plein écran exclusif et HDR ne sont pas validés.

```powershell
./scripts/package.ps1
```

Le package contient l'exécutable x64, le manifeste, les logos géométriques propres au projet et les licences des dépendances. Les datasets et ressources Riot ne sont pas incorporés. Le manifeste déclare une application desktop `runFullTrust`, avec accès Internet pour le chargement initial et un alias de commande.

## Signer avant installation

Un MSIX doit être signé par un certificat dont le sujet correspond exactement au `Publisher` du manifeste : `CN=Alexandre DO-O ALMEIDA`. Utiliser un certificat de signature adapté ; pour un test privé, un certificat de développement devra être explicitement approuvé sur chaque machine. Une signature de développement ne produit pas une réputation SmartScreen ni une certification Riot.

`scripts/sign-development.ps1` peut préparer cette signature sans installer l'application ni modifier les magasins de certificats. Il crée une clé de développement sauvegardée chiffrée par DPAPI pour l'utilisateur Windows courant, signe le package et exporte uniquement le certificat public à partager. Le PFX temporaire est supprimé. Une signature ne vaut pas approbation du certificat sur la machine destinataire.

```powershell
./scripts/sign-development.ps1 -PackagePath 'dist\MayhemLens_1.1.0.0_x64.msix' `
  -SigningDirectory 'dist\private-signing' -PublicCertificatePath 'dist\MayhemLens-Development.cer'
```

Le Windows SDK contient `SignTool.exe`. Exemple avec un certificat déjà présent et utilisable dans le magasin de l'utilisateur :

```powershell
& '<Windows SDK>\x64\signtool.exe' sign /fd SHA256 /sha1 '<empreinte du certificat>' 'dist\MayhemLens_1.1.0.0_x64.msix'
```

Ne pas committer ou partager la clé privée/PFX. Le fichier `.cer` distribué contient uniquement la clé publique. Un certificat de développement doit être approuvé dans le magasin de l'ordinateur `Trusted People` sur chaque PC de test ; cette étape demande les droits administrateur. [Documentation Microsoft sur les certificats de test](https://learn.microsoft.com/en-us/windows/uwp/packaging/create-certificate-package-signing).

## Installer depuis une release

1. Ouvrir la [dernière release GitHub](https://github.com/Aleqsd/mayhem-lens/releases/latest) et télécharger `MayhemLens-Development.cer` ainsi que `MayhemLens_1.1.0.0_x64.msix` pour la version 1.1.0.
2. Approuver le certificat public dans le magasin de l'ordinateur **Trusted People** (`LocalMachine\TrustedPeople`). Cette étape demande les droits administrateur. Depuis PowerShell ouvert en administrateur, dans le dossier du certificat :

   ```powershell
   Import-Certificate -FilePath '.\MayhemLens-Development.cer' -CertStoreLocation 'Cert:\LocalMachine\TrustedPeople'
   ```

3. Ouvrir le fichier `.msix` téléchargé et choisir **Installer** dans Windows.
4. Ouvrir **Mayhem Lens** depuis le menu Démarrer. Installer auparavant la fonctionnalité OCR de la langue configurée, comme indiqué ci-dessous.

À partir de la version 1.0.1, le MSIX direct est le parcours prévu ; aucune association à un fichier `.appinstaller` n'est requise. La [signature MSIX et sa confiance sur le PC](https://learn.microsoft.com/en-us/windows/msix/package/signing-package-overview) restent vérifiées par Windows.

Si la version 1.0.0 est déjà installée, ouvrir manuellement le MSIX 1.1.0 pour adopter ce mécanisme de mises à jour. Les versions 1.0.1 ou ultérieures disposent déjà du téléchargement natif ; le certificat reste identique et n'a pas à être approuvé à nouveau s'il est déjà installé.

Le code et les fichiers de release sont publics. Les clés privées, configurations personnelles, caches, captures et datasets tiers restent hors du dépôt et du package.

## Mises à jour

L'application consulte la dernière release stable GitHub en arrière-plan au lancement. Le menu de l'icône système propose aussi « Rechercher / préparer une mise à jour ». Pour une version supérieure, elle télécharge le MSIX, vérifie sa taille, son digest SHA-256 et son identité de package, puis demande à Windows de vérifier sa signature et de préparer son inscription différée. La mise à jour doit prendre effet au prochain lancement.

La préparation ne demande pas l'arrêt forcé de l'overlay ou du jeu, et aucun EXE n'est remplacé manuellement. Hors ligne, si GitHub est indisponible ou si un contrôle échoue, l'application continue avec sa version actuelle et ne présente pas cet échec comme une preuve qu'elle est à jour. Le certificat de développement doit rester approuvé pour que Windows accepte les versions suivantes.

La fenêtre **Réglages → Mises à jour** affiche les versions active et cible, le téléchargement et la préparation. **Vérifier** consulte seulement les métadonnées ; **Préparer la mise à jour** télécharge et demande sa préparation à Windows. L'état est enregistré dans `update-status.json`, et `update-pending.json` conserve la cible attendue après une préparation acceptée. Le libellé « Mise à jour préparée — prochain lancement » indique une préparation différée ; « Mise à jour appliquée » nécessite une version active identique à la cible après relance. Deux commandes sont également disponibles depuis le package installé :

```powershell
# Vérification seule, sans préparer de déploiement.
mayhem-lens.exe update check | Out-String
# Vérification puis préparation par Windows, sans arrêt forcé.
mayhem-lens.exe update | Out-String
```

L'installation du MSIX publié sur un autre PC et l'application effective d'une mise à jour différée restent à valider sur un package installé. Les tests unitaires et la création du package ne constituent pas cette preuve. Voir [le fonctionnement et les sources](mises-a-jour.md) et [le plan de validation](validation.md).

## OCR et démarrage

Installer la fonctionnalité OCR française et/ou anglaise dans les langues Windows, selon la langue configurée. `mayhem-lens.exe diagnose` vérifie l'identité du package, les langues et l'API locale sans lancer l'overlay. Un EXE nu rend un diagnostic d'identité manquante ; ce n'est pas une preuve que les OCR ne sont pas installés.

Lancer depuis l'application installée, ou l'alias d'exécution du package. `run` crée l'icône système et attend une partie dont le mode local est `KIWI`. Les données du MSIX sont dans `%LOCALAPPDATA%\Packages\<famille du package>\LocalState\MayhemLens` ; la famille commence par `Aleqsd.MayhemLens_` et reste stable entre versions. L'exécutable seul utilise `%LOCALAPPDATA%\MayhemLens`. Les erreurs sont conservées dans `last-error.txt` et l'état dans `runtime-status.json` dans ce dossier ; aucune réponse contenant les identités des joueurs n'est enregistrée.

La capture découvre une zone centrale large, apprend la géométrie des titres après deux retours cohérents et recommence après une lecture manquée ou un changement de fenêtre. Les noms longs sont regroupés conservativement. Les formats de cartes et d'en-têtes réels FR/EN restent à valider avant d'annoncer une précision ou une latence. Les contenus HDR ne sont pas convertis dans ce prototype.

## Réglages accessibles

Ouvrir **Réglages** depuis l'icône système. La fenêtre apparaît uniquement à cette demande et présente trois pages : **Général**, **Raccourcis** et **Mises à jour**. Langue, intervalle de scan, seuil de similarité, builds, stade, taille, opacité et décalages sont modifiables sans éditer le JSON. Les raccourcis peuvent être remplacés ; un conflit reste signalé dans le menu.

**Enregistrer** valide et applique les préférences ; **Annuler** ou Échap ne les enregistrent pas. Les valeurs par défaut restent un brouillon jusqu'à l'enregistrement. La saisie du stade prime sur l'automatique ; sans ordinal explicite reconnu dans l'en-tête, le mode automatique garde le tier champion et affiche que le choix est inconnu. Une lecture approchée exige deux observations stables au-dessus du seuil ; en dessous, les badges indiquent une lecture incertaine sans tier ni build, avec le raccourci de relecture configuré.

L'exécutable x64 lie le runtime C statiquement pour éviter une installation séparée de Visual C++ Redistributable. Les commandes d'un exécutable GUI lancées directement depuis PowerShell peuvent nécessiter `| Out-String` pour capturer leur sortie et attendre leur fin.

Le patch de données est celui du manifeste fournisseur ; sa compatibilité avec le catalogue est vérifiée. L'API LiveClientData observée ne fournit pas la version du client : ce prototype n'établit pas encore une égalité entre cette version et le patch du fournisseur.

## Règles personnelles

`config.json` accepte `synergy_rules`, vide par défaut. Exemple **synthétique**, dont les IDs doivent être remplacés par des augmentations du catalogue Mayhem courant :

```json
{
  "synergy_rules": [{
    "id": "ma-regle",
    "requires": [123],
    "offered": 456,
    "explanation_fr": "Explication personnelle de l'interaction.",
    "explanation_en": "Personal explanation of the interaction."
  }]
}
```

Ajouter ce champ aux réglages existants. Toutes les augmentations `requires` doivent avoir été confirmées pour afficher la règle. Le texte reçoit le libellé « Règle personnelle » et ne modifie jamais le tier fournisseur. Le niveau n'est pas utilisé pour deviner le stade d'une offre retardée.
