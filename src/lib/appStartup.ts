export interface RestoreStartupAgentRuntimeOptions {
  initSavedSql: () => Promise<unknown>;
  initConnections: () => Promise<unknown>;
  reconnectRestoredTabs: () => Promise<unknown> | unknown;
  scheduleSync: () => void;
}

export async function restoreStartupAgentRuntime(options: RestoreStartupAgentRuntimeOptions) {
  await options.initSavedSql();
  await options.initConnections();
  await options.reconnectRestoredTabs();
  options.scheduleSync();
}
