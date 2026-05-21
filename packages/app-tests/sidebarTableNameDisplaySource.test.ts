import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";
import test from "node:test";

const source = readFileSync("apps/desktop/src/components/sidebar/TreeItem.vue", "utf8");

test("sidebar table prefix hiding is display-only", () => {
  assert.match(source, /sidebarDisplayTableName/);
  assert.match(source, /sidebarHiddenTablePrefixes/);
  assert.match(source, /node\.type === "table" \|\| node\.type === "view" \|\| node\.type === "mongo-collection"/);
  assert.match(source, /{{ visibleLabel\(node\) }}/);
  assert.match(source, /{{ displayLabel\(node\) }}/);
});
