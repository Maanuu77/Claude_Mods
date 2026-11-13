/**
 * Semántica de estado (docs/DESIGN.md §3): un único sitio que decide el tono de
 * cada estado de la app. Las vistas no eligen colores; preguntan aquí.
 */
import type { AgentState, TaskStatus, ToolView } from "../live/types";

export type Tone = "accent" | "positive" | "attention" | "critical" | "neutral";

export function toolTone(status: ToolView["status"]): Tone {
  switch (status) {
    case "running":
      return "accent";
    case "awaiting_permission":
      return "attention";
    case "ok":
      return "positive";
    case "error":
      return "critical";
    default:
      return "neutral";
  }
}

export const TOOL_STATUS_LABEL: Record<ToolView["status"], string> = {
  running: "en curso",
  awaiting_permission: "esperando permiso",
  ok: "terminada",
  error: "error",
  interrupted: "interrumpida",
  unknown: "sin resultado",
};

export function taskTone(status: TaskStatus): Tone {
  switch (status) {
    case "in_progress":
      return "accent";
    case "completed":
      return "positive";
    default:
      return "neutral";
  }
}

export function agentTone(status: AgentState | null): Tone {
  switch (status) {
    case "running":
    case "pending":
      return "accent";
    case "waiting":
      return "attention";
    case "completed":
    case "idle":
      return "positive";
    case "failed":
    case "killed":
      return "critical";
    default:
      return "neutral";
  }
}

/** Uso de una capacidad (contexto, límites): ámbar desde el 75 %, rojo desde el 90 %. */
export function usageTone(percent: number): Tone {
  if (percent >= 90) return "critical";
  if (percent >= 75) return "attention";
  return "accent";
}
