# Installation Windows

La version 1.0.0 est un prototype technique. Le script de packaging produit un MSIX non signé ; la signature de développement est une étape séparée. Ces scripts n'installent rien, ne modifient pas le magasin de certificats et ne lancent pas l'overlay.

## Préparer

Le package cible Windows 11 22H2 ou plus récent (build 22621). Pour le construire, utiliser Rust MSVC et un Windows SDK contenant `MakeAppx.exe`. Le minimum Windows du manifeste garantit la disponibilité de l'API retenue pour les mises à jour différées. La cible initiale de validation reste le jeu sans bordure et SDR ; le plein écran exclusif et HDR ne sont pas validés.

```powershell
./scripts/package.ps1
```

Le package contient l'exécutable x64, le manifeste, les logos géométriques propres au projet et les licences des dépendances. Les datasets et ressources Riot ne sont pas incorporés. Le manifeste déclare une application desktop `runFullTrust`, avec accès Internet pour le chargement initial et un alias de commande.

## Signer avant installation

Un MSIX doit être signé par un certificat dont le sujet correspond exactement au `Publisher` du manifeste : `CN=Alexandre DO-O ALMEIDA`. Utiliser un certificat de signature adapté ; pour un test privé, un certificat de développement devra être explicitement approuvé sur chaque machine. Une signature de développement ne produit pas une réputation SmartScreen ni une certification Riot.

`scripts/sign-development.ps1` peut préparer cette signature sans installer l'application ni modifier les magasins de certificats. Il crée une clé de développement sauvegardée chiffrée par DPAPI pour l'utilisateur Windows courant, signe le package et exporte uniquement le certificat public à partager. Le PFX temporaire est supprimé. Une signature ne vaut pas approbation du certificat sur la machine destinataire.

```powershell
./scripts/sign-development.ps1 -PackagePath 'dist\MayhemLens_1.0.0.0_x64.msix' `
  -SigningDirectory 'dist\private-signing' -PublicCertificatePath 'dist\MayhemLens-Development.cer'
```

Le Windows SDK contient `SignTool.exe`. Exemple avec un certificat déjà présent et utilisable dans le magasin de l'utilisateur :

```powershell
& '<Windows SDK>\x64\signtool.exe' sign /fd SHA256 /sha1 '<empreinte du certificat>' 'dist\MayhemLens_1.0.0.0_x64.msix'
```

Ne pas committer ou partager la clé privée/PFX. Le fichier `.cer` distribué contient uniquement la clé publique. Un certificat de développement doit être approuvé dans le magasin de l'ordinateur `Trusted People` sur chaque PC de test ; cette étape demande les droits administrateur. [Documentation Microsoft sur les certificats de test](https://learn.microsoft.com/en-us/windows/uwp/packaging/create-certificate-package-signing).

## Installer depuis une release

Les versions publiques sont disponibles dans les [releases GitHub](https://github.com/Aleqsd/mayhem-lens/releases). Télécharger le certificat public `MayhemLens-Development.cer`, établir sa confiance pour ce test, puis télécharger et ouvrir [MayhemLens.appinstaller](https://github.com/Aleqsd/mayhem-lens/releases/latest/download/MayhemLens.appinstaller). Windows installe le MSIX signé référencé et associe l'application à sa source de mises à jour.

Ouvrir le `.appinstaller` est le parcours prévu pour conserver cette association. Installer seulement le MSIX brut permet une installation manuelle, mais ne garantit pas les vérifications natives de mises à jour de cette application. Le lien direct fonctionne sans le protocole `ms-appinstaller:`, désactivé par défaut sur les PC grand public. [Vue d'ensemble Microsoft](https://learn.microsoft.com/en-us/windows/msix/app-installer/app-installer-file-overview).

Le code et les fichiers de release sont publics. Les clés privées, configurations personnelles, caches, captures et datasets tiers restent hors du dépôt et du package.

## Mises à jour

L'application vérifie les mises à jour en arrière-plan au lancement. Le menu de l'icône système propose aussi « Rechercher / préparer une mise à jour ». Une version disponible est préparée par le gestionnaire de packages Windows avec une inscription différée pendant que l'application est ouverte : elle doit prendre effet au prochain lancement. La vérification et la préparation ne demandent pas l'arrêt forcé de l'overlay ou du jeu.

Le fichier `.appinstaller` demande aussi une vérification Windows à chaque lancement. Le contrôle effectué par l'application complète cette configuration, notamment pour les différents chemins de démarrage. Hors ligne, sans association App Installer ou en cas d'erreur, l'application continue à fonctionner et ne présente pas cet échec comme une preuve qu'elle est à jour.

L'état est enregistré séparément dans `update-status.json`, dans le dossier de données décrit ci-dessous. Le libellé « Mise à jour préparée — prochain lancement » indique une préparation différée ; relancer normalement l'overlay permet de vérifier la version active. Deux commandes sont également disponibles depuis le package installé :

```powershell
# Vérification seule, sans préparer de déploiement.
mayhem-lens.exe update check | Out-String
# Vérification puis préparation par Windows, sans arrêt forcé.
mayhem-lens.exe update | Out-String
```

La fonctionnalité est implémentée, mais l'installation par `.appinstaller` et l'application effective d'une mise à jour différée n'ont pas encore été validées sur un package installé. Voir [le fonctionnement et les sources](mises-a-jour.md) et [le plan de validation](validation.md).

## OCR et démarrage

Installer la fonctionnalité OCR française et/ou anglaise dans les langues Windows, selon la langue configurée. `mayhem-lens.exe diagnose` vérifie l'identité du package, les langues et l'API locale sans lancer l'overlay. Un EXE nu rend un diagnostic d'identité manquante ; ce n'est pas une preuve que les OCR ne sont pas installés.

Lancer depuis l'application installée, ou l'alias d'exécution du package. `run` crée l'icône système et attend une partie dont le mode local est `KIWI`. Les données du MSIX sont dans `%LOCALAPPDATA%\Packages\<famille du package>\LocalState\MayhemLens` ; la famille commence par `Aleqsd.MayhemLens_` et reste stable entre versions. L'exécutable seul utilise `%LOCALAPPDATA%\MayhemLens`. Les erreurs sont conservées dans `last-error.txt` et l'état dans `runtime-status.json` dans ce dossier ; aucune réponse contenant les identités des joueurs n'est enregistrée.

Les titres sont lus dans une bande relative à la fenêtre du jeu. Les noms longs sont regroupés conservativement. La zone doit être calibrée sur des choix réels FR/EN avant d'annoncer une précision ou une latence. Les contenus HDR ne sont pas convertis dans ce prototype.

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
