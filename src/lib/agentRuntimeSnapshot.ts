import type { ConnectionConfig, QueryResult, QueryTab } from "@/types/database";

export const DEFAULT_RESULT_SAMPLE_LIMIT = 50;

export interface RuntimeResultSample {
  columns: string[];
  rows: QueryResult["rows"];
  truncated: boolean;
  executionTimeMs: number;
  sampleLimit: number;
}

export interface AgentRuntimeSnapshot {
  activeConnectionId?: string;
  activeConnectionName?: string;
  database?: string;
  schema?: string;
  activeTabId?: string;
  activeTabTitle?: string;
  sql?: string;
  selectedSql?: string;
  selection: unknown;
  result?: RuntimeResultSample;
}

export interface BuildAgentRuntimeSnapshotOptions {
  tabs: QueryTab[];
  activeTabId: string | null;
  getConnection: (connectionId: string) => Pick<ConnectionConfig, "name"> | undefined;
  selectedSql?: string;
  selection?: unknown;
  resultSampleLimit?: number;
}

export function buildAgentRuntimeSnapshot(options: BuildAgentRuntimeSnapshotOptions): AgentRuntimeSnapshot {
  const tab = options.tabs.find((item) => item.id === options.activeTabId);
  const conn = tab ? options.getConnection(tab.connectionId) : undefined;
  const selectedSql = options.selectedSql?.trim();
  const limit = options.resultSampleLimit ?? DEFAULT_RESULT_SAMPLE_LIMIT;
  const result = tab?.result
    ? {
        columns: tab.result.columns,
        rows: tab.result.rows.slice(0, limit),
        truncated: tab.result.rows.length > limit || !!tab.result.truncated,
        executionTimeMs: tab.result.execution_time_ms,
        sampleLimit: limit,
      }
    : undefined;

  return {
    activeConnectionId: tab?.connectionId,
    activeConnectionName: conn?.name,
    database: tab?.database,
    schema: tab?.schema,
    activeTabId: tab?.id,
    activeTabTitle: tab?.title,
    sql: tab?.sql,
    selectedSql: selectedSql || undefined,
    selection: options.selection ?? { type: "none" },
    result,
  };
}
