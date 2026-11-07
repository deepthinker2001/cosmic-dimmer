applet-name = Atenuador

looking = Buscando pantallas…
display-unnamed = Pantalla { $number }
no-displays = No se encontraron pantallas.

level = { $value }%

software-unavailable = Atenuación por software no disponible: { $reason }
reason-not-found = pantalla no encontrada
reason-read-only = la pantalla no acepta cambios
reason-unsupported = no compatible con esta compilación
reason-timed-out = sin respuesta a tiempo
reason-invalid-level = nivel fuera de rango
reason-unavailable = no se puede acceder a la pantalla en este momento
reason-superseded = reemplazado por un nivel más reciente

not-applied = No aplicado: { $reason }
not-applied-at = No aplicado: { $reason }. La pantalla está al { $value }%.

lock = Mover ambos a la vez

auto-dimming = Atenuación automática
location = Ubicación para el amanecer y el atardecer
location-time-zone = Según la zona horaria ({ $zone })
location-placeholder = Latitud, longitud
location-invalid = No es una ubicación. Escriba latitud, longitud en grados, como 39.74, -104.99.
location-unknown = Su zona horaria no tiene una ubicación conocida. Escriba latitud, longitud para que funcione la atenuación automática.

settings = Configuración
floor-hardware = Nivel mínimo de hardware
floor-software = Brillo mínimo
floor-effective = Nivel mínimo combinado
key-step = Cambio por pulsación

keys-unavailable = Teclas de brillo no disponibles: { $error }
shortcuts-toggle = Atajos de brillo
shortcut-raise = Tecla para subir el brillo
shortcut-lower = Tecla para bajar el brillo
shortcut-press = Pulse las teclas…
shortcut-hint = Pulse una tecla F o una combinación con Super, Ctrl o Alt. Esc cancela. Las teclas que ya usa COSMIC no se registrarán.
shortcuts-taken = Sin cambios: { $binding } ya está en uso.
shortcuts-invalid = Sin cambios: esa combinación de teclas no se puede usar como atajo.
shortcut-description-raise = Atenuador: subir el brillo
shortcut-description-lower = Atenuador: bajar el brillo
shortcuts-unreadable = Sin cambios: sus atajos personalizados no se pudieron leer de forma segura, así que se dejaron como estaban.
shortcuts-failed = Sin cambios: { $error }

usage =
    Uso: cosmic-dimmer [--reset | --help | --version]
    Sin opciones, se ejecuta como applet del panel de COSMIC.
    --reset  Devuelve todas las pantallas al 100% (atenuación por hardware y
             por software) y lo guarda, para cuando una pantalla está demasiado
             oscura para usar el applet.
reset-applet-done = El applet en ejecución está poniendo todas las pantallas al 100% y quitando sus superposiciones.
reset-applet-not-running = El applet no se está ejecutando, así que no hay atenuación por software en pantalla.
reset-applet-no-answer = El applet no respondió ({ $error }). Si la pantalla sigue atenuada, cierre el applet con: pkill -f 'cosmic-dimmer$'
reset-saved = Niveles guardados puestos al 100% para { $count ->
        [one] { $count } pantalla
       *[other] { $count } pantallas
    }.
reset-saved-failed = No se restablecieron los niveles guardados: { $error }
reset-saved-no-home = No se restablecieron los niveles guardados: HOME no está definida, así que no se encuentra la carpeta de configuración.
reset-hardware = Brillo de hardware puesto al 100% en { $count ->
        [one] { $count } pantalla
       *[other] { $count } pantallas
    }.
reset-hardware-partial = Brillo de hardware puesto al 100% en { $count ->
        [one] { $count } pantalla
       *[other] { $count } pantallas
    }; falló en { $failed }.
reset-hardware-skipped = Brillo de hardware sin cambios: { $reason }.
reset-failed = Error al restablecer: { $error }
