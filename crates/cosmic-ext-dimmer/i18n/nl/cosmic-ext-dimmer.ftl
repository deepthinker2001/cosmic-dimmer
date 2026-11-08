applet-name = Dimmer

looking = Beeldschermen zoeken…
display-unnamed = Beeldscherm { $number }
no-displays = Geen beeldschermen gevonden.

level = { $value }%

software-unavailable = Softwarematig dimmen niet beschikbaar: { $reason }
reason-not-found = beeldscherm niet gevonden
reason-read-only = beeldscherm accepteert geen wijzigingen
reason-unsupported = niet ondersteund in deze build
reason-timed-out = geen antwoord op tijd
reason-invalid-level = niveau buiten bereik
reason-unavailable = beeldscherm is nu niet bereikbaar
reason-superseded = vervangen door een nieuwer niveau

not-applied = Niet toegepast: { $reason }
not-applied-at = Niet toegepast: { $reason }. Het beeldscherm staat op { $value }%.

lock = Beide samen verplaatsen

auto-dimming = Automatisch dimmen
location = Locatie voor zonsopkomst en zonsondergang
location-time-zone = Uit tijdzone ({ $zone })
location-placeholder = Breedtegraad, lengtegraad
location-invalid = Geen locatie. Typ breedtegraad, lengtegraad in graden, zoals 39.74, -104.99.
location-unknown = Voor je tijdzone is geen locatie bekend. Typ breedtegraad, lengtegraad zodat automatisch dimmen werkt.

settings = Instellingen
floor-hardware = Laagste hardwareniveau
floor-software = Minimale helderheid
floor-effective = Laagste gecombineerde niveau
key-step = Wijziging per toetsaanslag

keys-unavailable = Helderheidstoetsen niet beschikbaar: { $error }
shortcuts-toggle = Sneltoetsen voor helderheid
shortcut-raise = Toets voor helderheid verhogen
shortcut-lower = Toets voor helderheid verlagen
shortcut-press = Druk op toetsen…
shortcut-hint = Druk op een F-toets, of een combinatie met Super, Ctrl of Alt. Esc annuleert. Toetsen die COSMIC zelf gebruikt, worden niet geregistreerd.
shortcuts-taken = Niet gewijzigd: { $binding } is al in gebruik.
shortcuts-invalid = Niet gewijzigd: die toetscombinatie kan niet als sneltoets worden gebruikt.
shortcut-description-raise = Dimmer: helderheid verhogen
shortcut-description-lower = Dimmer: helderheid verlagen
shortcuts-unreadable = Niet gewijzigd: je eigen sneltoetsen konden niet veilig worden gelezen, dus ze zijn ongewijzigd gelaten.
shortcuts-failed = Niet gewijzigd: { $error }

usage =
    Gebruik: cosmic-ext-dimmer [--reset | --help | --version]
    Zonder opties draait het als COSMIC-paneelapplet.
    --reset  Zet elk beeldscherm terug op 100% (hardwarematig en softwarematig
             dimmen) en sla dat op, voor als een scherm te donker is voor de applet.
reset-applet-done = De draaiende applet zet elk beeldscherm op 100% en verwijdert de overlays.
reset-applet-not-running = De applet draait niet, dus er is geen softwarematig dimmen op het scherm.
reset-applet-no-answer = De applet antwoordde niet ({ $error }). Als het scherm nog gedimd is, beëindig de applet dan met: pkill -f 'cosmic-ext-dimmer$'
reset-saved = Opgeslagen niveaus op 100% gezet voor { $count ->
        [one] { $count } beeldscherm
       *[other] { $count } beeldschermen
    }.
reset-saved-failed = Opgeslagen niveaus zijn niet teruggezet: { $error }
reset-saved-no-home = Opgeslagen niveaus zijn niet teruggezet: HOME is niet ingesteld, dus de map met instellingen kan niet worden gevonden.
reset-hardware = Hardwarehelderheid op 100% gezet op { $count ->
        [one] { $count } beeldscherm
       *[other] { $count } beeldschermen
    }.
reset-hardware-partial = Hardwarehelderheid op 100% gezet op { $count ->
        [one] { $count } beeldscherm
       *[other] { $count } beeldschermen
    }; bij { $failed } is het niet gelukt.
reset-hardware-skipped = Hardwarehelderheid niet gewijzigd: { $reason }.
reset-failed = Terugzetten mislukt: { $error }
