import type { ActivityView } from "./types";

const MINUTE = 60_000;
export const ACTIVITY_MINUTES = 30;

/**
 * Herramientas por minuto en los últimos 30 minutos, alineadas con el reloj local. La
 * usan «En vivo» (estela) y «Tareas» (prismas), para que digan lo mismo.
 */
export function perMinute(activity: ActivityView, now: number): number[] {
  const nowMinute = Math.floor(now / MINUTE);
  return Array.from({ length: ACTIVITY_MINUTES }, (_, i) => {
    const minute = nowMinute - ACTIVITY_MINUTES + 1 + i;
    const index = minute - Math.floor(activity.since / MINUTE);
    return index >= 0 && index < activity.per_minute.length ? activity.per_minute[index] : 0;
  });
}
