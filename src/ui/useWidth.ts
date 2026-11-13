import { useCallback, useRef, useState } from "react";

/**
 * Ancho en píxeles de un elemento, al día con su tamaño. Los gráficos lo usan como
 * ancho del `viewBox` para que una unidad sea un píxel y el texto no se escale con
 * la ventana (DESIGN.md §6). Devuelve una ref de callback para el contenedor.
 */
export function useWidth<T extends HTMLElement>(fallback = 720): [(el: T | null) => void, number] {
  const [width, setWidth] = useState(fallback);
  const observer = useRef<ResizeObserver | null>(null);
  const ref = useCallback((el: T | null) => {
    observer.current?.disconnect();
    observer.current = null;
    if (!el) return;
    const update = () => setWidth(Math.max(240, Math.round(el.clientWidth)));
    update();
    observer.current = new ResizeObserver(update);
    observer.current.observe(el);
  }, []);
  return [ref, width];
}
