applet-name = Variateur

looking = Recherche des écrans…
display-unnamed = Écran { $number }
no-displays = Aucun écran trouvé.

level = { $value } %

software-unavailable = Atténuation logicielle indisponible : { $reason }
reason-not-found = écran introuvable
reason-read-only = l’écran n’accepte pas de modifications
reason-unsupported = non pris en charge dans cette version
reason-timed-out = pas de réponse à temps
reason-invalid-level = niveau hors limites
reason-unavailable = écran injoignable pour le moment
reason-superseded = remplacé par un niveau plus récent

not-applied = Non appliqué : { $reason }
not-applied-at = Non appliqué : { $reason }. L’écran est à { $value } %.

lock = Déplacer les deux ensemble

auto-dimming = Atténuation automatique
location = Position pour le lever et le coucher du soleil
location-time-zone = D’après le fuseau horaire ({ $zone })
location-placeholder = Latitude, longitude
location-invalid = Position non valide. Saisissez latitude, longitude en degrés, par exemple 39.74, -104.99.
location-unknown = Aucune position connue pour votre fuseau horaire. Saisissez latitude, longitude pour que l’atténuation automatique fonctionne.

settings = Paramètres
floor-hardware = Niveau matériel minimal
floor-software = Luminosité minimale
floor-effective = Niveau combiné minimal
key-step = Variation par appui

keys-unavailable = Touches de luminosité indisponibles : { $error }
shortcuts-toggle = Raccourcis de luminosité
shortcut-raise = Touche pour augmenter la luminosité
shortcut-lower = Touche pour diminuer la luminosité
shortcut-press = Appuyez sur des touches…
shortcut-hint = Appuyez sur une touche F ou une combinaison avec Super, Ctrl ou Alt. Esc annule. Les touches déjà utilisées par COSMIC ne seront pas prises en compte.
shortcuts-taken = Non modifié : { $binding } est déjà utilisé.
shortcuts-invalid = Non modifié : cette combinaison de touches ne peut pas servir de raccourci.
shortcut-description-raise = Variateur : augmenter la luminosité
shortcut-description-lower = Variateur : diminuer la luminosité
shortcuts-unreadable = Non modifié : vos raccourcis personnalisés n’ont pas pu être lus sans risque, ils ont donc été laissés tels quels.
shortcuts-failed = Non modifié : { $error }

usage =
    Utilisation : cosmic-dimmer [--reset | --help | --version]
    Sans option, il s’exécute comme applet du panneau COSMIC.
    --reset  Remet chaque écran à 100 % (atténuation matérielle et logicielle)
             et l’enregistre, pour quand un écran est trop sombre pour utiliser
             l’applet.
reset-applet-done = L’applet en cours d’exécution remet chaque écran à 100 % et retire ses superpositions.
reset-applet-not-running = L’applet n’est pas en cours d’exécution, aucune atténuation logicielle n’est donc à l’écran.
reset-applet-no-answer = L’applet n’a pas répondu ({ $error }). Si l’écran est toujours assombri, arrêtez l’applet avec : pkill -f 'cosmic-dimmer$'
reset-saved = Niveaux enregistrés remis à 100 % pour { $count ->
        [one] { $count } écran
       *[other] { $count } écrans
    }.
reset-saved-failed = Les niveaux enregistrés n’ont pas été réinitialisés : { $error }
reset-saved-no-home = Les niveaux enregistrés n’ont pas été réinitialisés : HOME n’est pas défini, le dossier des paramètres est donc introuvable.
reset-hardware = Luminosité matérielle remise à 100 % sur { $count ->
        [one] { $count } écran
       *[other] { $count } écrans
    }.
reset-hardware-partial = Luminosité matérielle remise à 100 % sur { $count ->
        [one] { $count } écran
       *[other] { $count } écrans
    } ; échec sur { $failed }.
reset-hardware-skipped = Luminosité matérielle inchangée : { $reason }.
reset-failed = Échec de la réinitialisation : { $error }
