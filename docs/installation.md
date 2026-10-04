# Installation Windows

La version 1.2.1 est expérimentale. Le parcours recommandé utilise `MayhemLens-Setup-1.2.1.exe`, un installateur natif avec une interface sombre qui contient le MSIX et son certificat public. L'installation manuelle du MSIX reste disponible. Les scripts de packaging ne lancent aucun installateur, n'approuvent aucun certificat et ne démarrent pas l'overlay ; la signature de développement est une étape séparée.

Le premier essai ARAM Mayhem de la 1.2.0 a chargé les données du champion, mais n'a affiché aucun badge et a provoqué un clignotement des bords signalé par l'utilisateur. L'application a été arrêtée. La 1.2.1 remplace la capture WGC par DXGI Desktop Duplication ; elle reste à tester en partie. Aucun correctif n'a été installé ou lancé pendant cette partie ; voir [le compte rendu et les vérifications restantes](validation.md#premier-essai-réel-120--4-octobre-2026).

## Installer avec le Setup

1. Depuis la [dernière release GitHub](https://github.com/Aleqsd/mayhem-lens/releases/latest), télécharger `MayhemLens-Setup-1.2.1.exe`, puis l'ouvrir avec le compte Windows qui utilisera Mayhem Lens.
2. Lire l'identité du certificat et accepter explicitement sa confiance pour cette installation. Si le certificat de développement n'est pas encore approuvé, seul le helper chargé de l'ajouter dans `LocalMachine\TrustedPeople` demande une élévation UAC. Le MSIX est installé pour l'utilisateur courant.
3. L'installateur indique le résultat de l'installation et les éventuelles langues OCR manquantes. Il n'installe pas silencieusement de fonctionnalités Windows ; choisir une langue OCR déjà installée ou ajouter la fonctionnalité souhaitée dans Windows.
4. Après réussite, cliquer sur **Lancer Mayhem Lens** pour ouvrir l'application, ou la retrouver ensuite dans le menu Démarrer. L'installation ne lance pas automatiquement l'overlay.

Mayhem Lens fonctionne dans la zone de notification près de l'horloge. Fermer la fenêtre de réglages laisse l'application active en arrière-plan ; le menu de l'icône permet de la rouvrir. **Quitter** dans ce menu arrête complètement l'application. Fermer l'installateur après installation ne ferme pas Mayhem Lens.

Les écrans de consentement et de sécurité Windows, dont UAC, conservent leur apparence standard. Le certificat de développement est auto-signé ; cette distribution reste expérimentale. La confiance du certificat, le parcours de l'installateur, l'OCR et le rendu réel restent à valider sur un PC avec l'utilisateur disponible.

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
./scripts/sign-development.ps1 -PackagePath 'dist\MayhemLens_1.2.1.0_x64.msix' `
  -SigningDirectory 'dist\private-signing' -PublicCertificatePath 'dist\MayhemLens-Development.cer'
```

Le Windows SDK contient `SignTool.exe`. Exemple avec un certificat déjà présent et utilisable dans le magasin de l'utilisateur :

```powershell
& '<Windows SDK>\x64\signtool.exe' sign /fd SHA256 /sha1 '<empreinte du certificat>' 'dist\MayhemLens_1.2.1.0_x64.msix'
```

Ne pas committer ou partager la clé privée/PFX. Le fichier `.cer` distribué contient uniquement la clé publique. Un certificat de développement doit être approuvé dans le magasin de l'ordinateur `Trusted People` sur chaque PC de test ; cette étape demande les droits administrateur. [Documentation Microsoft sur les certificats de test](https://learn.microsoft.com/en-us/windows/uwp/packaging/create-certificate-package-signing).

## Construire le Setup pour une release

Après création puis signature du MSIX, intégrer ce package et le certificat public :

```powershell
./scripts/package-setup.ps1 -PublicCertificatePath 'dist\MayhemLens-Development.cer'
./scripts/sign-development.ps1 -PackagePath 'dist\MayhemLens-Setup-1.2.1.exe' `
  -SigningDirectory 'dist\private-signing' -PublicCertificatePath 'dist\MayhemLens-Development.cer'
```

`package-setup.ps1` accepte aussi `-PackagePath` et `-OutputDirectory`. Il vérifie la version Cargo, le manifeste effectivement présent dans le MSIX, l'identité x64, le certificat public seul et les hashes SHA-256. Le MSIX est borné à 128 Mio et le certificat à 64 Kio. Il compile `mayhem-lens-setup`, restaure les variables d'environnement de compilation et copie l'EXE versionné dans le dossier de sortie. Son reçu décrit les payloads et le binaire ; il ne prouve ni signature, ni confiance, ni installation.

Pour une release publique, signer le MSIX **avant** de l'intégrer, puis signer également le Setup avec le même certificat de développement. La CI construit seulement un MSIX et un Setup **non signés**, en utilisant un certificat de test public créé en mémoire, sans import dans un magasin. Ses artefacts `UNSIGNED` ne sont pas les installateurs à distribuer.

## Installation manuelle du MSIX

1. Ouvrir la [dernière release GitHub](https://github.com/Aleqsd/mayhem-lens/releases/latest) et télécharger `MayhemLens-Development.cer` ainsi que `MayhemLens_1.2.1.0_x64.msix` pour la version 1.2.1.
2. Approuver le certificat public dans le magasin de l'ordinateur **Trusted People** (`LocalMachine\TrustedPeople`). Cette étape demande les droits administrateur. Depuis PowerShell ouvert en administrateur, dans le dossier du certificat :

   ```powershell
   Import-Certificate -FilePath '.\MayhemLens-Development.cer' -CertStoreLocation 'Cert:\LocalMachine\TrustedPeople'
   ```

3. Ouvrir le fichier `.msix` téléchargé et choisir **Installer** dans Windows.
4. Ouvrir **Mayhem Lens** depuis le menu Démarrer. Installer auparavant la fonctionnalité OCR de la langue configurée, comme indiqué ci-dessous.

À partir de la version 1.0.1, le MSIX direct est le parcours prévu ; aucune association à un fichier `.appinstaller` n'est requise. La [signature MSIX et sa confiance sur le PC](https://learn.microsoft.com/en-us/windows/msix/package/signing-package-overview) restent vérifiées par Windows.

Si la version 1.0.0 est déjà installée, utiliser le Setup ou ouvrir manuellement le MSIX 1.2.1 pour adopter le mécanisme de mises à jour corrigé. Les versions 1.0.1 ou ultérieures disposent déjà du téléchargement natif ; le certificat reste identique et n'a pas à être approuvé à nouveau s'il est déjà installé.

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

La capture DXGI est initialisée seulement lorsque le jeu pris en charge est au premier plan. Elle utilise l'adaptateur de l'écran contenant entièrement la fenêtre LoL, conserve une texture limitée au jeu sur le GPU et transfère uniquement la région OCR en mémoire CPU. Les badges de l'application sont exclus de l'image source ; aucune demande de capture WGC sans cadre n'est utilisée. La première cible reste sans bordure en SDR ; un écran pivoté ou une fenêtre répartie entre plusieurs écrans est refusé explicitement. Les contenus HDR ne sont pas convertis dans ce prototype.

La reconnaissance découvre une zone centrale large, apprend la géométrie des titres après deux retours cohérents et recommence après une lecture manquée ou un changement de fenêtre. Les noms longs sont regroupés conservativement. Une image immobile sur une duplication saine peut être relue ou recadrée sans inventer une nouvelle présentation desktop ; les caches sont invalidés lorsque le contexte du jeu ou la source change. Les formats de cartes et d'en-têtes réels FR/EN restent à valider avant d'annoncer une précision ou une latence.

## Réglages accessibles

Ouvrir **Réglages** depuis l'icône système. La fenêtre apparaît uniquement à cette demande et présente trois pages : **Général**, **Raccourcis** et **Mises à jour**. Langue, intervalle de scan, seuil de similarité, builds, stade, taille, opacité et décalages sont modifiables sans éditer le JSON. Les raccourcis peuvent être remplacés ; un conflit reste signalé dans le menu.

**Enregistrer** valide et applique les préférences ; **Annuler** ou Échap ne les enregistrent pas. Les valeurs par défaut restent un brouillon jusqu'à l'enregistrement. La saisie du stade prime sur l'automatique ; sans ordinal explicite reconnu dans l'en-tête, le mode automatique garde le tier champion et affiche que le choix est inconnu. Une lecture approchée exige deux observations stables au-dessus du seuil ; en dessous, les badges indiquent une lecture incertaine sans tier ni build, avec le raccourci de relecture configuré.

Une langue OCR manquante laisse le menu et les réglages accessibles. Le panneau indique son indisponibilité ; sélectionner une autre langue installée, ou installer la fonctionnalité OCR souhaitée dans Windows. La reconnaissance reste indisponible tant que cette condition n'est pas corrigée.

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
