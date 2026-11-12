# dev-cockpit-bridge

Mod de Claude Code que hace de puente entre una sesión y la app de escritorio DEV COCKPIT. Envía metadatos de la sesión a la app, que escucha solo en `127.0.0.1`. La arquitectura está en [`docs/SPEC.md`](../../docs/SPEC.md) §5.

## Qué hace

- Avisa a la app cuando empieza y cuando termina cada herramienta, para que se vea en curso al instante.
- Avisa de que una herramienta está esperando permiso: lee el veredicto de permisos del motor (`tool.check`) y lo devuelve sin tocarlo.
- Envía el final de cada turno y el uso de la sesión (contexto, límites y coste) cuando el motor lo publica.
- Muestra un indicador en la línea de estado (`◉ cockpit` conectado, `○ cockpit` sin conexión) y el comando `/cockpit` con el diagnóstico.

## Qué no hace

- **No bloquea ni reescribe nada.** Todos los hooks devuelven lo que devuelve el motor.
- **No envía argumentos ni resultados de herramientas.** Solo la lista blanca de SPEC §10: una ruta relativa al proyecto, el host de una URL y unos pocos indicadores.
- **No envía rutas hasta que la app lo autoriza.** Antes del primer handshake aplica la política más restrictiva.
- **No afecta a la sesión si la app está cerrada.** Guarda hasta 2.000 eventos en memoria y comprueba cada 15 s si la app ha aparecido.

## Uso durante el desarrollo

El mod se carga sin instalarlo, con `--plugin-dir`:

```text
claude --plugin-dir mods/dev-cockpit-bridge
```

La instalación permanente modifica `~/.claude` y requiere autorización expresa (SPEC §5.3).

## Comprobaciones

```text
npx tsc -p compat/tsconfig.2.1.289.json
npx tsc -p compat/tsconfig.2.1.317.json
npx tsc -p compat/tsconfig.2.1.321.json
claude plugin validate mods/dev-cockpit-bridge
```

Todo el mod está en `hooks/register.ts`. La lista blanca de metadatos de ese fichero es un espejo de `crates/cockpit-core/src/privacy.rs`: si cambia una, cambia la otra.
