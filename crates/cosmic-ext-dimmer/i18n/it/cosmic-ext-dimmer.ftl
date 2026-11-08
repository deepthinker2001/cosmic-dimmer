applet-name = Dimmer

looking = Ricerca degli schermi…
display-unnamed = Schermo { $number }
no-displays = Nessuno schermo trovato.

level = { $value }%

software-unavailable = Attenuazione software non disponibile: { $reason }
reason-not-found = schermo non trovato
reason-read-only = lo schermo non accetta modifiche
reason-unsupported = non supportato in questa build
reason-timed-out = nessuna risposta in tempo
reason-invalid-level = livello fuori intervallo
reason-unavailable = schermo non raggiungibile al momento
reason-superseded = sostituito da un livello più recente

not-applied = Non applicato: { $reason }
not-applied-at = Non applicato: { $reason }. Lo schermo è al { $value }%.

lock = Muovi entrambi insieme

auto-dimming = Attenuazione automatica
location = Posizione per alba e tramonto
location-time-zone = Dal fuso orario ({ $zone })
location-placeholder = Latitudine, longitudine
location-invalid = Posizione non valida. Inserisci latitudine, longitudine in gradi, ad esempio 39.74, -104.99.
location-unknown = Il tuo fuso orario non ha una posizione nota. Inserisci latitudine, longitudine perché l’attenuazione automatica funzioni.

settings = Impostazioni
floor-hardware = Livello hardware minimo
floor-software = Luminosità minima
floor-effective = Livello combinato minimo
key-step = Variazione per pressione

keys-unavailable = Tasti di luminosità non disponibili: { $error }
shortcuts-toggle = Scorciatoie di luminosità
shortcut-raise = Tasto per aumentare la luminosità
shortcut-lower = Tasto per diminuire la luminosità
shortcut-press = Premi i tasti…
shortcut-hint = Premi un tasto F o una combinazione con Super, Ctrl o Alt. Esc annulla. I tasti già usati da COSMIC non verranno registrati.
shortcuts-taken = Non modificato: { $binding } è già in uso.
shortcuts-invalid = Non modificato: questa combinazione di tasti non può essere usata come scorciatoia.
shortcut-description-raise = Dimmer: aumenta la luminosità
shortcut-description-lower = Dimmer: diminuisci la luminosità
shortcuts-unreadable = Non modificato: non è stato possibile leggere in sicurezza le tue scorciatoie personalizzate, quindi sono state lasciate invariate.
shortcuts-failed = Non modificato: { $error }

usage =
    Uso: cosmic-ext-dimmer [--reset | --help | --version]
    Senza opzioni viene eseguito come applet del pannello di COSMIC.
    --reset  Riporta ogni schermo al 100% (attenuazione hardware e software) e
             lo salva, per quando uno schermo è troppo scuro per usare l’applet.
reset-applet-done = L’applet in esecuzione sta riportando ogni schermo al 100% e rimuovendo le sue sovrapposizioni.
reset-applet-not-running = L’applet non è in esecuzione, quindi sullo schermo non c’è attenuazione software.
reset-applet-no-answer = L’applet non ha risposto ({ $error }). Se lo schermo è ancora attenuato, termina l’applet con: pkill -f 'cosmic-ext-dimmer$'
reset-saved = Livelli salvati riportati al 100% per { $count ->
        [one] { $count } schermo
       *[other] { $count } schermi
    }.
reset-saved-failed = I livelli salvati non sono stati reimpostati: { $error }
reset-saved-no-home = I livelli salvati non sono stati reimpostati: HOME non è impostata, quindi non è possibile trovare la cartella delle impostazioni.
reset-hardware = Luminosità hardware riportata al 100% su { $count ->
        [one] { $count } schermo
       *[other] { $count } schermi
    }.
reset-hardware-partial = Luminosità hardware riportata al 100% su { $count ->
        [one] { $count } schermo
       *[other] { $count } schermi
    }; non riuscito su { $failed }.
reset-hardware-skipped = Luminosità hardware non modificata: { $reason }.
reset-failed = Ripristino non riuscito: { $error }
