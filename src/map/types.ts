/** Espejo de `cockpit_ingest::project_scan::ProjectScan` (SPEC §9). */

export type Ecosystem = "node" | "rust" | "java" | "python" | "dotnet" | "unity" | "rojo";
export type DepScope = "prod" | "dev" | "build" | "peer" | "optional" | "module";

export interface LanguageCount {
  language: string;
  files: number;
}

export interface DirSummary {
  /** `""` es la raíz. */
  rel_dir: string;
  depth: number;
  files: number;
  bytes: number;
  languages: LanguageCount[];
}

export interface TreeSummary {
  dirs: DirSummary[];
  files: number;
  bytes: number;
  truncated: boolean;
}

export interface Component {
  id: string;
  ecosystem: Ecosystem;
  name: string;
  rel_dir: string;
  manifest: string;
  version: string | null;
  is_group: boolean;
  parts: string[];
}

export interface Dependency {
  from: string;
  name: string;
  version_req: string | null;
  scope: DepScope;
  to: string | null;
}

export interface Problem {
  manifest: string;
  reason: string;
}

export interface ProjectScan {
  source: "git" | "walk";
  tree: TreeSummary;
  map: {
    components: Component[];
    deps: Dependency[];
    problems: Problem[];
  };
  /** Los componentes salen de la caché: ningún manifiesto ha cambiado. */
  map_cached: boolean;
}
