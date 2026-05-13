<script setup lang="ts">
import { computed, ref, onBeforeUnmount, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { Copy, Eye, Trash2, Save, RefreshCw, Plus, Loader2, Pencil } from "lucide-vue-next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Sheet, SheetContent, SheetFooter, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import DangerConfirmDialog from "@/components/editor/DangerConfirmDialog.vue";
import * as api from "@/lib/api";
import type { RedisValue } from "@/lib/api";
import {
  canEditRedisMemberDetail,
  clampRedisMemberDetailSheetWidth,
  formatRedisMemberDetail,
  getRedisMemberSelectionKey,
  highlightRedisJsonDetail,
} from "@/lib/redisValuePresentation";

const { t } = useI18n();

const props = defineProps<{
  connectionId: string;
  db: number;
  keyDisplay: string;
  keyRaw: string;
}>();

const emit = defineEmits<{ deleted: [] }>();

const data = ref<RedisValue | null>(null);
const loading = ref(false);
const loadingMore = ref(false);
const editValue = ref("");
const isEditing = ref(false);
const newField = ref("");
const newValue = ref("");
const newScore = ref("");
const showDeleteConfirm = ref(false);
const showMemberDetail = ref(false);
const editingTtl = ref(false);
const ttlInput = ref("");
const collectionItems = ref<any[]>([]);
const scanCursor = ref<number | undefined>(undefined);
const selectedMemberTitle = ref("");
const selectedMemberRaw = ref<unknown>("");
const selectedMemberKey = ref("");
const selectedMemberContext = ref<RedisMemberContext | null>(null);
const isEditingMember = ref(false);
const savingMember = ref(false);
const memberEditValue = ref("");
const memberDetailSheetWidth = ref(420);
const isResizingMemberSheet = ref(false);
const selectedMemberDetail = computed(() => formatRedisMemberDetail(selectedMemberRaw.value));
const selectedMemberJsonHtml = computed(() =>
  selectedMemberDetail.value.format === "json" ? highlightRedisJsonDetail(selectedMemberDetail.value.text) : "",
);
const selectedMemberCanEdit = computed(
  () => selectedMemberContext.value != null && canEditRedisMemberDetail(selectedMemberContext.value.kind),
);

type PendingDelete =
  | { kind: "key" }
  | { kind: "hash"; field: string }
  | { kind: "list"; index: number }
  | { kind: "set"; member: string }
  | { kind: "zset"; member: string };

const pendingDelete = ref<PendingDelete | null>(null);

let memberSheetResizeStartX = 0;
let memberSheetResizeStartWidth = 0;

type RedisMemberContext =
  | { kind: "list"; index: number }
  | { kind: "set"; member: string }
  | { kind: "hash"; field: string }
  | { kind: "zset"; member: string; score: number }
  | { kind: "stream"; field: string };

const deleteDetails = computed(() => {
  const pending = pendingDelete.value;
  if (!pending) return "";
  if (pending.kind === "key") return t("dangerDialog.redisKeyDetails", { key: props.keyDisplay });
  if (pending.kind === "hash")
    return t("dangerDialog.redisHashFieldDetails", { key: props.keyDisplay, field: pending.field });
  if (pending.kind === "list")
    return t("dangerDialog.redisListItemDetails", { key: props.keyDisplay, index: pending.index });
  if (pending.kind === "zset")
    return t("dangerDialog.redisSetMemberDetails", { key: props.keyDisplay, member: pending.member });
  return t("dangerDialog.redisSetMemberDetails", { key: props.keyDisplay, member: pending.member });
});

const isBinaryStringValue = computed(() => data.value?.key_type === "string" && data.value?.value_is_binary);
const hasMore = computed(() => scanCursor.value != null && scanCursor.value > 0);

async function load(options: { selectDefaultMember?: boolean } = {}) {
  const shouldSelectDefaultMember = options.selectDefaultMember ?? true;
  loading.value = true;
  try {
    data.value = await api.redisGetValue(props.connectionId, props.db, props.keyRaw);
    scanCursor.value = data.value.scan_cursor ?? undefined;
    if (data.value.key_type === "string") {
      editValue.value = String(data.value.value);
      clearSelectedMember();
    } else if (["list", "set", "zset", "hash"].includes(data.value.key_type)) {
      collectionItems.value = Array.isArray(data.value.value) ? [...data.value.value] : [];
      if (shouldSelectDefaultMember) selectDefaultMember(data.value);
    } else if (data.value.key_type === "stream") {
      if (shouldSelectDefaultMember) selectDefaultMember(data.value);
    } else {
      clearSelectedMember();
    }
  } finally {
    loading.value = false;
  }
}

async function loadMore() {
  if (!data.value || !hasMore.value || loadingMore.value) return;
  loadingMore.value = true;
  try {
    const result = await api.redisLoadMore(
      props.connectionId,
      props.db,
      props.keyRaw,
      data.value.key_type,
      scanCursor.value!,
      200,
    );
    const newItems = Array.isArray(result.value) ? result.value : [];
    collectionItems.value = [...collectionItems.value, ...newItems];
    scanCursor.value = result.scan_cursor ?? undefined;
  } finally {
    loadingMore.value = false;
  }
}

async function saveString() {
  if (isBinaryStringValue.value) return;
  await api.redisSetString(props.connectionId, props.db, props.keyRaw, editValue.value);
  isEditing.value = false;
  await load();
}

function handleStringInput() {
  if (!isBinaryStringValue.value) {
    isEditing.value = true;
  }
}

async function applyDeleteKey() {
  await api.redisDeleteKey(props.connectionId, props.db, props.keyRaw);
  emit("deleted");
}

function requestDeleteKey() {
  pendingDelete.value = { kind: "key" };
  showDeleteConfirm.value = true;
}

function copyValue() {
  if (!data.value) return;
  const text = typeof data.value.value === "string" ? data.value.value : JSON.stringify(data.value.value, null, 2);
  navigator.clipboard.writeText(text);
  toast(t("redis.copied"), 2000);
}

function copyText(text: string) {
  navigator.clipboard.writeText(text);
  toast(t("redis.copied"), 2000);
}

function copyMember(value: unknown) {
  copyText(formatRedisMemberDetail(value).text);
}

function selectMember(title: string, value: unknown, context: RedisMemberContext) {
  selectedMemberTitle.value = title;
  selectedMemberRaw.value = value;
  selectedMemberKey.value = getRedisMemberSelectionKey(title, value);
  selectedMemberContext.value = context;
  isEditingMember.value = false;
  memberEditValue.value = formatRedisMemberDetail(value).text;
}

function clearSelectedMember() {
  selectedMemberTitle.value = "";
  selectedMemberRaw.value = "";
  selectedMemberKey.value = "";
  selectedMemberContext.value = null;
  isEditingMember.value = false;
  memberEditValue.value = "";
}

function isSelectedMember(title: string, value: unknown) {
  return selectedMemberKey.value === getRedisMemberSelectionKey(title, value);
}

function viewMember(title: string, value: unknown, context: RedisMemberContext) {
  selectMember(title, value, context);
  showMemberDetail.value = true;
}

function handleMemberDetailOpenChange(open: boolean) {
  showMemberDetail.value = open;
  if (!open) isEditingMember.value = false;
}

function finishMemberDetailClose() {
  isEditingMember.value = false;
}

function stopResizeMemberSheet() {
  isResizingMemberSheet.value = false;
  window.removeEventListener("pointermove", resizeMemberSheet);
  window.removeEventListener("pointerup", stopResizeMemberSheet);
}

function resizeMemberSheet(event: PointerEvent) {
  if (!isResizingMemberSheet.value) return;
  const delta = memberSheetResizeStartX - event.clientX;
  memberDetailSheetWidth.value = clampRedisMemberDetailSheetWidth(
    memberSheetResizeStartWidth + delta,
    window.innerWidth,
  );
}

function startResizeMemberSheet(event: PointerEvent) {
  isResizingMemberSheet.value = true;
  memberSheetResizeStartX = event.clientX;
  memberSheetResizeStartWidth = memberDetailSheetWidth.value;
  window.addEventListener("pointermove", resizeMemberSheet);
  window.addEventListener("pointerup", stopResizeMemberSheet);
}

function startEditMember() {
  memberEditValue.value = selectedMemberDetail.value.text;
  isEditingMember.value = true;
}

function cancelEditMember() {
  memberEditValue.value = selectedMemberDetail.value.text;
  isEditingMember.value = false;
}

async function saveMemberEdit() {
  const context = selectedMemberContext.value;
  if (!context || !selectedMemberCanEdit.value) return;
  const title = selectedMemberTitle.value;
  let nextContext: RedisMemberContext = context;
  savingMember.value = true;
  try {
    if (context.kind === "list") {
      await api.redisListSet(props.connectionId, props.db, props.keyRaw, context.index, memberEditValue.value);
    } else if (context.kind === "hash") {
      await api.redisHashSet(props.connectionId, props.db, props.keyRaw, context.field, memberEditValue.value);
    } else if (context.kind === "set") {
      await api.redisSetRemove(props.connectionId, props.db, props.keyRaw, context.member);
      await api.redisSetAdd(props.connectionId, props.db, props.keyRaw, memberEditValue.value);
      nextContext = { kind: "set", member: memberEditValue.value };
    } else if (context.kind === "zset") {
      await api.redisZrem(props.connectionId, props.db, props.keyRaw, context.member);
      await api.redisZadd(props.connectionId, props.db, props.keyRaw, memberEditValue.value, context.score);
      nextContext = { kind: "zset", member: memberEditValue.value, score: context.score };
    }
    const editedValue = memberEditValue.value;
    isEditingMember.value = false;
    await load({ selectDefaultMember: false });
    selectMember(title, editedValue, nextContext);
  } finally {
    savingMember.value = false;
  }
}

function selectDefaultMember(redisValue: RedisValue) {
  if (redisValue.key_type === "list" || redisValue.key_type === "set") {
    if (collectionItems.value.length === 0) {
      clearSelectedMember();
      return;
    }
    selectMember(
      redisValue.key_type === "list" ? "#0" : t("redis.member"),
      collectionItems.value[0],
      redisValue.key_type === "list"
        ? { kind: "list", index: 0 }
        : { kind: "set", member: String(collectionItems.value[0]) },
    );
    return;
  }

  if (redisValue.key_type === "hash") {
    const first = collectionItems.value[0];
    if (!first) {
      clearSelectedMember();
      return;
    }
    selectMember(String(first.field), first.value, { kind: "hash", field: String(first.field) });
    return;
  }

  if (redisValue.key_type === "zset") {
    const first = collectionItems.value[0];
    if (!first) {
      clearSelectedMember();
      return;
    }
    selectMember(String(first.score), first.member, {
      kind: "zset",
      member: String(first.member),
      score: Number(first.score),
    });
    return;
  }

  if (redisValue.key_type === "stream" && Array.isArray(redisValue.value)) {
    const firstEntry = redisValue.value[0];
    const firstField = firstEntry?.fields ? Object.entries(firstEntry.fields)[0] : undefined;
    if (!firstField) {
      clearSelectedMember();
      return;
    }
    selectMember(String(firstField[0]), firstField[1], { kind: "stream", field: String(firstField[0]) });
    return;
  }

  clearSelectedMember();
}

// TTL
function startEditTtl() {
  if (!data.value) return;
  ttlInput.value = data.value.ttl > 0 ? String(data.value.ttl) : "";
  editingTtl.value = true;
}

async function saveTtl() {
  const val = ttlInput.value.trim();
  const ttl = val === "" || val === "-1" ? -1 : parseInt(val, 10);
  if (isNaN(ttl)) return;
  await api.redisSetTtl(props.connectionId, props.db, props.keyRaw, ttl);
  editingTtl.value = false;
  await load();
}

function cancelEditTtl() {
  editingTtl.value = false;
}

// Hash
async function hashSet() {
  if (!newField.value) return;
  await api.redisHashSet(props.connectionId, props.db, props.keyRaw, newField.value, newValue.value);
  newField.value = "";
  newValue.value = "";
  await load();
}
async function applyHashDel(field: string) {
  await api.redisHashDel(props.connectionId, props.db, props.keyRaw, field);
  await load();
}
function requestHashDel(field: string) {
  pendingDelete.value = { kind: "hash", field };
  showDeleteConfirm.value = true;
}

// List
async function listPush() {
  if (!newValue.value) return;
  await api.redisListPush(props.connectionId, props.db, props.keyRaw, newValue.value);
  newValue.value = "";
  await load();
}
async function applyListRemove(index: number) {
  await api.redisListRemove(props.connectionId, props.db, props.keyRaw, index);
  await load();
}
function requestListRemove(index: number) {
  pendingDelete.value = { kind: "list", index };
  showDeleteConfirm.value = true;
}

// Set
async function setAdd() {
  if (!newValue.value) return;
  await api.redisSetAdd(props.connectionId, props.db, props.keyRaw, newValue.value);
  newValue.value = "";
  await load();
}
async function applySetRemove(member: string) {
  await api.redisSetRemove(props.connectionId, props.db, props.keyRaw, member);
  await load();
}
function requestSetRemove(member: string) {
  pendingDelete.value = { kind: "set", member };
  showDeleteConfirm.value = true;
}

// ZSet
async function zsetAdd() {
  if (!newValue.value) return;
  const score = parseFloat(newScore.value || "0");
  await api.redisZadd(props.connectionId, props.db, props.keyRaw, newValue.value, score);
  newValue.value = "";
  newScore.value = "";
  await load();
}
async function applyZsetRemove(member: string) {
  await api.redisZrem(props.connectionId, props.db, props.keyRaw, member);
  await load();
}
function requestZsetRemove(member: string) {
  pendingDelete.value = { kind: "zset", member };
  showDeleteConfirm.value = true;
}

async function confirmDelete() {
  const pending = pendingDelete.value;
  if (!pending) return;
  if (pending.kind === "key") await applyDeleteKey();
  else if (pending.kind === "hash") await applyHashDel(pending.field);
  else if (pending.kind === "list") await applyListRemove(pending.index);
  else if (pending.kind === "set") await applySetRemove(pending.member);
  else if (pending.kind === "zset") await applyZsetRemove(pending.member);
  pendingDelete.value = null;
}

function formatValue(val: any): string {
  if (typeof val === "string") return val;
  return JSON.stringify(val, null, 2);
}

onMounted(load);
onBeforeUnmount(stopResizeMemberSheet);
</script>

<template>
  <div class="h-full flex flex-col overflow-hidden">
    <div v-if="loading" class="flex-1 flex items-center justify-center text-muted-foreground">
      {{ t("common.loading") }}
    </div>

    <template v-else-if="data">
      <!-- Header -->
      <div class="h-9 flex items-center gap-2 px-4 border-b bg-muted/30 shrink-0">
        <span class="font-mono text-sm font-medium truncate">{{ data.key_display }}</span>
        <Badge variant="secondary" class="text-xs">{{ data.key_type }}</Badge>
        <template v-if="!editingTtl">
          <Badge
            v-if="data.ttl > 0"
            variant="outline"
            class="text-xs cursor-pointer hover:bg-accent"
            @click="startEditTtl"
            >TTL: {{ data.ttl }}s</Badge
          >
          <Badge
            v-else-if="data.ttl === -1"
            variant="outline"
            class="text-xs opacity-50 cursor-pointer hover:bg-accent hover:opacity-100"
            @click="startEditTtl"
            >{{ t("redis.noExpiry") }}</Badge
          >
        </template>
        <div v-else class="flex items-center gap-1">
          <Input
            v-model="ttlInput"
            class="h-6 w-20 text-xs"
            placeholder="seconds (-1=no expiry)"
            autofocus
            @keydown.enter="saveTtl"
            @keydown.escape="cancelEditTtl"
          />
          <Button variant="ghost" size="icon" class="h-6 w-6" @click="saveTtl"><Save class="h-3 w-3" /></Button>
        </div>
        <span class="flex-1" />
        <Button variant="ghost" size="icon" class="h-7 w-7" @click="load"><RefreshCw class="h-3.5 w-3.5" /></Button>
        <Button variant="ghost" size="icon" class="h-7 w-7" @click="copyValue"><Copy class="h-3.5 w-3.5" /></Button>
        <Button variant="ghost" size="icon" class="h-7 w-7 text-destructive" @click="requestDeleteKey"
          ><Trash2 class="h-3.5 w-3.5"
        /></Button>
      </div>

      <!-- String -->
      <div v-if="data.key_type === 'string'" class="flex-1 flex flex-col overflow-hidden">
        <textarea
          v-model="editValue"
          class="flex-1 p-4 font-mono text-sm bg-background resize-none outline-none"
          :readonly="isBinaryStringValue"
          @input="handleStringInput"
        />
        <div v-if="isBinaryStringValue" class="px-4 py-2 border-t text-xs text-muted-foreground shrink-0">
          {{ t("redis.binaryStringReadonlyHint") }}
        </div>
        <div v-if="isEditing" class="px-4 py-2 border-t flex justify-end gap-2 shrink-0">
          <Button
            variant="ghost"
            size="sm"
            @click="
              isEditing = false;
              editValue = String(data.value);
            "
            >{{ t("grid.discard") }}</Button
          >
          <Button size="sm" @click="saveString"><Save class="w-3 h-3 mr-1" /> {{ t("grid.save") }}</Button>
        </div>
      </div>

      <!-- List -->
      <div v-else-if="data.key_type === 'list'" class="flex-1 flex flex-col overflow-hidden">
        <div class="flex items-center gap-2 px-4 py-1.5 border-b shrink-0">
          <span class="text-xs text-muted-foreground">{{
            t("redis.items", { count: collectionItems.length }) + (data.total != null ? ` / ${data.total}` : "")
          }}</span>
          <span class="flex-1" />
          <Input v-model="newValue" class="h-6 w-40 text-xs" placeholder="value" @keydown.enter="listPush" />
          <Button variant="ghost" size="sm" class="h-6 text-xs" @click="listPush"
            ><Plus class="w-3 h-3 mr-1" />Push</Button
          >
        </div>
        <div class="grid grid-cols-[60px_1fr_84px] border-b bg-muted/50 shrink-0">
          <div class="px-3 py-1 text-xs font-medium text-muted-foreground border-r">#</div>
          <div class="px-3 py-1 text-xs font-medium text-muted-foreground">Value</div>
          <div />
        </div>
        <div class="flex-1 overflow-y-auto">
          <div
            v-for="(item, idx) in collectionItems"
            :key="idx"
            class="grid grid-cols-[60px_1fr_84px] border-b text-sm font-mono hover:bg-accent/50 group cursor-pointer"
            :class="{ 'bg-accent/60': isSelectedMember(`#${idx}`, item) }"
            @click="viewMember(`#${idx}`, item, { kind: 'list', index: Number(idx) })"
          >
            <div class="px-3 py-1.5 text-xs text-muted-foreground border-r">{{ idx }}</div>
            <div class="px-3 py-1.5 truncate">{{ item }}</div>
            <div class="flex items-center justify-center gap-1">
              <Button
                variant="ghost"
                size="icon"
                class="h-5 w-5 opacity-0 group-hover:opacity-100"
                :title="t('redis.viewMember')"
                @click.stop="viewMember(`#${idx}`, item, { kind: 'list', index: Number(idx) })"
                ><Eye class="w-3 h-3"
              /></Button>
              <Button
                variant="ghost"
                size="icon"
                class="h-5 w-5 opacity-0 group-hover:opacity-100"
                :title="t('redis.copyMember')"
                @click.stop="copyMember(item)"
                ><Copy class="w-3 h-3"
              /></Button>
              <Button
                variant="ghost"
                size="icon"
                class="h-5 w-5 opacity-0 group-hover:opacity-100 text-destructive"
                @click.stop="requestListRemove(Number(idx))"
                ><Trash2 class="w-3 h-3"
              /></Button>
            </div>
          </div>
          <div v-if="hasMore" class="p-2">
            <Button variant="outline" size="sm" class="w-full h-7 text-xs" :disabled="loadingMore" @click="loadMore">
              <Loader2 v-if="loadingMore" class="w-3 h-3 mr-1.5 animate-spin" />
              {{ t("redis.loadMoreKeys") }}
            </Button>
          </div>
        </div>
      </div>

      <!-- Set -->
      <div v-else-if="data.key_type === 'set'" class="flex-1 flex flex-col overflow-hidden">
        <div class="flex items-center gap-2 px-4 py-1.5 border-b shrink-0">
          <span class="text-xs text-muted-foreground">{{
            t("redis.items", { count: collectionItems.length }) + (data.total != null ? ` / ${data.total}` : "")
          }}</span>
          <span class="flex-1" />
          <Input v-model="newValue" class="h-6 w-40 text-xs" placeholder="member" @keydown.enter="setAdd" />
          <Button variant="ghost" size="sm" class="h-6 text-xs" @click="setAdd"
            ><Plus class="w-3 h-3 mr-1" />Add</Button
          >
        </div>
        <div class="grid grid-cols-[1fr_84px] border-b bg-muted/50 shrink-0">
          <div class="px-3 py-1 text-xs font-medium text-muted-foreground">Member</div>
          <div />
        </div>
        <div class="flex-1 overflow-y-auto">
          <div
            v-for="(item, idx) in collectionItems"
            :key="idx"
            class="grid grid-cols-[1fr_84px] border-b text-sm font-mono hover:bg-accent/50 group cursor-pointer"
            :class="{ 'bg-accent/60': isSelectedMember(t('redis.member'), item) }"
            @click="viewMember(t('redis.member'), item, { kind: 'set', member: String(item) })"
          >
            <div class="px-3 py-1.5 truncate">{{ item }}</div>
            <div class="flex items-center justify-center gap-1">
              <Button
                variant="ghost"
                size="icon"
                class="h-5 w-5 opacity-0 group-hover:opacity-100"
                :title="t('redis.viewMember')"
                @click.stop="viewMember(t('redis.member'), item, { kind: 'set', member: String(item) })"
                ><Eye class="w-3 h-3"
              /></Button>
              <Button
                variant="ghost"
                size="icon"
                class="h-5 w-5 opacity-0 group-hover:opacity-100"
                :title="t('redis.copyMember')"
                @click.stop="copyMember(item)"
                ><Copy class="w-3 h-3"
              /></Button>
              <Button
                variant="ghost"
                size="icon"
                class="h-5 w-5 opacity-0 group-hover:opacity-100 text-destructive"
                @click.stop="requestSetRemove(String(item))"
                ><Trash2 class="w-3 h-3"
              /></Button>
            </div>
          </div>
          <div v-if="hasMore" class="p-2">
            <Button variant="outline" size="sm" class="w-full h-7 text-xs" :disabled="loadingMore" @click="loadMore">
              <Loader2 v-if="loadingMore" class="w-3 h-3 mr-1.5 animate-spin" />
              {{ t("redis.loadMoreKeys") }}
            </Button>
          </div>
        </div>
      </div>

      <!-- Hash -->
      <div v-else-if="data.key_type === 'hash'" class="flex-1 flex flex-col overflow-hidden">
        <div class="flex items-center gap-2 px-4 py-1.5 border-b shrink-0">
          <span class="text-xs text-muted-foreground">{{
            t("redis.fields", { count: collectionItems.length }) + (data.total != null ? ` / ${data.total}` : "")
          }}</span>
          <span class="flex-1" />
          <Input v-model="newField" class="h-6 w-24 text-xs" placeholder="field" />
          <Input v-model="newValue" class="h-6 w-32 text-xs" placeholder="value" @keydown.enter="hashSet" />
          <Button variant="ghost" size="sm" class="h-6 text-xs" @click="hashSet"
            ><Plus class="w-3 h-3 mr-1" />Set</Button
          >
        </div>
        <div class="grid grid-cols-[minmax(8rem,0.3fr)_1fr_84px] border-b bg-muted/50 shrink-0">
          <div class="px-3 py-1 text-xs font-medium text-muted-foreground border-r">Field</div>
          <div class="px-3 py-1 text-xs font-medium text-muted-foreground">Value</div>
          <div />
        </div>
        <div class="flex-1 overflow-y-auto">
          <div
            v-for="(item, idx) in collectionItems"
            :key="idx"
            class="grid grid-cols-[minmax(8rem,0.3fr)_1fr_84px] border-b text-sm font-mono hover:bg-accent/50 group cursor-pointer"
            :class="{ 'bg-accent/60': isSelectedMember(String(item.field), item.value) }"
            @click="viewMember(String(item.field), item.value, { kind: 'hash', field: String(item.field) })"
          >
            <div class="px-3 py-1.5 text-blue-500 truncate border-r">{{ item.field }}</div>
            <div class="px-3 py-1.5 truncate text-muted-foreground">{{ item.value }}</div>
            <div class="flex items-center justify-center gap-1">
              <Button
                variant="ghost"
                size="icon"
                class="h-5 w-5 opacity-0 group-hover:opacity-100"
                :title="t('redis.viewMember')"
                @click.stop="viewMember(String(item.field), item.value, { kind: 'hash', field: String(item.field) })"
                ><Eye class="w-3 h-3"
              /></Button>
              <Button
                variant="ghost"
                size="icon"
                class="h-5 w-5 opacity-0 group-hover:opacity-100"
                :title="t('redis.copyMember')"
                @click.stop="copyMember(item.value)"
                ><Copy class="w-3 h-3"
              /></Button>
              <Button
                variant="ghost"
                size="icon"
                class="h-5 w-5 opacity-0 group-hover:opacity-100 text-destructive"
                @click.stop="requestHashDel(String(item.field))"
                ><Trash2 class="w-3 h-3"
              /></Button>
            </div>
          </div>
          <div v-if="hasMore" class="p-2">
            <Button variant="outline" size="sm" class="w-full h-7 text-xs" :disabled="loadingMore" @click="loadMore">
              <Loader2 v-if="loadingMore" class="w-3 h-3 mr-1.5 animate-spin" />
              {{ t("redis.loadMoreKeys") }}
            </Button>
          </div>
        </div>
      </div>

      <!-- Sorted Set -->
      <div v-else-if="data.key_type === 'zset'" class="flex-1 flex flex-col overflow-hidden">
        <div class="flex items-center gap-2 px-4 py-1.5 border-b shrink-0">
          <span class="text-xs text-muted-foreground">{{
            t("redis.members", { count: collectionItems.length }) + (data.total != null ? ` / ${data.total}` : "")
          }}</span>
          <span class="flex-1" />
          <Input v-model="newScore" class="h-6 w-20 text-xs" placeholder="score" />
          <Input v-model="newValue" class="h-6 w-32 text-xs" placeholder="member" @keydown.enter="zsetAdd" />
          <Button variant="ghost" size="sm" class="h-6 text-xs" @click="zsetAdd"
            ><Plus class="w-3 h-3 mr-1" />Add</Button
          >
        </div>
        <div class="grid grid-cols-[100px_1fr_84px] border-b bg-muted/50 shrink-0">
          <div class="px-3 py-1 text-xs font-medium text-muted-foreground border-r">Score</div>
          <div class="px-3 py-1 text-xs font-medium text-muted-foreground">Member</div>
          <div />
        </div>
        <div class="flex-1 overflow-y-auto">
          <div
            v-for="(item, idx) in collectionItems"
            :key="idx"
            class="grid grid-cols-[100px_1fr_84px] border-b text-sm font-mono hover:bg-accent/50 group cursor-pointer"
            :class="{ 'bg-accent/60': isSelectedMember(String(item.score), item.member) }"
            @click="
              viewMember(String(item.score), item.member, {
                kind: 'zset',
                member: String(item.member),
                score: Number(item.score),
              })
            "
          >
            <div class="px-3 py-1.5 text-muted-foreground text-xs border-r">{{ item.score }}</div>
            <div class="px-3 py-1.5 truncate">{{ item.member }}</div>
            <div class="flex items-center justify-center gap-1">
              <Button
                variant="ghost"
                size="icon"
                class="h-5 w-5 opacity-0 group-hover:opacity-100"
                :title="t('redis.viewMember')"
                @click.stop="
                  viewMember(String(item.score), item.member, {
                    kind: 'zset',
                    member: String(item.member),
                    score: Number(item.score),
                  })
                "
                ><Eye class="w-3 h-3"
              /></Button>
              <Button
                variant="ghost"
                size="icon"
                class="h-5 w-5 opacity-0 group-hover:opacity-100"
                :title="t('redis.copyMember')"
                @click.stop="copyMember(item.member)"
                ><Copy class="w-3 h-3"
              /></Button>
              <Button
                variant="ghost"
                size="icon"
                class="h-5 w-5 opacity-0 group-hover:opacity-100 text-destructive"
                @click.stop="requestZsetRemove(String(item.member))"
                ><Trash2 class="w-3 h-3"
              /></Button>
            </div>
          </div>
          <div v-if="hasMore" class="p-2">
            <Button variant="outline" size="sm" class="w-full h-7 text-xs" :disabled="loadingMore" @click="loadMore">
              <Loader2 v-if="loadingMore" class="w-3 h-3 mr-1.5 animate-spin" />
              {{ t("redis.loadMoreKeys") }}
            </Button>
          </div>
        </div>
      </div>

      <!-- Stream (readonly) -->
      <div v-else-if="data.key_type === 'stream'" class="flex-1 overflow-auto">
        <div class="px-4 py-1 text-xs text-muted-foreground border-b">
          {{ t("redis.entries", { count: Array.isArray(data.value) ? data.value.length : 0 }) }}
        </div>
        <div
          v-for="entry in data.value"
          :key="entry.id"
          class="px-4 py-2 border-b text-sm font-mono hover:bg-accent/50"
        >
          <div class="mb-1 text-xs text-muted-foreground">{{ entry.id }}</div>
          <div
            v-for="(val, field) in entry.fields"
            :key="String(field)"
            class="grid grid-cols-[minmax(6rem,0.35fr)_1fr_56px] gap-3 py-0.5 group cursor-pointer"
            :class="{ 'bg-accent/60': isSelectedMember(String(field), val) }"
            @click="viewMember(String(field), val, { kind: 'stream', field: String(field) })"
          >
            <span class="truncate text-blue-500">{{ field }}</span>
            <span class="truncate text-muted-foreground">{{ val }}</span>
            <span class="flex justify-end gap-1">
              <Button
                variant="ghost"
                size="icon"
                class="h-5 w-5 opacity-0 group-hover:opacity-100"
                :title="t('redis.viewMember')"
                @click.stop="viewMember(String(field), val, { kind: 'stream', field: String(field) })"
                ><Eye class="w-3 h-3"
              /></Button>
              <Button
                variant="ghost"
                size="icon"
                class="h-5 w-5 opacity-0 group-hover:opacity-100"
                :title="t('redis.copyMember')"
                @click.stop="copyMember(val)"
                ><Copy class="w-3 h-3"
              /></Button>
            </span>
          </div>
        </div>
      </div>

      <!-- Unknown -->
      <div v-else class="flex-1 overflow-auto p-4">
        <pre class="font-mono text-sm whitespace-pre-wrap">{{ formatValue(data.value) }}</pre>
      </div>
    </template>

    <DangerConfirmDialog
      v-model:open="showDeleteConfirm"
      :message="t('dangerDialog.deleteMessage')"
      :details="deleteDetails"
      :confirm-label="t('dangerDialog.deleteConfirm')"
      @confirm="confirmDelete"
    />

    <Sheet :open="showMemberDetail" @update:open="handleMemberDetailOpenChange">
      <SheetContent
        side="right"
        class="gap-0 p-0 sm:max-w-[calc(100vw-2rem)]"
        :class="{ 'select-none': isResizingMemberSheet }"
        :style="{ width: `${memberDetailSheetWidth}px`, maxWidth: 'calc(100vw - 2rem)' }"
        @close-auto-focus="finishMemberDetailClose"
        @pointer-down-outside.prevent
        @interact-outside.prevent
      >
        <div
          class="absolute inset-y-0 left-0 z-10 w-2 -translate-x-1 cursor-col-resize border-l border-transparent hover:border-primary/60"
          @pointerdown.prevent="startResizeMemberSheet"
        />
        <SheetHeader class="border-b px-5 py-4 pr-12">
          <SheetTitle class="flex items-center gap-2">
            <span class="truncate">{{ selectedMemberTitle || t("redis.memberDetail") }}</span>
            <Badge variant="outline" class="shrink-0 text-xs">{{ selectedMemberDetail.format.toUpperCase() }}</Badge>
          </SheetTitle>
        </SheetHeader>
        <textarea
          v-if="isEditingMember"
          v-model="memberEditValue"
          class="min-h-0 flex-1 resize-none bg-background p-5 font-mono text-[13px] leading-6 outline-none"
          spellcheck="false"
        />
        <pre
          v-else-if="selectedMemberDetail.format === 'json'"
          class="json-viewer min-h-0 flex-1 overflow-auto bg-background p-5 font-mono text-[13px] leading-6"
          v-html="selectedMemberJsonHtml"
        />
        <pre
          v-else
          class="min-h-0 flex-1 overflow-auto bg-background p-5 font-mono text-[13px] leading-6 whitespace-pre-wrap break-words"
          >{{ selectedMemberDetail.text }}</pre
        >
        <SheetFooter class="shrink-0 border-t px-5 py-3">
          <template v-if="isEditingMember">
            <Button variant="ghost" :disabled="savingMember" @click="cancelEditMember">
              {{ t("grid.discard") }}
            </Button>
            <Button :disabled="savingMember" @click="saveMemberEdit">
              <Loader2 v-if="savingMember" class="h-4 w-4 animate-spin" />
              <Save v-else class="h-4 w-4" />
              {{ t("grid.save") }}
            </Button>
          </template>
          <Button v-else-if="selectedMemberCanEdit" variant="outline" @click="startEditMember">
            <Pencil class="h-4 w-4" />
            {{ t("redis.editMember") }}
          </Button>
          <Button variant="outline" @click="copyText(selectedMemberDetail.text)">
            <Copy class="h-4 w-4" />
            {{ t("redis.copyMember") }}
          </Button>
        </SheetFooter>
      </SheetContent>
    </Sheet>
  </div>
</template>

<style scoped>
.json-viewer {
  tab-size: 2;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

:deep(.json-key) {
  color: #7c3aed;
  font-weight: 600;
}

:deep(.json-string) {
  color: #15803d;
}

:deep(.json-number) {
  color: #b45309;
}

:deep(.json-boolean) {
  color: #2563eb;
  font-weight: 600;
}

:deep(.json-null) {
  color: #64748b;
  font-style: italic;
}

:global(.dark) :deep(.json-key) {
  color: #c4b5fd;
}

:global(.dark) :deep(.json-string) {
  color: #86efac;
}

:global(.dark) :deep(.json-number) {
  color: #fbbf24;
}

:global(.dark) :deep(.json-boolean) {
  color: #93c5fd;
}

:global(.dark) :deep(.json-null) {
  color: #94a3b8;
}
</style>
