# DEV COCKPIT: sistema visual «Instrumento»

> **Estado:** normativo para la interfaz desde el 2026-11-13. La SPEC define qué muestra cada vista (§7–§10); este documento define **cómo se ve**. Si alguna vez discreparan en algo funcional, manda la SPEC.
>
> **Origen:** ni la SPEC ni las revisiones aprobadas recogían una propuesta estética (se buscó en el prompt inicial, en las seis revisiones de la SPEC, en el plan aprobado y en el repositorio). El usuario pidió desarrollar la estética completa sin consultas intermedias; este sistema se deriva de lo que sí está aprobado: el propósito de la app (observar, nunca actuar), sus estados y sus restricciones (sin dependencias ni llamadas externas, Windows 11, uso personal).

## 1. Idea

DEV COCKPIT es un **cuadro de instrumentos**: se mira de reojo mientras se trabaja con Claude Code y tiene que decir, de un vistazo, qué está pasando y si algo necesita atención. De ahí salen cinco principios, que deciden todo lo demás:

1. **El color es estado, no decoración.** Las superficies son neutras. El color aparece solo cuando significa algo, y siempre con el mismo significado en todas las vistas (§3). Nunca va solo: lo acompaña un icono o una palabra.
2. **Lectura de instrumento.** Cifras en tipografía tabular y alineadas a la derecha; identificadores y rutas en monoespaciada; etiquetas cortas en versalitas pequeñas. Lo importante es la cifra, no su marco.
3. **Densidad calibrada.** Mucha información en poco espacio, pero con aire suficiente para leerla: rejilla de 4 px, filas de 28–32 px, tarjetas de 16 px de relleno.
4. **Profundidad mínima.** Tres niveles: lienzo, superficie y elevado. Se separan con bordes finos; la sombra solo en lo que flota (diálogos, avisos emergentes).
5. **Movimiento con propósito.** Solo se anima lo vivo (una herramienta en curso, un estado que cambia). Nada se mueve por adorno y todo respeta `prefers-reduced-motion`.

La app sigue el tema claro u oscuro del sistema (como hasta ahora). Los dos temas están diseñados por separado, con sus contrastes comprobados (§10), no invertidos.

## 2. Tokens

Todos viven en `src/ui/tokens.css` como propiedades CSS. **Ningún componente usa un color, tamaño o radio literal**: siempre un token.

### 2.1 Superficies y texto

| Token | Claro | Oscuro | Uso |
|---|---|---|---|
| `--bg` | `#F4F5F7` | `#0E1116` | lienzo de la ventana |
| `--nav` | `#FBFBFC` | `#11141A` | barra lateral |
| `--surface` | `#FFFFFF` | `#161A21` | tarjetas y paneles |
| `--sunken` | `#F8F9FB` | `#12161C` | pozos: listas, código, árboles |
| `--hover` | `#F1F3F6` | `#1C2129` | fila o control bajo el puntero |
| `--border` | `#E2E5EB` | `#252B35` | separación normal |
| `--border-strong` | `#CDD2DB` | `#343C48` | controles, separación marcada |
| `--text` | `#161A21` | `#E6E9EF` | texto principal |
| `--text-muted` | `#566070` | `#9AA3B2` | metadatos, etiquetas |
| `--text-faint` | `#79828F` | `#737C8C` | marcadores de posición, iconos secundarios (nunca texto esencial) |

### 2.2 Semántica (§3)

| Token | Claro (color / fondo suave) | Oscuro (color / fondo suave) |
|---|---|---|
| `--accent` | `#2459DB` / `#EAF0FF` | `#6E9CFF` / `#1A2640` |
| `--positive` | `#147A3B` / `#E5F5EB` | `#4DC48B` / `#122A1F` |
| `--attention` | `#9A5800` / `#FFF3DD` | `#EDB04D` / `#2D2310` |
| `--critical` | `#C02828` / `#FDEBEB` | `#F27070` / `#361A1D` |
| `--neutral` | `#566070` / `#EEF0F4` | `#9AA3B2` / `#222833` |

### 2.3 Datos

Paleta categórica validada (guía de visualización de datos, ΔE CVD ≥ 24 entre las dos series, contraste ≥ 3:1):

| Token | Claro | Oscuro | Uso |
|---|---|---|---|
| `--series-1` | `#2a78d6` | `#3987e5` | serie única (tokens por día, actividad) y primera serie |
| `--series-2` | `#eb6834` | `#d95926` | segunda serie (límite de 7 días) |
| `--diff-add` / `--diff-del` | fondos `positive` y `critical` suaves | — | líneas añadidas y borradas en los diffs |

Los colores de estado **nunca** se usan como series.

### 2.4 Tipografía

Solo fuentes del sistema (sin descargas): **Segoe UI Variable** (texto y títulos, incluida en Windows 11) y **Cascadia Mono** para identificadores, rutas, hashes y código.

| Token | Tamaño / interlineado | Peso | Uso |
|---|---|---|---|
| `--fs-label` | 11 px / 16, versalitas, +0,06 em | 600 | etiquetas de sección, cabeceras de tabla |
| `--fs-meta` | 12 px / 16 | 400 | metadatos, ayudas |
| `--fs-body` | 13 px / 20 | 400 | texto por defecto, filas |
| `--fs-strong` | 14 px / 20 | 600 | títulos de tarjeta, nombres |
| `--fs-title` | 18 px / 24 | 600 | título de vista |
| `--fs-metric` | 22 px / 28 | 600, tabular | cifras de las métricas |
| `--fs-hero` | 28 px / 34 | 600, tabular | cifras principales del histórico |

Todas las cifras llevan `font-variant-numeric: tabular-nums`.

### 2.5 Espacio, forma y profundidad

- **Espacio:** `--sp-1` 4 px · `--sp-2` 8 · `--sp-3` 12 · `--sp-4` 16 · `--sp-5` 20 · `--sp-6` 24 · `--sp-8` 32. Margen de la vista 24 px (16 px en ventanas estrechas).
- **Radios:** `--r-xs` 4 px (etiquetas) · `--r-sm` 6 px (botones, campos, selectores) · `--r-md` 8 px (pozos) · `--r-lg` 12 px (tarjetas, paneles).
- **Bordes:** 1 px siempre. El **filo de estado** (3 px a la izquierda) solo lo llevan las entidades vivas: sesiones y subagentes.
- **Elevación:** nivel 0 lienzo; nivel 1 superficie (borde + `--shadow-1`, casi imperceptible en claro, nula en oscuro); nivel 2 elevado (`--shadow-2`: avisos emergentes, menús, confirmaciones).

### 2.6 Movimiento

`--dur-fast` 120 ms (hover, foco) · `--dur-base` 180 ms (aparición, cambio de estado) · curva `cubic-bezier(0.2, 0, 0, 1)`. El único movimiento continuo es el **pulso** del punto «en curso» (1,6 s). Con `prefers-reduced-motion: reduce`, todo se queda quieto.

### 2.7 Foco

`:focus-visible` dibuja un anillo de 2 px en `--accent` separado 2 px del control. Nunca se quita sin sustituirlo.

## 3. Semántica de estado

Una sola tabla para toda la app. Si algo no encaja aquí, no lleva color.

| Tono | Significa | Ejemplos |
|---|---|---|
| **accent** (azul) | actividad en curso, selección, interacción | sesión «Trabajando», herramienta en curso, paso del plan en curso, pestaña activa, botón principal |
| **positive** (verde) | sano, terminado, conectado | herramienta «ok», tarea completada, mod conectado, agregación al día, «Viva» en espera |
| **attention** (ámbar) | necesita mirarse, sin ser un fallo | esperando permiso, esperando respuesta, sin mod, días por agregar, bloqueos, cambio masivo |
| **critical** (rojo) | fallo | herramienta con error, error de la API, núcleo sin conexión, días que no cuadran, Git sin responder |
| **neutral** (gris) | inactivo o sin dato | sin proceso, interrumpida, desconocido, pendiente |

Cada tono aparece en tres formas, siempre con texto o icono: **punto** (`StatusDot`, 8 px, con pulso si es «en curso»), **insignia** (`Badge`, fondo suave + texto del tono) y **filo** de las entidades vivas.

**Quién hizo un cambio** (Cambios en directo) no es un estado: Claude va en azul (`accent`, sólido con confianza alta, contorno con media o baja) y lo que no es de Claude en gris (`neutral`). El texto dice siempre quién y con qué confianza.

## 4. Iconografía

Juego propio en `src/ui/icons.tsx`: SVG de 16 px (20 en la navegación), trazo de 1,5 px, extremos redondeados, `currentColor`, sin rellenos salvo los puntos. Un icono acompaña a una palabra; cuando va solo (barra lateral plegada, botones de icono) lleva `aria-label` y tooltip.

**Marca:** un dial de instrumento (arco de medidor con aguja) en `--accent`. Es el icono de la ventana, de la bandeja y del instalador (`src-tauri/icons`, generados desde `src-tauri/icons/source.svg`).

## 5. Estructura de la ventana

```
┌────────────┬─────────────────────────────────────────────┐
│ ◔ DEV      │ Título de la vista          [acciones]      │
│   COCKPIT  │ resumen de una línea                        │
│            ├─────────────────────────────────────────────┤
│ ▸ En vivo 2│ avisos globales (error del núcleo, lectura   │
│ ▸ Tareas   │ del historial)                              │
│ ▸ Cambios 1│                                             │
│ ▸ Mapa     │ contenido de la vista (se desplaza)         │
│ ▸ Histórico│                                             │
│ ▸ Diagnóst.│                                             │
│            │                                             │
│ ● Núcleo   │                                             │
│   conectado│                                             │
└────────────┴─────────────────────────────────────────────┘
```

- **Barra lateral** (220 px; 64 px, solo iconos con tooltip, por debajo de 1.000 px de ancho): marca, navegación con contadores y, abajo, el estado del núcleo (conectado, conectando, sin conexión) y de la lectura del historial.
- **Cabecera de vista:** título, resumen de una línea y las acciones de la vista (Actualizar, periodo, proyecto).
- **Avisos globales** debajo de la cabecera; los de una vista, dentro de ella.
- El contenido usa rejillas de tarjetas que se reorganizan solas (mínimo 340 px por columna).

## 6. Componentes compartidos

En `src/ui/` (`components.tsx`, `components.css`). Las vistas no definen botones, insignias, tablas ni avisos propios.

| Componente | Variantes |
|---|---|
| `Button` | `primary`, `secondary`, `ghost`, `danger`; `sm`/`md`; con icono |
| `Segmented` | selector de una opción (periodo, proyecto); con contador |
| `Badge` | tonos de §3; `soft`, `outline`; con punto |
| `StatusDot` | tonos de §3; `pulse` |
| `Card` | cabecera (título, subtítulo, acciones), filo de estado, cuerpo |
| `SectionLabel` | etiqueta de sección con contador opcional |
| `Metric` | etiqueta, cifra, nota; tono opcional para la nota |
| `PropertyList` / `Property` | pares etiqueta-valor (Diagnóstico) |
| `DataTable` (`.ui-table`) | cabecera fija, cifras a la derecha, filas compactas |
| `Notice` | `info`, `attention`, `critical`, `positive`; icono, título, acciones, progreso |
| `EmptyState` | icono, título, explicación, acción |
| `Loading` / `Skeleton` | indicador con texto; líneas de espera |
| `ProgressBar` | tono; `sm` |
| `Switch`, `NumberField`, `Field` | ajustes con etiqueta y ayuda |
| `ConfirmBar` | confirmación en línea de una acción irreversible |
| `Sparkline` | actividad por minuto |
| `Meter` | uso 0–100 % (contexto, límites) con umbrales de tono |
| `LiveEmpty` (`src/app/Shell.tsx`) | vacío de las vistas que llena el núcleo: distingue conectando, sin conexión y vacío |

Los gráficos se dibujan a escala 1:1: el ancho del `viewBox` es el ancho real del contenedor (`useWidth`), de modo que el texto mide siempre 11 px y las barras no pasan de 28 px de ancho, con 2 px de separación.

## 7. Estados de cada vista

Toda vista resuelve, con los componentes de §6:

- **Vacío:** `EmptyState` con icono, qué falta y cuándo aparecerá. En las vistas que llena el núcleo, `LiveEmpty`: mientras conecta muestra `Loading` y sin conexión dice «Sin datos del núcleo», nunca «no hay nada».
- **Cargando:** `Loading` (o `Skeleton` donde la forma final es conocida).
- **Error:** `Notice` `critical` con el mensaje y, si tiene sentido, «Reintentar». Las tarjetas que dependen de esa lectura dicen «Sin datos» en vez de quedarse vacías.
- **Sin conexión con el núcleo:** estado en la barra lateral (`critical`) y aviso global; los datos ya pintados siguen visibles.
- **Recuperación:** al reconectar, el aviso desaparece solo y el estado vuelve a `positive`.

## 8. Aplicación por vista

| Vista | Cómo se aplica |
|---|---|
| **En vivo** | Tarjeta por sesión con filo de estado; cabecera con proyecto, estado (punto + insignia) y motor; fila de métricas; medidores de contexto y límites; progreso declarado; herramientas en curso arriba y últimas debajo; pie con mod y última actividad. |
| **Tareas** | Tarjeta por sesión; cada lista con su fuente, barra de progreso y pasos (icono de estado, en curso resaltado, bloqueos en ámbar); árbol de subagentes con punto y duración; actividad separada con `Sparkline`. |
| **Cambios en directo** | Selector de proyecto; panel con Git en metadatos (rama, adelanto, `HEAD`, ficheros); lista en un pozo con grupos por herramienta; columna de ficheros con cambios y visor de diff con líneas coloreadas. |
| **Mapa** | Selector de proyecto; árbol en un pozo con cifras alineadas y calor de hoy en ámbar; componentes por ecosistema; grafo con nodos neutros y grupos en `accent`. |
| **Histórico** | Periodo en la cabecera; métricas grandes; gráficos con la paleta de datos (§2.3), eje discreto, tooltip y vista de tabla; tablas compactas. |
| **Diagnóstico** | Tarjetas de propiedades con insignias de estado; «Privacidad y retención» con interruptores, campo numérico y botones (`primary`, `secondary`, `danger` con `ConfirmBar`). |

## 9. Tamaños de ventana

- **≥ 1.200 px:** barra lateral completa, rejillas de 2–3 columnas.
- **1.000–1.200 px:** barra completa, rejillas de 2 columnas.
- **< 1.000 px:** barra de iconos (64 px); paneles de dos columnas pasan a una.
- **< 640 px:** márgenes de 16 px, métricas en 2 columnas, tablas con desplazamiento horizontal propio (nunca la página entera).

En una fila con insignias (herramientas, ficheros), la ruta es lo primero que cede sitio y se recorta por la izquierda; las insignias y los tiempos no se recortan ni se salen de la tarjeta.

## 10. Accesibilidad

- Contraste comprobado de todos los pares de texto/fondo en los dos temas: texto ≥ 4,5:1 (mínimo medido 4,80) y `--text-faint` ≥ 3:1, solo para lo no esencial.
- El estado nunca depende solo del color (icono o palabra siempre).
- Navegación por teclado con foco visible; los iconos solos llevan nombre accesible.
- Las gráficas tienen tooltip, etiqueta accesible y vista de tabla.
- `prefers-reduced-motion` detiene el pulso y las transiciones.
