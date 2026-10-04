# Installation Windows

La version 0.1.0 est un prototype technique. Le MSIX construit est volontairement non signé : aucune modification du magasin de certificats, aucune installation et aucun lancement de fenêtre ne sont exécutés par le script de préparation.

## Préparer

Windows 10 2004 ou plus récent, Rust MSVC et un Windows SDK contenant `MakeAppx.exe` sont requis pour construire le package. La cible initiale de validation reste Windows 11, sans bordure et SDR ; le plein écran exclusif et HDR ne sont pas validés.

```powershell
./scripts/package.ps1
```

Le package contient l'exécutable x64, le manifeste, les logos géométriques propres au projet et les licences des dépendances. Les datasets et ressources Riot ne sont pas incorporés. Le manifeste déclare une application desktop `runFullTrust`, avec accès Internet pour le chargement initial et un alias de commande.

## Signer avant installation

Un MSIX doit être signé par un certificat dont le sujet correspond exactement au `Publisher` du manifeste : `CN=Alexandre DO-O ALMEIDA`. Utiliser un certificat de signature adapté ; pour un test privé, un certificat de développement devra être explicitement approuvé sur chaque machine. Une signature de développement ne produit pas une réputation SmartScreen ni une certification Riot.

`scripts/sign-development.ps1` peut préparer cette signature sans installer l'application ni modifier les magasins de certificats. Il crée une clé de développement sauvegardée chiffrée par DPAPI pour l'utilisateur Windows courant, signe le package et exporte uniquement le certificat public à partager. Le PFX temporaire est supprimé. Une signature ne vaut pas approbation du certificat sur la machine destinataire.

```powershell
./scripts/sign-development.ps1 -PackagePath 'dist\MayhemLens_0.1.0.0_x64.msix' `
  -SigningDirectory 'dist\private-signing' -PublicCertificatePath 'dist\MayhemLens-Development.cer'
```

Le Windows SDK contient `SignTool.exe`. Exemple avec un certificat déjà présent et utilisable dans le magasin de l'utilisateur :

```powershell
& '<Windows SDK>\x64\signtool.exe' sign /fd SHA256 /sha1 '<empreinte du certificat>' 'dist\MayhemLens_0.1.0.0_x64.msix'
```

Ne pas committer ou partager la clé privée/PFX. Après signature et confiance établies, l'installation peut être effectuée avec l'interface Windows ou `Add-AppxPackage -Path <package signé>`. Cette étape n'est pas effectuée par le projet.

## OCR et démarrage

Installer la fonctionnalité OCR française et/ou anglaise dans les langues Windows, selon la langue configurée. `mayhem-lens.exe diagnose` vérifie l'identité du package, les langues et l'API locale sans lancer l'overlay. Un EXE nu rend un diagnostic d'identité manquante ; ce n'est pas une preuve que les OCR ne sont pas installés.

Lancer depuis l'application installée, ou l'alias d'exécution du package. `run` crée l'icône système et attend une partie dont le mode local est `KIWI`. Les erreurs sont conservées dans `%LOCALAPPDATA%\MayhemLens\last-error.txt` et l'état dans `runtime-status.json` ; aucune réponse contenant les identités des joueurs n'est enregistrée.

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
