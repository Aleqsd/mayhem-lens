# Mises à jour de Mayhem Lens

La version 1.0.0 utilise le gestionnaire de packages Windows pour préparer une nouvelle version au lancement ou après une vérification manuelle dans le menu de l'icône système. Aucun exécutable de mise à jour supplémentaire n'est distribué. L'inscription du package est différée tant que l'application est utilisée ; la nouvelle version doit prendre effet au prochain lancement.

Ce fonctionnement est implémenté. L'installation depuis la release, l'association App Installer et l'application effective d'une mise à jour sur un PC restent à valider. La compilation et les tests locaux ne constituent pas cette preuve.

## Canal de distribution

La source stable est :

`https://github.com/Aleqsd/mayhem-lens/releases/latest/download/MayhemLens.appinstaller`

Le fichier décrit le MSIX signé de la version publiée, avec une URL épinglée au tag de release. Par exemple, la version 1.0.0 référence `releases/download/v1.0.0/MayhemLens_1.0.0.0_x64.msix`. Une ancienne release reste disponible séparément ; elle ne remplace pas la source stable.

Les valeurs `Name`, `Publisher`, `Version` et `ProcessorArchitecture` du `MainPackage` correspondent exactement à l'identité du manifeste MSIX. App Installer vérifie cette correspondance lors du déploiement. La version du fichier `.appinstaller` et celle du package sont incrémentées lors d'une nouvelle publication. [Création d'un fichier App Installer](https://learn.microsoft.com/en-us/windows/msix/app-installer/how-to-create-appinstaller-file).

Le premier téléchargement à ouvrir est le `.appinstaller`, afin de créer l'association de mise à jour Windows. Un MSIX signé avec le certificat de développement exige que son certificat public ait été approuvé sur le PC de test. Les prochaines versions doivent conserver l'identité de package et utiliser une signature approuvée. La clé privée/PFX n'est jamais distribuée. [Certificats de signature de package](https://learn.microsoft.com/en-us/windows/uwp/packaging/create-certificate-package-signing).

Les fichiers sont accessibles en HTTPS sans authentification GitHub. Avant de considérer la distribution fonctionnelle, vérifier les réponses de l'hébergement : `.appinstaller` en `application/appinstaller`, `.msix` en `application/msix`, longueur de contenu et prise en charge des requêtes par plages. [Installation depuis le web](https://learn.microsoft.com/en-us/windows/msix/app-installer/installing-windows10-apps-web), [diagnostic de livraison MSIX](https://learn.microsoft.com/en-us/windows/msix/msix-troubleshooting-guide).

## Vérification et préparation natives

Le contrôle au démarrage s'exécute hors du thread d'affichage. La même logique sert au menu « Rechercher / préparer une mise à jour » (« Check / prepare update » en anglais). L'identité provient du package installé et la source doit appartenir au canal prévu. La commande `update check` effectue uniquement la lecture ; `update` vérifie puis prépare le déploiement.

La vérification utilise `Package.CheckUpdateAvailabilityAsync`. Cette API travaille avec l'association `.appinstaller` et ne suffit pas pour une installation depuis un MSIX brut. Microsoft documente aussi un échec « Access denied » lorsqu'elle est appelée directement sur `Package.Current` : le package à vérifier est donc récupéré par `PackageManager.FindPackageForUser("", Package.Current.Id.FullName)`. Un résultat inconnu ou une erreur ne prouve pas que l'application est à jour. [API de vérification](https://learn.microsoft.com/en-us/uwp/api/windows.applicationmodel.package.checkupdateavailabilityasync?view=winrt-26100).

Si l'association au canal stable manque, le chemin de préparation peut tenter de l'établir en traitant le `.appinstaller` stable avec Windows. La commande de vérification seule ne le fait pas. Cette tentative doit également être validée sur une installation issue d'un MSIX brut.

Pour préparer une version disponible, le code utilise `Windows.Management.Deployment.PackageManager.AddPackageByUriAsync` avec l'URI du `.appinstaller` et les options suivantes :

```text
DeferRegistrationWhenPackagesAreInUse = true
AllowUnsigned = false
ForceAppShutdown = false
ForceTargetAppShutdown = false
```

L'API accepte les fichiers `.appinstaller` depuis le build Windows 22556 ; le manifeste de Mayhem Lens exige le build stable 22621 ou plus récent, soit Windows 11 22H2 minimum. La propriété de report diffère l'inscription jusqu'à la prochaine activation lorsque le package est utilisé. Ce chemin conserve la source App Installer et demande à Windows de traiter le package signé, sans remplacer l'EXE de l'application à la main. [AddPackageByUriAsync](https://learn.microsoft.com/en-us/uwp/api/windows.management.deployment.packagemanager.addpackagebyuriasync?view=winrt-26100), [report de l'inscription](https://learn.microsoft.com/en-us/uwp/api/windows.management.deployment.addpackageoptions.deferregistrationwhenpackagesareinuse?view=winrt-26100).

Un téléchargement ou une opération de déploiement terminée n'implique pas que la version active a changé. `DeploymentResult.IsRegistered` indique si le package est enregistré et prêt ; une préparation différée doit être distinguée d'une mise à jour active. La version réellement exécutée est vérifiée au lancement suivant avec `Package.Id.Version`. [Résultat du déploiement](https://learn.microsoft.com/en-us/uwp/api/windows.management.deployment.deploymentresult?view=winrt-26100).

Le menu expose cet état et `update-status.json` conserve sa phase, la version installée connue et la date du contrôle. Pour le MSIX, ce fichier est dans `%LOCALAPPDATA%\Packages\<famille du package>\LocalState\MayhemLens`, avec une famille commençant par `Aleqsd.MayhemLens_`, stable entre versions. L'exécutable seul utilise `%LOCALAPPDATA%\MayhemLens`. `ready_on_restart` signifie « Mise à jour préparée — prochain lancement » ; `registered` indique un package enregistré et demande de relancer l'overlay. `unassociated`, `unsupported` et `error` ne sont jamais présentés comme « à jour ».

## Configuration App Installer et limites

Le fichier demande aussi un contrôle Windows au lancement avec `OnLaunch HoursBetweenUpdateChecks="0"`. Zéro signifie une vérification à chaque lancement ; le défaut sans cet attribut est 24 heures. Le contrôle natif de l'application complète cette politique Windows. Les mises à jour n'imposent pas un arrêt forcé de l'application. [Paramètres de mise à jour](https://learn.microsoft.com/en-us/windows/msix/app-installer/update-settings).

Il ne faut pas dépendre de `ShowPrompt` ou `UpdateBlocksActivation` pour l'expérience de Mayhem Lens. Microsoft décrit une mise à jour silencieuse pour les applications desktop empaquetées et limite ces attributs aux activations par menu Démarrer, alias ou protocole ; ils n'agissent pas depuis un raccourci de bureau ou la barre des tâches. [Schéma OnLaunch](https://learn.microsoft.com/en-us/uwp/schemas/appinstallerschema/element-onlaunch).

Le lien d'installation des releases est un téléchargement HTTPS direct du `.appinstaller`, pas une URI `ms-appinstaller:?source=...`. Ce protocole est désactivé par défaut sur les PC grand public depuis décembre 2023. [État des fonctions de distribution Windows](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/distribution-feature-status).

Une absence de réseau, d'association ou de source valide laisse l'application fonctionner avec sa version actuelle. La confiance du certificat et les politiques de l'ordinateur peuvent empêcher l'installation. Les cas de succès, de report et d'échec natifs figurent dans le [plan de validation](validation.md).
