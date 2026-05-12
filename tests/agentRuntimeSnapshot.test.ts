import { strict as assert } from "node:assert";
import test from "node:test";
import { buildAgentRuntimeSnapshot } from "../src/lib/agentRuntimeSnapshot.ts";
import { restoreStartupAgentRuntime } from "../src/lib/appStartup.ts";

test("builds runtime snapshot from active tab, selection, and limited result rows", () => {
  const snapshot = buildAgentRuntimeSnapshot({
    tabs: [
      {
        id: "tab-1",
        title: "Orders",
        connectionId: "conn-1",
        database: "sales",
        schema: "public",
        sql: "select * from orders",
        result: {
          columns: ["id"],
          rows: [[1], [2], [3]],
          affected_rows: 0,
          execution_time_ms: 12,
        },
        isExecuting: false,
        mode: "query",
      },
    ],
    activeTabId: "tab-1",
    getConnection: (id) => (id === "conn-1" ? { name: "Prod Sales" } : undefined),
    selectedSql: "select *",
    selection: {
      type: "grid-cells",
      data: { columns: ["id"], rows: [[1]] },
    },
    resultSampleLimit: 2,
  });

  assert.deepEqual(snapshot, {
    activeConnectionId: "conn-1",
    activeConnectionName: "Prod Sales",
    database: "sales",
    schema: "public",
    activeTabId: "tab-1",
    activeTabTitle: "Orders",
    sql: "select * from orders",
    selectedSql: "select *",
    selection: {
      type: "grid-cells",
      data: { columns: ["id"], rows: [[1]] },
    },
    result: {
      columns: ["id"],
      rows: [[1], [2]],
      truncated: true,
      executionTimeMs: 12,
      sampleLimit: 2,
    },
  });
});

test("omits empty selected SQL and marks empty UI selection as none", () => {
  const snapshot = buildAgentRuntimeSnapshot({
    tabs: [
      {
        id: "tab-2",
        title: "Scratch",
        connectionId: "conn-2",
        database: "",
        sql: "",
        isExecuting: false,
        mode: "query",
      },
    ],
    activeTabId: "tab-2",
    getConnection: () => ({ name: "Local" }),
    selectedSql: "   ",
    resultSampleLimit: 50,
  });

  assert.equal(snapshot.selectedSql, undefined);
  assert.deepEqual(snapshot.selection, { type: "none" });
  assert.equal(snapshot.result, undefined);
});

test("schedules startup runtime sync only after connections and restored tabs are ready", async () => {
  const calls: string[] = [];

  await restoreStartupAgentRuntime({
    initSavedSql: async () => {
      calls.push("saved-sql");
    },
    initConnections: async () => {
      calls.push("connections");
    },
    reconnectRestoredTabs: async () => {
      calls.push("reconnect");
    },
    scheduleSync: () => {
      calls.push("schedule");
    },
  });

  assert.deepEqual(calls, ["saved-sql", "connections", "reconnect", "schedule"]);
});
