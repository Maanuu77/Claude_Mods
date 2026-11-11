// Contrato del stream En vivo. Es el JSON que serializa Rust
// (`cockpit_ingest::live::LiveDelta`), con los nombres en snake_case.

export interface TokenUsage {
  input: number;
  output: number;
  cache_read: number;
  cache_creation: number;
}

export interface ToolView {
  tool_use_id: string;
  tool: string;
  agent_id: string | null;
  status: "running" | "ok" | "error" | "interrupted" | "unknown";
  started_at: number;
  duration_ms: number | null;
  rel_path: string | null;
}

export interface Liveness {
  pid: number;
  status: string | null;
  kind: string | null;
  updated_at: number | null;
}

export interface SessionView {
  id: string;
  cwd: string | null;
  entrypoint: string | null;
  engine_version: string | null;
  started_at: number;
  last_seen_at: number;
  model: string | null;
  usage: TokenUsage;
  api_messages: number;
  tool_calls: number;
  tool_errors: number;
  api_errors: number;
  cost_state_usd: number | null;
  running_tools: ToolView[];
  recent_tools: ToolView[];
  /** Presente si el proceso de la sesión está vivo. */
  live: Liveness | null;
}

export interface BackfillState {
  running: boolean;
  files_done: number;
  files_total: number;
  bytes_done: number;
  bytes_total: number;
}

export type LiveDelta =
  | { type: "snapshot"; sessions: SessionView[]; backfill: BackfillState; generated_at: number }
  | {
      type: "update";
      upserts: SessionView[];
      removed: string[];
      backfill: BackfillState | null;
      generated_at: number;
    };
