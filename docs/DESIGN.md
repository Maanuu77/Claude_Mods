# DEV COCKPIT: sistema visual «Holographic»

> **Estado:** normativo para la interfaz desde el 2026-11-15. La SPEC define qué muestra cada vista (§7–§10); este documento define **cómo se ve**. Si alguna vez discreparan en algo funcional, manda la SPEC.
>
> **Origen:** la SPEC no fijaba una estética. Un primer sistema sobrio («Instrumento», 2026-11-13) no respondía a lo que el usuario buscaba. El 2026-11-15 pidió una dirección **HOLOGRAPHIC 3D · SPATIAL INTERFACE · FUTURISTIC COMMAND CENTER**: paneles flotantes con profundidad, hologramas, cristal, luz volumétrica, gráficos con volumen y animaciones cinematográficas, sin perjudicar la legibilidad, la velocidad, la accesibilidad ni la interacción. Aprobó la propuesta conceptual de las seis pantallas (lienzo «DEV COCKPIT Holographic»), con un solo tema oscuro, y este documento la fija.

## 1. Idea

DEV COCKPIT es un **centro de mando**: un espacio con luz en el que flotan paneles de cristal y, entre ellos, hologramas de lo que está pasando. Cinco principios deciden todo lo demás:

1. **El 3D vive en el fondo y en las visualizaciones; el texto, siempre de frente.** Ningún texto se inclina, se desenfoca ni se anima. Las etiquetas sobre un plano 3D deshacen su inclinación y miran al espectador.
2. **La luz es estado.** Azul eléctrico y cian para lo activo, violeta para lo estructural, verde solo en pequeños acentos de «correcto»; ámbar y rosa para atención y fallo (§3). Lo que brilla, significa algo, y siempre va con palabra o icono.
3. **Profundidad por planos** (§5): espacio, hologramas, cristal y HUD. La cercanía se dice con sombra y halo, nunca con desenfoque del texto.
4. **Gráficos honestos.** Los prismas 3D usan proyección oblicua: base horizontal y alturas comparables. Cada gráfico conserva su vista de tabla.
5. **Solo se mueve lo vivo.** Los anillos giran si la sesión trabaja; late lo que está en curso. Sin sesiones trabajando, nada se mueve. Con «reducir movimiento» la escena queda quieta, pero sigue en 3D. Con la ventana oculta, todo se pausa.

Un solo tema, oscuro: el espacio de mando es la identidad de la app.

## 2. Tokens

Todos viven en `src/ui/tokens.css`. **Ningún componente usa un color o tamaño literal**: siempre un token (las únicas excepciones son la marca, que es fija, y los blancos de las cifras con brillo).

### 2.1 Espacio, cristal y texto

| Token | Valor | Uso |
|---|---|---|
| `--bg` | `#03050C` | el vacío del espacio |
| `--glass` | degradado `rgba(44,72,156,.40)` → `rgba(8,12,30,.92)` | paneles de cristal |
| `--glass-dock` | degradado azul → `rgba(7,11,26,.90)` | dock lateral |
| `--rim` | degradado `#8AB8FF` → violeta → cian | canto luminoso de los paneles |
| `--surface-solid` | `#0B1126` | cabeceras fijas de tabla, tooltips, base del cristal |
| `--sunken` | `rgba(2,5,14,.55)` | pozos: listas, código, árboles |
| `--border` / `--border-strong` | `rgba(138,184,255,.12)` / `.26` | separaciones |
| `--text` | `#EAF2FF` | texto principal |
| `--text-muted` | `#A9B8D8` | metadatos |
| `--text-faint` | `#8FA0C4` | etiquetas HUD |

### 2.2 Luz (semántica, §3)

| Token | Valor | Suave |
|---|---|---|
| `--accent` / `--accent-text` | `#3D7BFF` / `#8AB8FF` | `rgba(61,123,255,.16)` |
| `--cyan` | `#2EE6FF` | — |
| `--violet` / `--violet-text` | `#A07BFF` / `#C3A8FF` | — |
| `--positive` | `#45FFB0` | `rgba(69,255,176,.12)` |
| `--attention` | `#FFC163` | `rgba(255,193,99,.12)` |
| `--critical` | `#FF6D8F` | `rgba(255,109,143,.12)` |
| `--neutral` | `#8FA0C4` | `rgba(143,160,196,.12)` |

### 2.3 Datos

- **Una serie** (tokens por día, actividad): prismas con degradado `--prism-hi` `#7FB4FF` → `--prism-lo` `#2A5BE0`; el máximo, en cian.
- **Dos series** (límites de 5 h y 7 días): `--series-1` `#109FCF` y `--series-2` `#8F5BFF`. Par validado con la guía de visualización sobre `#0B1126`: dentro de la banda de luminosidad, ΔE 12 con deuteranopía y contraste ≥ 3:1. Siempre con leyenda y etiqueta directa.

### 2.4 Tipografía

Solo fuentes de Windows: nada se descarga.

| Uso | Fuente | Tamaño |
|---|---|---|
| Títulos de sala y cifras | Bahnschrift Light / Regular | 34 px (título), 40 px (cifra grande), 22 px (cifra) |
| Etiquetas HUD, antetítulos, nombres | Bahnschrift, versalitas con tracking 0,18–0,32 em | 10,5–13 px |
| Lectura | Segoe UI Variable Text | 13 px |
| Rutas e identificadores | Cascadia Mono | 12 px |

Las cifras usan numerales tabulares.

### 2.5 Forma y profundidad

Rejilla de 4 px. Radios 6 / 10 / 14 / 20 px. Sombras contenidas (`--shadow-1`, `--shadow-lift`): una de caída y un halo azul. Los desenfoques son de 36–60 px como mucho, porque cada sombra grande es una máscara que la GPU rasteriza y guarda (§11).

### 2.6 Movimiento

140–220 ms para transiciones; entrada de paneles 640 ms escalonada; anillos de 7 a 12 s por vuelta. Solo `transform` y `opacity`.

### 2.7 Foco

Anillo cian de 2 px con halo, visible sobre cualquier plano.

## 3. Semántica de estado

| Luz | Significa | Ejemplos |
|---|---|---|
| **accent** (azul eléctrico) | actividad en curso, selección | sesión «Trabajando», herramienta en curso, vista activa, botón principal |
| **cian** | foco, lo que está pasando ahora | paso en curso, estela de actividad, barrido de luz, anillo de foco |
| **violeta** | estructura | grupos del mapa, relación «contiene», subagente |
| **positive** (verde) | sano, terminado, conectado | herramienta «ok», tarea hecha, mod conectado, al día |
| **attention** (ámbar) | necesita mirarse sin ser un fallo | esperando permiso, días por agregar, bloqueos, proyecto sin vigilar |
| **critical** (rosa) | fallo | error de herramienta, núcleo sin conexión, días que no cuadran |
| **neutral** (gris azulado) | inactivo o sin dato | sin proceso, interrumpida, dial sin lectura |

Cada luz aparece como **baliza** (`StatusDot`, punto con halo; con onda si está en curso), **chip** (`Badge`, píldora teñida) o **canto** encendido de un panel vivo. Siempre con texto o icono.

## 4. Iconografía y marca

Juego propio en `src/ui/icons.tsx`: SVG de 16 px (18 en el dock), trazo de 1,5 px, `currentColor`. Un icono solo lleva `aria-label` y tooltip.

**Marca:** un dial de instrumento con su aguja, en un cuadrado con degradado azul eléctrico → violeta y el arco encendido en cian. Es el icono de la ventana, de la bandeja y del instalador (`src-tauri/icons`, generados con `tauri icon` desde `src-tauri/icons/source.svg`).

## 5. Planos de profundidad y estructura

| Plano | Qué vive ahí |
|---|---|
| **0 · Espacio** | halos de luz volumétrica, partículas y suelo holográfico en perspectiva (`Space`, estático) |
| **1 · Hologramas** | reactor de sesión, diales proyectados, prismas, constelaciones de subagentes y componentes |
| **2 · Cristal** | paneles con todo el texto, siempre de frente |
| **3 · HUD** | cabecera de sala, visor de diff, avisos, confirmaciones, tooltips |

- **Dock lateral** flotante (228 px; 72 px, solo iconos con tooltip, por debajo de 1.000 px): marca, seis salas con contador y haz de luz en la activa, y el módulo del núcleo.
- **Cabecera HUD:** antetítulo de la sala («Puente de mando · 01»…), título con degradado de luz, resumen, acciones y reloj.
- **Apertura:** al arrancar, el reactor se enciende con la lista de encendido real (núcleo, historial, proyectos) durante 1,6 s; se puede saltar y no aparece con «reducir movimiento». Después, los paneles llegan desde el fondo.
- Al cambiar de sala, el contenido vuelve arriba y los paneles entran escalonados.

## 6. Componentes compartidos

En `src/ui/` (`components.tsx`, `components.css`). Las vistas no definen botones, chips, tablas ni avisos propios.

| Componente | Qué es |
|---|---|
| `Card` | panel de cristal; `edge` enciende su canto con el tono, `live` añade el barrido, `plane` lo acerca (`lift`) o lo aleja (`far`, por color, nunca por desenfoque) |
| `Button` | `primary` (degradado azul → cian con halo), `secondary` (cristal), `ghost`, `danger` |
| `Segmented` | selector de cristal; un punto de tabulación y flechas |
| `Badge` / `StatusDot` | chip y baliza de §3 |
| `ProgressBar` | segmentos de luz |
| `Gauge` | dial holográfico de 270° sobre su proyector; sin dato, apagado y con su explicación |
| `Reactor` | anillos 3D del color del estado alrededor de un núcleo; giran solo si la sesión trabaja |
| `Prisms` | prismas 3D en proyección oblicua con etiqueta del máximo y tooltip por prisma |
| `Trail` | estela de actividad: una luz por minuto con herramientas |
| `Metric` | cifra de instrumento con brillo; `lg` para las fichas |
| `PropertyList`, `Notice`, `EmptyState`, `Loading`, `Skeleton`, `ConfirmBar`, `Switch`, `NumberField` | resto de piezas, con la misma luz |
| `LiveEmpty` (`src/app/Shell.tsx`) | vacío de las salas que llena el núcleo: distingue conectando, sin conexión y vacío |

## 7. Estados de cada vista

- **Vacío:** `EmptyState` con su icono en un aro de luz, qué falta y cuándo aparecerá. En las salas del núcleo, `LiveEmpty`.
- **Cargando:** `Loading` (aro cian) o `Skeleton` con brillo que recorre.
- **Error:** `Notice` crítico con «Reintentar» si procede; las tarjetas que dependen de esa lectura dicen «Sin datos».
- **Sin conexión con el núcleo:** módulo del dock en rosa y aviso global; los datos ya pintados siguen visibles.
- **Lectura del historial:** aviso con su barra de segmentos y la cifra en el dock.

## 8. Aplicación por sala

| Sala | Composición |
|---|---|
| **01 · En vivo, puente de mando** | La sesión viva más reciente en el plano cercano: reactor, diales de contexto y límites, seis cifras, planes, estela de actividad, herramientas en curso y últimas. Las demás, compactas a su lado; las que no tienen proceso, en el plano lejano. |
| **02 · Tareas, plan de misión** | Cada lista sobre su espina de luz: hecho en verde, en curso encendido con onda, bloqueos en ámbar, borradas tachadas. Subagentes en una constelación sobre un plano inclinado (fondo: subagentes; centro: bucle principal; delante: sus hijos) y su lista con todos los datos. Actividad en prismas. |
| **03 · Cambios en directo, flujo** | Corriente de cambios con tiempos, tipo en versalitas de color, ruta y quién; lo recién llegado entra desde el fondo y brilla. +/− como barras de luz. El visor de diff, en el plano frontal a todo el ancho. |
| **04 · Mapa, constelación** | Árbol en un pozo; componentes en cristal. Con hasta 12 componentes, el grafo es una constelación sobre un plano holográfico con pilares de luz y etiquetas de frente; con más, el grafo plano. |
| **05 · Histórico, telemetría** | Fichas con cifra grande y línea de luz; tokens por día en prismas; límites como líneas de luz con área en dos capas; tablas de cristal. |
| **06 · Diagnóstico, sistemas** | Módulos con su estado en el canto y un resumen «N sistemas al día · M con aviso» en la cabecera; la base de datos como cilindro holográfico por capas; ajustes con interruptores de luz; lo irreversible, con su confirmación en rosa. |

## 9. Tamaños de ventana

- **≥ 1.100 px:** fichas del Histórico en fila de cuatro.
- **< 1.100 px:** fichas 2 × 2.
- **< 1.000 px:** dock de iconos; paneles de dos columnas pasan a una.
- **< 640 px:** cifras grandes a 28 px y cabecera compacta.

En una fila con chips, la ruta es lo primero que cede sitio; el nombre, los chips y los tiempos nunca se recortan ni se salen del panel.

## 10. Accesibilidad

- Contraste del texto ≥ 4,5:1 sobre el cristal en su punto más claro (el tono más tenue, `#8FA0C4`, da 5,0:1).
- El estado nunca depende solo de la luz: siempre palabra o icono.
- Teclado: foco visible, Tab y Enter en el dock, flechas en los selectores; los iconos solos llevan nombre accesible.
- Las gráficas tienen etiqueta accesible, tooltip y vista de tabla.
- `prefers-reduced-motion`: sin apertura, sin giros ni latidos; la escena queda quieta y en 3D.

## 11. Rendimiento

Solo CSS 3D, SVG y degradados, sin librerías. Las decisiones que lo sostienen:

- Cristal **sin** `backdrop-filter` (cada panel guardaría una textura de lo que tiene detrás); el canto es un borde degradado, no una máscara.
- Suelo en perspectiva dibujado como SVG plano, no como capa 3D.
- Entradas animadas con relleno `backwards`: al terminar, la GPU suelta la capa de cada panel.
- Plano lejano por color, no por opacidad de grupo; sin `box-reflect` ni filtros de sombra en lo que se repinta.
- Sin sesiones trabajando no hay animaciones continuas; con la ventana oculta, se pausan.

Medido el 2026-11-15 con el muestreador de B8 (C2), en el mismo equipo y a continuación de la versión anterior:

| | En reposo (sin sesiones) | Con una sesión trabajando |
|---|---|---|
| Holographic | 236–262 MB (media de cada ejecución), CPU 0,09–0,16 % | 242–252 MB, CPU 0,07–0,12 % |
| Versión anterior | 224–230 MB, CPU 0,06–0,22 % | 227–245 MB, CPU 0,06–0,15 % |

La diferencia está en el proceso GPU de WebView2: unos +15 MB en reposo y +20 MB con una sesión trabajando. Entre ejecuciones la variación es de ±15 MB y depende del estado del equipo (el mismo día, la versión anterior llegó a 268 MB de máximo con carga). La CPU no cambia. La memoria queda en el límite de B8 (< 250 MB): por debajo en la mayoría de las ejecuciones en reposo y justo en él con carga. Antes de cerrar la fase hay que repetir B8 y B9 formalmente.
