applet-name = Dimmer

looking = Procurando telas…
display-unnamed = Tela { $number }
no-displays = Nenhuma tela encontrada.

level = { $value }%

software-unavailable = Escurecimento por software indisponível: { $reason }
reason-not-found = tela não encontrada
reason-read-only = a tela não aceita alterações
reason-unsupported = sem suporte nesta compilação
reason-timed-out = sem resposta a tempo
reason-invalid-level = nível fora do intervalo
reason-unavailable = não é possível acessar a tela agora
reason-superseded = substituído por um nível mais recente

not-applied = Não aplicado: { $reason }
not-applied-at = Não aplicado: { $reason }. A tela está em { $value }%.

lock = Mover os dois juntos

auto-dimming = Escurecimento automático
location = Local para o nascer e o pôr do sol
location-time-zone = Pelo fuso horário ({ $zone })
location-placeholder = Latitude, longitude
location-invalid = Não é um local. Digite latitude, longitude em graus, como 39.74, -104.99.
location-unknown = Seu fuso horário não tem um local conhecido. Digite latitude, longitude para o escurecimento automático funcionar.

settings = Configurações
floor-hardware = Nível mínimo de hardware
floor-software = Brilho mínimo
floor-effective = Nível mínimo combinado
key-step = Variação por tecla pressionada

keys-unavailable = Teclas de brilho indisponíveis: { $error }
shortcuts-toggle = Atalhos de brilho
shortcut-raise = Tecla para aumentar o brilho
shortcut-lower = Tecla para diminuir o brilho
shortcut-press = Pressione as teclas…
shortcut-hint = Pressione uma tecla F ou uma combinação com Super, Ctrl ou Alt. Esc cancela. Teclas que o próprio COSMIC usa não serão registradas.
shortcuts-taken = Não alterado: { $binding } já está em uso.
shortcuts-invalid = Não alterado: essa combinação de teclas não pode ser usada como atalho.
shortcut-description-raise = Dimmer: aumentar o brilho
shortcut-description-lower = Dimmer: diminuir o brilho
shortcuts-unreadable = Não alterado: não foi possível ler seus atalhos personalizados com segurança, então eles foram mantidos como estavam.
shortcuts-failed = Não alterado: { $error }

usage =
    Uso: cosmic-ext-dimmer [--reset | --help | --version]
    Sem opções, é executado como applet do painel do COSMIC.
    --reset  Volta todas as telas para 100% (escurecimento por hardware e por
             software) e salva isso, para quando uma tela está escura demais
             para usar o applet.
reset-applet-done = O applet em execução está colocando todas as telas em 100% e removendo suas sobreposições.
reset-applet-not-running = O applet não está em execução, então não há escurecimento por software na tela.
reset-applet-no-answer = O applet não respondeu ({ $error }). Se a tela ainda estiver escurecida, encerre o applet com: pkill -f 'cosmic-ext-dimmer$'
reset-saved = Níveis salvos definidos como 100% para { $count ->
        [one] { $count } tela
       *[other] { $count } telas
    }.
reset-saved-failed = Os níveis salvos não foram redefinidos: { $error }
reset-saved-no-home = Os níveis salvos não foram redefinidos: HOME não está definida, então não é possível encontrar a pasta de configurações.
reset-hardware = Brilho de hardware definido como 100% em { $count ->
        [one] { $count } tela
       *[other] { $count } telas
    }.
reset-hardware-partial = Brilho de hardware definido como 100% em { $count ->
        [one] { $count } tela
       *[other] { $count } telas
    }; falhou em { $failed }.
reset-hardware-skipped = Brilho de hardware não alterado: { $reason }.
reset-failed = Falha ao redefinir: { $error }
