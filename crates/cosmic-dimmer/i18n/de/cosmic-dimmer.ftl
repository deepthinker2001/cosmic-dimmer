applet-name = Dimmer

looking = Bildschirme werden gesucht…
display-unnamed = Bildschirm { $number }
no-displays = Keine Bildschirme gefunden.

level = { $value } %

software-unavailable = Software-Dimmung nicht verfügbar: { $reason }
reason-not-found = Bildschirm nicht gefunden
reason-read-only = Bildschirm nimmt keine Änderungen an
reason-unsupported = in diesem Build nicht unterstützt
reason-timed-out = keine rechtzeitige Antwort
reason-invalid-level = Stufe außerhalb des gültigen Bereichs
reason-unavailable = Bildschirm ist gerade nicht erreichbar
reason-superseded = durch eine neuere Stufe ersetzt

not-applied = Nicht angewendet: { $reason }
not-applied-at = Nicht angewendet: { $reason }. Der Bildschirm steht auf { $value } %.

lock = Beide gemeinsam bewegen

auto-dimming = Automatisches Dimmen
location = Standort für Sonnenauf- und -untergang
location-time-zone = Aus Zeitzone ({ $zone })
location-placeholder = Breitengrad, Längengrad
location-invalid = Kein gültiger Standort. Geben Sie Breitengrad, Längengrad in Grad ein, etwa 39.74, -104.99.
location-unknown = Für Ihre Zeitzone ist kein Standort bekannt. Geben Sie Breitengrad, Längengrad ein, damit automatisches Dimmen funktioniert.

settings = Einstellungen
floor-hardware = Niedrigste Hardware-Stufe
floor-software = Minimale Helligkeit
floor-effective = Niedrigste Gesamtstufe
key-step = Änderung pro Tastendruck

keys-unavailable = Helligkeitstasten nicht verfügbar: { $error }
shortcuts-toggle = Helligkeits-Tastenkürzel
shortcut-raise = Taste für mehr Helligkeit
shortcut-lower = Taste für weniger Helligkeit
shortcut-press = Tasten drücken…
shortcut-hint = Drücken Sie eine F-Taste oder eine Kombination mit Super, Ctrl oder Alt. Esc bricht ab. Tasten, die COSMIC selbst verwendet, werden nicht erkannt.
shortcuts-taken = Nicht geändert: { $binding } ist bereits belegt.
shortcuts-invalid = Nicht geändert: Diese Tastenkombination kann nicht als Tastenkürzel verwendet werden.
shortcut-description-raise = Dimmer: Helligkeit erhöhen
shortcut-description-lower = Dimmer: Helligkeit verringern
shortcuts-unreadable = Nicht geändert: Ihre eigenen Tastenkürzel konnten nicht sicher gelesen werden und wurden daher unverändert gelassen.
shortcuts-failed = Nicht geändert: { $error }

usage =
    Aufruf: cosmic-dimmer [--reset | --help | --version]
    Ohne Optionen läuft es als COSMIC-Panel-Applet.
    --reset  Setzt jeden Bildschirm auf 100 % zurück (Hardware- und Software-
             Dimmung) und speichert das, für den Fall, dass ein Bildschirm zu
             dunkel ist, um das Applet zu bedienen.
reset-applet-done = Das laufende Applet setzt jeden Bildschirm auf 100 % und entfernt seine Überlagerungen.
reset-applet-not-running = Das Applet läuft nicht, daher ist keine Software-Dimmung auf dem Bildschirm aktiv.
reset-applet-no-answer = Das Applet hat nicht geantwortet ({ $error }). Wenn der Bildschirm noch gedimmt ist, beenden Sie das Applet mit: pkill -f 'cosmic-dimmer$'
reset-saved = Gespeicherte Stufen für { $count ->
        [one] { $count } Bildschirm
       *[other] { $count } Bildschirme
    } auf 100 % gesetzt.
reset-saved-failed = Gespeicherte Stufen wurden nicht zurückgesetzt: { $error }
reset-saved-no-home = Gespeicherte Stufen wurden nicht zurückgesetzt: HOME ist nicht gesetzt, daher ist der Einstellungsordner nicht auffindbar.
reset-hardware = Hardware-Helligkeit von { $count ->
        [one] { $count } Bildschirm
       *[other] { $count } Bildschirmen
    } auf 100 % gesetzt.
reset-hardware-partial = Hardware-Helligkeit von { $count ->
        [one] { $count } Bildschirm
       *[other] { $count } Bildschirmen
    } auf 100 % gesetzt; bei { $failed } fehlgeschlagen.
reset-hardware-skipped = Hardware-Helligkeit nicht geändert: { $reason }.
reset-failed = Zurücksetzen fehlgeschlagen: { $error }
