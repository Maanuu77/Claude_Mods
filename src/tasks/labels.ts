import type { TaskListView } from "../live/types";

/** Fuente de cada lista (SPEC §8): siempre se indica de dónde sale el progreso. */
export const SOURCE_LABEL: Record<TaskListView["source"], string> = {
  task_tools: "Tareas (TaskCreate)",
  todo_write: "Lista TodoWrite",
  desktop_plan: "Plan del Desktop",
};
