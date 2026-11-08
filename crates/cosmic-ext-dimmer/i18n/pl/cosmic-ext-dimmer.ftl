applet-name = Ściemniacz

looking = Wyszukiwanie ekranów…
display-unnamed = Ekran { $number }
no-displays = Nie znaleziono ekranów.

level = { $value }%

software-unavailable = Ściemnianie programowe niedostępne: { $reason }
reason-not-found = nie znaleziono ekranu
reason-read-only = ekran nie przyjmuje zmian
reason-unsupported = nieobsługiwane w tej kompilacji
reason-timed-out = brak odpowiedzi w wyznaczonym czasie
reason-invalid-level = poziom poza zakresem
reason-unavailable = ekran jest teraz nieosiągalny
reason-superseded = zastąpiono nowszym poziomem

not-applied = Nie zastosowano: { $reason }
not-applied-at = Nie zastosowano: { $reason }. Ekran jest ustawiony na { $value }%.

lock = Przesuwaj oba razem

auto-dimming = Automatyczne ściemnianie
location = Lokalizacja dla wschodu i zachodu słońca
location-time-zone = Ze strefy czasowej ({ $zone })
location-placeholder = Szerokość, długość geograficzna
location-invalid = To nie jest lokalizacja. Wpisz szerokość, długość geograficzną w stopniach, np. 39.74, -104.99.
location-unknown = Dla twojej strefy czasowej nie jest znana lokalizacja. Wpisz szerokość, długość geograficzną, aby automatyczne ściemnianie działało.

settings = Ustawienia
floor-hardware = Najniższy poziom sprzętowy
floor-software = Minimalna jasność
floor-effective = Najniższy łączny poziom
key-step = Zmiana na naciśnięcie klawisza

keys-unavailable = Klawisze jasności niedostępne: { $error }
shortcuts-toggle = Skróty jasności
shortcut-raise = Klawisz zwiększania jasności
shortcut-lower = Klawisz zmniejszania jasności
shortcut-press = Naciśnij klawisze…
shortcut-hint = Naciśnij klawisz F lub kombinację z Super, Ctrl albo Alt. Esc anuluje. Klawisze używane przez sam COSMIC nie zostaną zarejestrowane.
shortcuts-taken = Nie zmieniono: { $binding } jest już używany.
shortcuts-invalid = Nie zmieniono: tej kombinacji klawiszy nie można użyć jako skrótu.
shortcut-description-raise = Ściemniacz: zwiększ jasność
shortcut-description-lower = Ściemniacz: zmniejsz jasność
shortcuts-unreadable = Nie zmieniono: nie udało się bezpiecznie odczytać twoich własnych skrótów, więc pozostawiono je bez zmian.
shortcuts-failed = Nie zmieniono: { $error }

usage =
    Użycie: cosmic-ext-dimmer [--reset | --help | --version]
    Bez opcji działa jako aplet panelu COSMIC.
    --reset  Przywraca każdy ekran do 100% (ściemnianie sprzętowe i programowe)
             i zapisuje to, na wypadek gdy ekran jest zbyt ciemny, by użyć apletu.
reset-applet-done = Działający aplet ustawia każdy ekran na 100% i usuwa swoje nakładki.
reset-applet-not-running = Aplet nie działa, więc na ekranie nie ma ściemniania programowego.
reset-applet-no-answer = Aplet nie odpowiedział ({ $error }). Jeśli ekran jest nadal przyciemniony, zakończ aplet poleceniem: pkill -f 'cosmic-ext-dimmer$'
reset-saved = Zapisane poziomy ustawiono na 100% dla { $count ->
        [one] { $count } ekranu
        [few] { $count } ekranów
        [many] { $count } ekranów
       *[other] { $count } ekranu
    }.
reset-saved-failed = Nie zresetowano zapisanych poziomów: { $error }
reset-saved-no-home = Nie zresetowano zapisanych poziomów: zmienna HOME nie jest ustawiona, więc nie można znaleźć folderu ustawień.
reset-hardware = Jasność sprzętową ustawiono na 100% na { $count ->
        [one] { $count } ekranie
        [few] { $count } ekranach
        [many] { $count } ekranach
       *[other] { $count } ekranu
    }.
reset-hardware-partial = Jasność sprzętową ustawiono na 100% na { $count ->
        [one] { $count } ekranie
        [few] { $count } ekranach
        [many] { $count } ekranach
       *[other] { $count } ekranu
    }; liczba ekranów, które jej nie przyjęły: { $failed }.
reset-hardware-skipped = Nie zmieniono jasności sprzętowej: { $reason }.
reset-failed = Resetowanie nie powiodło się: { $error }
