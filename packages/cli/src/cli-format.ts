import { formatCell, mdTable } from "@dbx-app/node-core";
import type { ConnectionConfig } from "@dbx-app/node-core";

export { formatCell, mdTable };

export function connectionSummary(connection: ConnectionConfig) {
  return {
    id: connection.id,
    name: connection.name,
    type: connection.db_type,
    host: connection.host,
    port: connection.port,
    database: connection.database || undefined,
  };
}

export function errorPayload(code: string, message: string) {
  const hint = errorHint(code, message);
  return { error: hint ? { code, message, hint } : { code, message } };
}

export function formatErrorMessage(code: string, message: string) {
  const hint = errorHint(code, message);
  return hint ? `${message}\n\nHint: ${hint}` : message;
}

function errorHint(code: string, message: string): string | undefined {
  if (code === "CONNECTION_STORE_ERROR" && /NODE_MODULE_VERSION|compiled against a different Node\.js version/i.test(message)) {
    return "Rebuild DBX CLI native dependencies with your active Node.js: pnpm rebuild better-sqlite3 keytar --pending, or reinstall the package with the same Node.js version you use to run dbx.";
  }
  return undefined;
}

export function csvTable<T extends Record<string, unknown>>(headers: string[], rows: T[] | Record<string, unknown>[]) {
  const lines = [headers.map(csvCell).join(",")];
  for (const row of rows) {
    const values = row as Record<string, unknown>;
    lines.push(headers.map((header) => csvCell(values[header])).join(","));
  }
  return `${lines.join("\n")}\n`;
}

function csvCell(value: unknown) {
  if (value === null || value === undefined) return "";
  const text = typeof value === "object" ? JSON.stringify(value) : String(value);
  if (/[",\r\n]/.test(text)) return `"${text.replace(/"/g, '""')}"`;
  return text;
}
