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
  status: "running" | "awaiting_permission" | "ok" | "error" | "interrupted" | "unknown";
  started_at: number;
  duration_ms: number | null;
  rel_path: string | null;
}

export interface RateLimit {
  kind: string;
  percent_used: number;
  resets_at: string | null;
}

/** Contexto, límites y coste que publica el motor. Solo llega con el mod. */
export interface UsageView {
  ts: number;
  context_tokens: number | null;
  context_window: number | null;
  context_percent: number | null;
  rate_limits: RateLimit[];
  cost_usd: number | null;
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
  usage_snapshot: UsageView | null;
  running_tools: ToolView[];
  recent_tools: ToolView[];
  /** Presente si el proceso de la sesión está vivo. */
  live: Liveness | null;
  /** El mod de la sesión está conectado a la app. */
  mod_connected: boolean;
}

export interface BackfillState {
  running: boolean;
  files_done: number;
  files_total: number;
  bytes_done: number;
  bytes_total: number;
}

// --- Workspace Watcher (SPEC §7) -------------------------------------------

export type ChangeKind = "created" | "modified" | "deleted" | "renamed";
export type Actor = "claude" | "claude-shell" | "unknown" | "external";
export type Confidence = "high" | "medium" | "low";

/** Un cambio de fichero (`cockpit_store::FileEventView`). Nunca lleva contenido. */
export interface FileEventView {
  id: number;
  root_id: number;
  rel_path: string | null;
  from_rel_path: string | null;
  change: ChangeKind;
  size: number | null;
  ts: number;
  actor: Actor;
  session_id: string | null;
  tool_use_id: string | null;
  tool: string | null;
  confidence: Confidence;
  /** `false` durante la ventana de asentamiento de 2 s (§7.3). */
  settled: boolean;
}

export interface DirtyFile {
  rel_path: string;
  from_rel_path: string | null;
  status: string;
  staged: boolean;
  /** `null` si es binario o no está versionado. */
  added: number | null;
  deleted: number | null;
}

export interface GitView {
  branch: string | null;
  head: string | null;
  upstream: string | null;
  ahead: number | null;
  behind: number | null;
  files: DirtyFile[];
  files_total: number;
  based_on_ts: number;
  updated_at: number;
  took_ms: number;
  error: string | null;
}

export interface BulkView {
  kind: "storm" | "rescan";
  dir: string;
  events: number;
  at: number;
}

export interface WorkspaceView {
  root_id: number;
  root: string;
  name: string;
  is_git: boolean;
  is_worktree: boolean;
  watch: "active" | "network" | "too_broad" | "error";
  sessions: string[];
  git: GitView | null;
  bulk: BulkView | null;
}

export interface FileDiff {
  text: string;
  truncated: boolean;
  binary: boolean;
}

export interface FilePreview {
  text: string;
  truncated: boolean;
  binary: boolean;
  size: number;
}

export type LiveDelta =
  | {
      type: "snapshot";
      sessions: SessionView[];
      backfill: BackfillState;
      workspaces: WorkspaceView[];
      file_events: FileEventView[];
      generated_at: number;
    }
  | {
      type: "update";
      upserts: SessionView[];
      removed: string[];
      backfill: BackfillState | null;
      workspaces: WorkspaceView[];
      workspaces_removed: number[];
      file_events: FileEventView[];
      generated_at: number;
    };
