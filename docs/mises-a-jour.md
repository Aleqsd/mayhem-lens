# Mises à jour de Mayhem Lens

La version expérimentale 1.1.1 consulte les releases GitHub au lancement et depuis le menu de l'icône système. Une version supérieure est téléchargée sous forme de MSIX local, contrôlée, puis confiée au gestionnaire de packages Windows. L'inscription est différée pendant que Mayhem Lens est utilisé ; la nouvelle version doit prendre effet au prochain lancement. L'installateur personnalisé sert à la première installation ; le canal de mise à jour continue d'utiliser le MSIX signé.

La confiance du certificat, l'installation depuis la release et l'application effective d'une mise à jour sur un PC restent à valider. La compilation, les tests unitaires et la création d'un MSIX ne constituent pas cette preuve.

## Source publique

La source est l'API publique :

`https://api.github.com/repos/Aleqsd/mayhem-lens/releases/latest`

L'application sélectionne la release stable et son MSIX x64 versionné, par exemple `MayhemLens_1.0.1.0_x64.msix`. Les brouillons et préversions sont exclus. Aucun compte ou token GitHub n'est nécessaire pour lire les releases publiques. L'API expose notamment `browser_download_url`, `size` et `digest` pour chaque asset. [API des releases GitHub](https://docs.github.com/en/rest/releases/releases#get-the-latest-release), [métadonnées des assets](https://docs.github.com/en/rest/releases/assets).

La version 1.0.0 utilisait un fichier `.appinstaller` distant. Lors de la vérification du 4 octobre 2026, GitHub le livrait en `application/octet-stream`, malgré le type correct dans les métadonnées de l'asset. La version 1.0.1 utilise le téléchargement du MSIX puis son chemin local, sans dépendre de ce type MIME ou d'une association App Installer.

L'installation initiale consiste à approuver le certificat public de développement dans `LocalMachine\TrustedPeople`, puis télécharger et ouvrir directement le MSIX signé. Les étapes figurent dans [Installation Windows](installation.md). Les versions suivantes conservent l'identité de package et une signature approuvée. La clé privée/PFX n'est jamais distribuée.

## Contrôles avant préparation

Le worker de mise à jour effectue les opérations suivantes, hors du thread d'affichage :

1. Lire la version du package installé et les métadonnées de la dernière release stable.
2. Identifier une version supérieure et l'asset MSIX attendu. Son URL doit désigner le dépôt et le tag exacts ; les redirections sont limitées aux CDN GitHub autorisés en HTTPS.
3. Télécharger le fichier localement, vérifier sa taille annoncée, bornée à 128 Mio, et calculer son SHA-256. Le digest GitHub `sha256:…` est obligatoire ; un digest absent ou un contenu différent bloque la préparation.
4. Lire l'identité MSIX avec les API natives Windows. Vérifier le nom `Aleqsd.MayhemLens`, le publisher `CN=Alexandre DO-O ALMEIDA`, la version cible et l'architecture x64.
5. Confier le fichier local validé à Windows, qui vérifie la signature et sa confiance avant d'accepter le déploiement.

Le SHA-256 vérifie que le fichier correspond à l'asset annoncé ; Windows décide de la confiance de sa signature. Un échec laisse la version actuellement installée utilisable et n'est jamais affiché comme « à jour ». [Signature et confiance MSIX](https://learn.microsoft.com/en-us/windows/msix/package/signing-package-overview).

La commande `update check` vérifie uniquement les métadonnées et la disponibilité. Elle ne télécharge pas le MSIX et ne demande pas de déploiement. La commande `update` et la vérification du menu peuvent préparer la version disponible.

## Déploiement différé

Le prototype cible Windows 11 22H2 ou plus récent (build 22621). La préparation utilise `Windows.Management.Deployment.PackageManager.AddPackageByUriAsync` avec une URI de fichier local et les options suivantes :

```text
DeferRegistrationWhenPackagesAreInUse = true
AllowUnsigned = false
ForceAppShutdown = false
ForceTargetAppShutdown = false
```

Windows traite le package signé et reporte son inscription jusqu'à une prochaine activation lorsqu'il est utilisé. L'application ne remplace pas son EXE manuellement, ne demande aucun arrêt forcé et ne redémarre pas le jeu. [API de déploiement](https://learn.microsoft.com/en-us/uwp/api/windows.management.deployment.packagemanager.addpackagebyuriasync?view=winrt-26100), [report de l'inscription](https://learn.microsoft.com/en-us/uwp/api/windows.management.deployment.addpackageoptions.deferregistrationwhenpackagesareinuse?view=winrt-26100).

Une opération terminée ne prouve pas que le processus actif a changé de version. Le résultat de déploiement distingue la préparation différée de l'inscription du package ; la version réellement exécutée est vérifiée au lancement suivant via `Package.Id.Version`. [Résultat de déploiement](https://learn.microsoft.com/en-us/uwp/api/windows.management.deployment.deploymentresult?view=winrt-26100).

Le menu et la fenêtre de réglages exposent l'état. `update-status.json` conserve la version active issue de l'identité du processus, la version cible, les octets téléchargés et les phases recherche, téléchargement, vérification et préparation. La progression est bornée et publiée au plus une fois par seconde pendant le transfert, avec une publication finale. Le panneau permet de rechercher les métadonnées seules puis de préparer la mise à jour.

Après acceptation du package par Windows, `update-pending.json` garde l'identité et la cible attendue. « Mise à jour préparée — prochain lancement » ne signifie pas que le processus actif a changé de version. La confirmation « Mise à jour appliquée » exige que la version observée via `Package.Current.Id.Version` soit exactement celle du reçu. Ce reçu est conservé après confirmation ; un échec réseau ultérieur ne supprime pas cette preuve. Un fichier téléchargé, un digest réussi ou `IsRegistered` seul ne produisent pas cette confirmation.

## Validation restante

Tester le certificat de développement sur un autre PC, le téléchargement depuis la release publique, le report pendant que l'application reste ouverte et la prise d'effet après fermeture et relance. Inclure les refus de digest, taille, identité, version et signature, ainsi que les erreurs réseau.

Les tests de parseurs et la lecture d'un manifeste MSIX ne prouvent pas que Windows acceptera une installation. Le [plan de validation](validation.md) sépare ces contrôles des essais natifs à effectuer avec l'utilisateur disponible.
