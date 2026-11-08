applet-name = Dimmer

looking = Letar efter skärmar…
display-unnamed = Skärm { $number }
no-displays = Inga skärmar hittades.

level = { $value } %

software-unavailable = Mjukvarudimning är inte tillgänglig: { $reason }
reason-not-found = skärmen hittades inte
reason-read-only = skärmen tar inte emot ändringar
reason-unsupported = stöds inte i den här versionen
reason-timed-out = inget svar i tid
reason-invalid-level = nivån är utanför intervallet
reason-unavailable = skärmen kan inte nås just nu
reason-superseded = ersattes av en nyare nivå

not-applied = Tillämpades inte: { $reason }
not-applied-at = Tillämpades inte: { $reason }. Skärmen är på { $value } %.

lock = Flytta båda tillsammans

auto-dimming = Automatisk dimning
location = Plats för soluppgång och solnedgång
location-time-zone = Från tidszon ({ $zone })
location-placeholder = Latitud, longitud
location-invalid = Inte en plats. Skriv latitud, longitud i grader, till exempel 39.74, -104.99.
location-unknown = Din tidszon har ingen känd plats. Skriv latitud, longitud för att automatisk dimning ska fungera.

settings = Inställningar
floor-hardware = Lägsta hårdvarunivå
floor-software = Minsta ljusstyrka
floor-effective = Lägsta sammanlagda nivå
key-step = Ändring per knapptryckning

keys-unavailable = Ljusstyrkeknappar är inte tillgängliga: { $error }
shortcuts-toggle = Kortkommandon för ljusstyrka
shortcut-raise = Tangent för ökad ljusstyrka
shortcut-lower = Tangent för minskad ljusstyrka
shortcut-press = Tryck på tangenter…
shortcut-hint = Tryck på en F-tangent eller en kombination med Super, Ctrl eller Alt. Esc avbryter. Tangenter som COSMIC själv använder registreras inte.
shortcuts-taken = Inte ändrat: { $binding } används redan.
shortcuts-invalid = Inte ändrat: den tangentkombinationen kan inte användas som kortkommando.
shortcut-description-raise = Dimmer: öka ljusstyrkan
shortcut-description-lower = Dimmer: minska ljusstyrkan
shortcuts-unreadable = Inte ändrat: dina egna kortkommandon kunde inte läsas säkert, så de lämnades som de var.
shortcuts-failed = Inte ändrat: { $error }

usage =
    Användning: cosmic-ext-dimmer [--reset | --help | --version]
    Utan flaggor körs den som en panelapplet i COSMIC.
    --reset  Återställ varje skärm till 100 % (hårdvaru- och mjukvarudimning) och
             spara det, för när en skärm är för mörk för att använda appleten.
reset-applet-done = Den körande appleten ställer varje skärm på 100 % och tar bort sina överlägg.
reset-applet-not-running = Appleten körs inte, så ingen mjukvarudimning syns på skärmen.
reset-applet-no-answer = Appleten svarade inte ({ $error }). Om skärmen fortfarande är dimmad, avsluta appleten med: pkill -f 'cosmic-ext-dimmer$'
reset-saved = Sparade nivåer satta till 100 % för { $count ->
        [one] { $count } skärm
       *[other] { $count } skärmar
    }.
reset-saved-failed = Sparade nivåer återställdes inte: { $error }
reset-saved-no-home = Sparade nivåer återställdes inte: HOME är inte satt, så inställningsmappen kan inte hittas.
reset-hardware = Hårdvarans ljusstyrka satt till 100 % på { $count ->
        [one] { $count } skärm
       *[other] { $count } skärmar
    }.
reset-hardware-partial = Hårdvarans ljusstyrka satt till 100 % på { $count ->
        [one] { $count } skärm
       *[other] { $count } skärmar
    }; { $failed } tog inte emot den.
reset-hardware-skipped = Hårdvarans ljusstyrka ändrades inte: { $reason }.
reset-failed = Återställningen misslyckades: { $error }
