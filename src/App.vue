<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import appLogo from "./assets/logo.png";
import defaultAvatar from "./assets/default-avatar.svg";
import {
  addItemTag,
  bootstrapApp,
  checkToutiaoLogin,
  chooseDataDirectory,
  diagnosePage,
  getUserProfile,
  launchDebugChrome,
  listSessions,
  listSyncEvents,
  listTags,
  migrateDataDirectory,
  openDownloadDir,
  openItemDir,
  openItemFile,
  removeItemTag,
  searchItemsWithType,
  setDownloadThreads,
  startSync,
  stopSync,
} from "./lib/api";
import type { AppBootstrap, ContentItem, DownloadStatusFilter, LoginStatus, PageDiagnosis, SyncEvent, SyncSession, SyncSource, TagOption, UserProfile } from "./types";

const bootstrap = ref<AppBootstrap | null>(null);
const sessions = ref<SyncSession[]>([]);
const events = ref<SyncEvent[]>([]);
const items = ref<ContentItem[]>([]);
const tagOptions = ref<TagOption[]>([]);
const profile = ref<UserProfile | null>(null);
const query = ref("");
const selectedTags = ref<string[]>([]);
const tagDrafts = ref<Record<number, string>>({});
const currentPage = ref(1);
const totalItems = ref(0);
const loading = ref(false);
const syncing = ref(false);
const loginChecking = ref(false);
const activeTab = ref<"history" | "logs" | "local">("local");
const syncSource = ref<SyncSource>("favorites");
const searchSource = ref<"all" | SyncSource>("favorites");
const selectedSessionId = ref<string>("");
const diagnosis = ref<PageDiagnosis | null>(null);
const loginStatus = ref<LoginStatus | null>(null);
const settingsOpen = ref(false);
const downloadThreads = ref(2);
const error = ref("");
let timer: number | undefined;
let searchTimer: number | undefined;
let pollTick = 0;
let refreshInFlight = false;
let lastItemRefreshAt = 0;
const ITEM_REFRESH_INTERVAL_MS = 1_000;
const PAGE_SIZE = 50;
const originalHtmlCache = new Map<string, string>();

const latestSession = computed(() => sessions.value[0] ?? null);
const effectiveSessionId = computed(() => selectedSessionId.value || latestSession.value?.id || "");
const runningSession = computed(() => sessions.value.find((session) => session.status === "running") ?? null);
const visibleProfile = computed(() => (loginStatus.value?.loggedIn ? profile.value : null));
const canSync = computed(() => Boolean(loginStatus.value?.loggedIn) && !loginChecking.value && !syncing.value);
const pageCount = computed(() => Math.max(1, Math.ceil(totalItems.value / PAGE_SIZE)));
const pageStart = computed(() => (totalItems.value ? (currentPage.value - 1) * PAGE_SIZE + 1 : 0));
const pageEnd = computed(() => Math.min(currentPage.value * PAGE_SIZE, totalItems.value));
const systemTags = new Set(["所有", "视频", "文章", "已下载", "未下载", "下载失败"]);
const semanticTagFilters = computed(() => selectedTags.value.filter((tag) => !systemTags.has(tag)));
const contentTypeFilter = computed(() => {
  const wantsVideo = selectedTags.value.includes("视频");
  const wantsArticle = selectedTags.value.includes("文章");
  if (wantsVideo && !wantsArticle) return "video";
  if (wantsArticle && !wantsVideo) return "article";
  return undefined;
});
const downloadStatusFilter = computed<DownloadStatusFilter | undefined>(() => {
  const wantsDownloaded = selectedTags.value.includes("已下载");
  const wantsPending = selectedTags.value.includes("未下载");
  const wantsFailed = selectedTags.value.includes("下载失败");
  if (wantsDownloaded && !wantsPending && !wantsFailed) return "downloaded";
  if (wantsPending && !wantsDownloaded && !wantsFailed) return "pending";
  if (wantsFailed && !wantsDownloaded && !wantsPending) return "failed";
  return undefined;
});
const renderedItems = computed(() =>
  items.value.map((item) => {
    const originalHtml = getOriginalCardHtml(item);
    return {
      item,
      originalHtml,
      originalLine: originalHtml ? "" : getOriginalLine(item),
      listImage: originalHtml ? "" : getListImage(item),
    };
  }),
);

function parseRawJson(item: ContentItem) {
  try {
    return JSON.parse(item.rawJson || "{}");
  } catch {
    return {};
  }
}

function getRawListHtml(raw: any) {
  const direct = raw?.list?.listHtml;
  if (typeof direct === "string" && direct.trim()) {
    return direct;
  }
  const nested = raw?.list?.raw?.list?.listHtml;
  return typeof nested === "string" ? nested : "";
}

function getListText(item: ContentItem) {
  const raw = parseRawJson(item);
  const html = getRawListHtml(raw);
  if (typeof html !== "string" || !html.trim()) {
    return item.summary || item.title;
  }
  const doc = new DOMParser().parseFromString(html, "text/html");
  return (doc.body.textContent || item.summary || item.title).replace(/\s+/g, " ").trim();
}

function getListImage(item: ContentItem) {
  const raw = parseRawJson(item);
  const html = getRawListHtml(raw);
  if (typeof html !== "string" || !html.trim()) {
    return item.coverUrl || "";
  }
  const doc = new DOMParser().parseFromString(html, "text/html");
  const media = doc.querySelector("video[poster], img");
  return media?.getAttribute("poster") || media?.getAttribute("src") || item.coverUrl || "";
}

function getOriginalLine(item: ContentItem) {
  const text = getListText(item);
  return text.replace(item.title, "").trim().slice(0, 80);
}

function getDownloadStatusText(item: ContentItem) {
  if (item.downloaded) {
    return "已下载";
  }
  if (item.downloadError) {
    return "下载失败";
  }
  return "未下载";
}

function getOriginalCardHtml(item: ContentItem) {
  const cacheKey = `${item.id}:${item.syncedAt}:${item.downloaded ? 1 : 0}:${item.downloadError || ""}:${item.rawJson.length}`;
  const cached = originalHtmlCache.get(cacheKey);
  if (cached !== undefined) {
    return cached;
  }
  const raw = parseRawJson(item);
  const html = getRawListHtml(raw);
  if (typeof html !== "string" || !html.trim()) {
    originalHtmlCache.set(cacheKey, "");
    return "";
  }
  const doc = new DOMParser().parseFromString(html, "text/html");
  doc.querySelectorAll("a[href]").forEach((anchor) => {
    const href = anchor.getAttribute("href") || "";
    if (href.startsWith("/")) {
      anchor.setAttribute("href", `https://www.toutiao.com${href}`);
    }
    anchor.setAttribute("target", "_blank");
    anchor.setAttribute("rel", "noreferrer");
  });
  doc.querySelectorAll("img[src]").forEach((image) => {
    const src = image.getAttribute("src") || "";
    if (src.startsWith("//")) {
      image.setAttribute("src", `https:${src}`);
    }
  });
  doc
    .querySelectorAll(".actions-list-wrapper, .profile-feed-card-tools-actions, .qrcode-panel")
    .forEach((element) => element.remove());
  doc.querySelectorAll("*").forEach((element) => {
    const text = (element.textContent || "").replace(/\s+/g, "").trim();
    if (text === "取消收藏") {
      element.remove();
    }
  });
  doc.querySelectorAll(".feed-card-footer-share-cmp").forEach((element) => {
    const shareButton = element.querySelector(".share-btn");
    if (shareButton) {
      if (shareButton.querySelector(".local-share-before-icon")) {
        shareButton.classList.add("has-local-share-icon");
      }
      const wrapper = doc.createElement("div");
      wrapper.className = "ttp-interact-share";
      wrapper.appendChild(shareButton);
      element.replaceChildren(wrapper);
    } else {
      element.replaceChildren(document.createTextNode("分享"));
    }
  });
  doc
    .querySelectorAll(".left-tools:empty, .right-tools:empty, .feed-card-footer-cmp:empty")
    .forEach((element) => element.remove());
  const footerTarget =
    doc.querySelector(".feed-card-footer-cmp .left-tools") ||
    doc.querySelector(".feed-card-footer-cmp .right-tools") ||
    doc.querySelector(".feed-card-footer-cmp");
  if (footerTarget) {
    const localMeta = doc.createElement("span");
    localMeta.className = "local-sync-meta";
    const metaItem = doc.createElement("span");
    metaItem.textContent = getDownloadStatusText(item);
    if (item.downloadError) {
      metaItem.setAttribute("title", item.downloadError);
    }
    localMeta.appendChild(metaItem);
    footerTarget.appendChild(localMeta);
  }
  const normalizedHtml = doc.body.innerHTML;
  originalHtmlCache.set(cacheKey, normalizedHtml);
  if (originalHtmlCache.size > 300) {
    const firstKey = originalHtmlCache.keys().next().value;
    if (firstKey) {
      originalHtmlCache.delete(firstKey);
    }
  }
  return normalizedHtml;
}

async function loadItems() {
  if (searchSource.value === "all") {
    items.value = [];
    totalItems.value = 0;
    currentPage.value = 1;
    lastItemRefreshAt = Date.now();
    return;
  }
  const source = searchSource.value === "likes" ? "likes" : "favorites";
  const result = await searchItemsWithType(
    query.value,
    source,
    contentTypeFilter.value,
    semanticTagFilters.value,
    downloadStatusFilter.value,
    currentPage.value,
    PAGE_SIZE,
  );
  items.value = result.items;
  totalItems.value = result.total;
  if (currentPage.value > pageCount.value) {
    currentPage.value = pageCount.value;
    await loadItems();
    return;
  }
  lastItemRefreshAt = Date.now();
}

async function loadTagOptions() {
  if (searchSource.value === "all") {
    tagOptions.value = [];
    return;
  }
  tagOptions.value = await listTags(searchSource.value);
}

async function loadEvents(sessionId?: string) {
  events.value = await listSyncEvents(sessionId || effectiveSessionId.value || undefined);
}

async function loadAll(options?: { includeItems?: boolean; includeEvents?: boolean }) {
  if (refreshInFlight) {
    return;
  }
  refreshInFlight = true;
  const includeItems = options?.includeItems ?? true;
  const includeEvents = options?.includeEvents ?? true;
  loading.value = true;
  error.value = "";
  try {
    bootstrap.value = await bootstrapApp();
    downloadThreads.value = bootstrap.value.downloadThreads || 2;
    document.title = bootstrap.value.appTitle;
    sessions.value = await listSessions();
    if (!selectedSessionId.value && sessions.value[0]) {
      selectedSessionId.value = sessions.value[0].id;
    }
    syncing.value = sessions.value.some((session) => session.status === "running");
    if (includeEvents) {
      await loadEvents();
    }
    if (includeItems) {
      await loadTagOptions();
      await loadItems();
    }
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  } finally {
    loading.value = false;
    refreshInFlight = false;
  }
}

function stopPolling() {
  if (timer) {
    window.clearInterval(timer);
    timer = undefined;
  }
}

function startPolling() {
  if (timer) {
    return;
  }
  pollTick = 0;
  timer = window.setInterval(() => {
    pollTick += 1;
    const shouldRefreshItems =
      activeTab.value === "local" &&
      (syncing.value || Date.now() - lastItemRefreshAt >= ITEM_REFRESH_INTERVAL_MS);
    void loadAll({
      includeEvents: true,
      includeItems: shouldRefreshItems,
    });
  }, 2000);
}

async function handleSync() {
  if (!canSync.value) {
    return;
  }
  syncing.value = true;
  error.value = "";
  try {
    const session = await startSync({ source: syncSource.value, mode: "list" });
    selectedSessionId.value = session.id;
    await loadAll({ includeItems: false, includeEvents: true });
  } catch (err) {
    syncing.value = false;
    error.value = err instanceof Error ? err.message : String(err);
  }
}

async function handleDownloadContent() {
  if (!canSync.value) {
    return;
  }
  syncing.value = true;
  error.value = "";
  try {
    const session = await startSync({ source: syncSource.value, mode: "download" });
    selectedSessionId.value = session.id;
    await loadAll({ includeItems: true, includeEvents: true });
  } catch (err) {
    syncing.value = false;
    error.value = err instanceof Error ? err.message : String(err);
  }
}



async function handleStopSync() {
  const session = runningSession.value;
  if (!session) {
    syncing.value = false;
    return;
  }
  error.value = "";
  try {
    await stopSync(session.id);
    syncing.value = false;
    selectedSessionId.value = session.id;
    await loadAll({ includeItems: true, includeEvents: true });
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  }
}

async function handleDiagnose() {
  error.value = "";
  try {
    diagnosis.value = await diagnosePage({ source: syncSource.value });
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  }
}

async function handleCheckLogin() {
  loginChecking.value = true;
  error.value = "";
  try {
    await launchDebugChrome();
    loginStatus.value = await checkToutiaoLogin({ source: syncSource.value });
    profile.value = loginStatus.value.loggedIn ? await getUserProfile() : null;
  } catch (err) {
    loginStatus.value = {
      loggedIn: false,
      loginRequired: true,
      source: syncSource.value,
      message: err instanceof Error ? err.message : String(err),
    };
    profile.value = null;
  } finally {
    loginChecking.value = false;
  }
}

async function handleChooseDataDirectory() {
  error.value = "";
  try {
    const result = await chooseDataDirectory();
    if (result) {
      bootstrap.value = result;
      downloadThreads.value = result.downloadThreads || 2;
      await loadAll();
    }
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  }
}

async function handleMigrateDataDirectory() {
  error.value = "";
  try {
    const result = await migrateDataDirectory();
    if (result) {
      bootstrap.value = result;
      downloadThreads.value = result.downloadThreads || 2;
      await loadAll();
    }
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  }
}

async function handleSearch() {
  error.value = "";
  try {
    activeTab.value = "local";
    await loadTagOptions();
    await loadItems();
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  }
}

async function handleSetDownloadThreads() {
  const nextValue = Math.min(Math.max(1, Math.trunc(Number(downloadThreads.value) || 2)), 8);
  downloadThreads.value = nextValue;
  error.value = "";
  try {
    bootstrap.value = await setDownloadThreads(nextValue);
    downloadThreads.value = bootstrap.value.downloadThreads || nextValue;
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  }
}

function toggleTag(tag: string) {
  currentPage.value = 1;
  if (tag === "所有") {
    selectedTags.value = [];
    void handleSearch();
    return;
  }
  const selected = new Set(selectedTags.value);
  if (selected.has(tag)) {
    selected.delete(tag);
  } else {
    if (tag === "已下载") {
      selected.delete("未下载");
    } else if (tag === "未下载") {
      selected.delete("已下载");
    }
    selected.add(tag);
  }
  selectedTags.value = Array.from(selected);
  void handleSearch();
}

function selectSearchSource(source: "all" | SyncSource) {
  activeTab.value = "local";
  searchSource.value = source;
  selectedTags.value = [];
  currentPage.value = 1;
  if (source !== "all") {
    syncSource.value = source;
  }
  void handleSearch();
}

async function goToPage(page: number) {
  const nextPage = Math.min(Math.max(1, page), pageCount.value);
  if (nextPage === currentPage.value) {
    return;
  }
  currentPage.value = nextPage;
  await loadItems();
}

async function handleAddTag(item: ContentItem) {
  const tag = (tagDrafts.value[item.id] || "").trim();
  if (!tag) {
    return;
  }
  try {
    item.tags = await addItemTag(item.id, tag);
    tagDrafts.value[item.id] = "";
    await loadTagOptions();
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  }
}

async function handleRemoveTag(item: ContentItem, tag: string) {
  try {
    item.tags = await removeItemTag(item.id, tag);
    await loadTagOptions();
    if (selectedTags.value.includes(tag)) {
      await loadItems();
    }
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  }
}

async function handleClearSearch() {
  if (!query.value) {
    return;
  }
  query.value = "";
  currentPage.value = 1;
  await handleSearch();
}

watch(effectiveSessionId, async (value) => {
  if (!value) {
    events.value = [];
    return;
  }
  try {
    await loadEvents(value);
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  }
});

watch(syncSource, () => {
  loginStatus.value = null;
  diagnosis.value = null;
  profile.value = null;
});

watch(searchSource, () => {
  selectedTags.value = [];
});

watch(query, () => {
  currentPage.value = 1;
  if (searchTimer) {
    window.clearTimeout(searchTimer);
  }
  searchTimer = window.setTimeout(() => {
    handleSearch();
  }, 250);
});

watch(syncing, async (value, oldValue) => {
  if (value) {
    startPolling();
    return;
  }
  stopPolling();
  if (oldValue) {
    try {
      await loadItems();
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
    }
  }
});

onMounted(async () => {
  await loadAll();
});

onBeforeUnmount(() => {
  stopPolling();
  if (searchTimer) {
    window.clearTimeout(searchTimer);
  }
});
</script>

<template>
  <main class="layout">
    <section class="top-toolbar">
      <img :src="appLogo" alt="今日收藏" class="toolbar-logo" />
      <select v-model="syncSource">
        <option value="favorites">同步收藏</option>
        <option value="likes">同步喜欢</option>
      </select>
      <button :disabled="!canSync" @click="handleSync">
        {{ syncing ? "同步中..." : "同步列表" }}
      </button>
      <button v-if="syncing" class="danger" @click="handleStopSync">停止同步</button>
      <button class="secondary" :disabled="!canSync" @click="handleDownloadContent">下载内容</button>
      <button class="secondary" @click="handleDiagnose">诊断当前页面</button>
      <button class="secondary" @click="activeTab = 'history'">日志</button>
      <div class="toolbar-search">
        <span class="search-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" focusable="false">
            <path d="M10.8 4.2a6.6 6.6 0 1 0 0 13.2 6.6 6.6 0 0 0 0-13.2Zm-8.6 6.6a8.6 8.6 0 1 1 15.1 5.6l4 4a1.4 1.4 0 0 1-2 2l-4-4A8.6 8.6 0 0 1 2.2 10.8Z" />
          </svg>
        </span>
        <input
          v-model="query"
          placeholder="搜索收藏内容"
          @keyup.enter="handleSearch"
        />
        <button
          v-if="query"
          class="search-clear"
          type="button"
          aria-label="清除搜索"
          title="清除搜索"
          @click="handleClearSearch"
        >
          ×
        </button>
      </div>
      <button class="icon-button" aria-label="设置" title="设置" @click="settingsOpen = !settingsOpen">⚙</button>
    </section>

    <p v-if="error" class="error">{{ error }}</p>

    <section v-if="settingsOpen" class="panel settings-panel">
      <p class="hint">
        默认数据目录是 `D:\toutiao-sync`；你也可以改目录或迁移目录。同步列表会先保存收藏/点赞列表，下载内容会再保存文章 HTML/JSON 和视频文件。
      </p>
      <div v-if="bootstrap" class="meta-grid">
        <span>版本：{{ bootstrap.buildLabel }}</span>
        <span>数据库：{{ bootstrap.dbPath }}</span>
        <span>数据目录：{{ bootstrap.dataDir }}</span>
        <span>下载目录：{{ bootstrap.downloadDir }}</span>
        <span>下载线程：{{ bootstrap.downloadThreads }}</span>
      </div>
      <label class="settings-field">
        <span>下载线程数量</span>
        <input
          v-model.number="downloadThreads"
          type="number"
          min="1"
          max="8"
          step="1"
          :disabled="syncing"
          @change="handleSetDownloadThreads"
        />
        <small>1-8，默认 2；只影响“下载内容”。</small>
      </label>
      <div class="settings-actions">
        <button class="secondary" :disabled="syncing" @click="handleChooseDataDirectory">选择数据目录</button>
        <button class="secondary" :disabled="syncing" @click="handleMigrateDataDirectory">迁移数据目录</button>
        <button class="secondary" @click="openDownloadDir">打开下载目录</button>
      </div>
    </section>

    <section class="profile-shell">
      <div class="login-status" :class="{ ok: loginStatus?.loggedIn, danger: loginStatus?.loginRequired }">
        <strong>登录状态：{{ loginChecking ? "检查中..." : loginStatus?.loggedIn ? "已登录" : "未登录" }}</strong>
        <span>{{ loginStatus?.message || "点击“刷新”会先打开 Chrome，再检查今日头条登录状态" }}</span>
        <button class="status-refresh" type="button" :disabled="loginChecking" @click="handleCheckLogin">
          {{ loginChecking ? "刷新中..." : "刷新" }}
        </button>
      </div>
      <div class="profile-banner">
        <div class="profile-avatar">
          <img :src="visibleProfile?.avatarUrl || defaultAvatar" alt="" />
        </div>
        <div class="profile-info">
          <h1>{{ visibleProfile?.name || "未登录" }}</h1>
          <div class="profile-stats">
            <span><strong>{{ visibleProfile?.likes || "-" }}</strong>获赞</span>
            <span><strong>{{ visibleProfile?.followers || "-" }}</strong>粉丝</span>
            <span><strong>{{ visibleProfile?.following || "-" }}</strong>关注</span>
          </div>
          <p>简介：{{ visibleProfile?.bio || "-" }}</p>
          <a class="profile-more" href="#">更多信息 〉</a>
        </div>
      </div>
    </section>

      <div v-if="diagnosis" class="panel diagnosis">
        <div class="section-head">
          <h2>页面诊断</h2>
          <span>{{ diagnosis.ok ? "可同步" : "需修正" }}</span>
        </div>
        <p :class="diagnosis.ok ? 'hint' : 'error'">{{ diagnosis.message }}</p>
        <div class="meta-grid">
          <span>来源：{{ diagnosis.source === "favorites" ? "收藏" : "喜欢" }}</span>
          <span>标题：{{ diagnosis.pageTitle || "无" }}</span>
          <span>地址：{{ diagnosis.pageUrl || "无" }}</span>
        </div>
        <div class="cards log-list">
          <article v-for="(log, index) in diagnosis.logs" :key="`${index}-${log}`" class="card log-card">
            <p>{{ log }}</p>
          </article>
        </div>
      </div>

    <section class="content-shell">
      <div class="site-tabs" role="tablist" aria-label="同步内容">
        <button
          class="site-tab"
          :class="{ active: activeTab === 'local' && searchSource === 'all' }"
          type="button"
          @click="selectSearchSource('all')"
        >
          全部
        </button>
        <button
          class="site-tab"
          :class="{ active: activeTab === 'local' && searchSource === 'favorites' }"
          role="tab"
          :aria-selected="activeTab === 'local' && searchSource === 'favorites'"
          @click="selectSearchSource('favorites')"
        >
          收藏
        </button>
        <button
          class="site-tab"
          :class="{ active: activeTab === 'local' && searchSource === 'likes' }"
          role="tab"
          :aria-selected="activeTab === 'local' && searchSource === 'likes'"
          @click="selectSearchSource('likes')"
        >
          喜欢
        </button>
      </div>

      <div class="tag-filter-bar" aria-label="内容标签筛选">
        <button
          v-for="tag in tagOptions"
          :key="tag.name"
          class="tag-chip"
          :class="{ active: tag.name === '所有' ? !selectedTags.length : selectedTags.includes(tag.name) }"
          type="button"
          @click="toggleTag(tag.name)"
        >
          <span>{{ tag.name }}</span>
          <small>{{ tag.count }}</small>
        </button>
      </div>

      <div class="content-panel">
        <section v-if="activeTab === 'history'" class="tab-pane history-panel">
          <div class="tabs content-tabs" role="tablist" aria-label="日志内容">
            <button
              class="tab-button"
              :class="{ active: activeTab === 'history' }"
              role="tab"
              :aria-selected="activeTab === 'history'"
              @click="activeTab = 'history'"
            >
              同步历史
            </button>
        <button
          class="tab-button"
              :class="{ active: activeTab === 'logs' }"
          role="tab"
              :aria-selected="activeTab === 'logs'"
              @click="activeTab = 'logs'"
        >
              同步日志
        </button>
      </div>
          <div class="section-head">
            <h2>同步历史</h2>
            <span v-if="loading">刷新中</span>
          </div>
          <div v-if="latestSession" class="latest">
            <strong>{{ latestSession.source === "favorites" ? "收藏" : "喜欢" }}</strong>
            <span>{{ latestSession.status }}</span>
            <span>{{ latestSession.totalCandidates }} 条候选</span>
            <span>{{ latestSession.totalSkipped }} 条跳过</span>
            <span>{{ latestSession.totalDiscovered }} 条新增</span>
            <span>{{ latestSession.totalSaved }} 条入库</span>
            <span>{{ latestSession.totalDownloaded }} 个资源</span>
          </div>
          <div class="table-wrap">
            <table class="table history-table">
              <thead>
                <tr>
                  <th>开始时间</th>
                  <th>来源</th>
                  <th>状态</th>
                  <th>候选</th>
                  <th>跳过</th>
                  <th>新增</th>
                  <th>入库</th>
                  <th>下载</th>
                  <th>消息</th>
                </tr>
              </thead>
              <tbody>
                <tr
                  v-for="session in sessions"
                  :key="session.id"
                  :class="{ selected: session.id === effectiveSessionId }"
                  @click="selectedSessionId = session.id"
                >
                  <td>{{ session.startedAt }}</td>
                  <td>{{ session.source === "favorites" ? "收藏" : "喜欢" }}</td>
                  <td>{{ session.status }}</td>
                  <td>{{ session.totalCandidates }}</td>
                  <td>{{ session.totalSkipped }}</td>
                  <td>{{ session.totalDiscovered }}</td>
                  <td>{{ session.totalSaved }}</td>
                  <td>{{ session.totalDownloaded }}</td>
                  <td class="message">{{ session.message }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>

        <section v-if="activeTab === 'logs'" class="tab-pane">
          <div class="section-head">
            <h2>同步日志</h2>
            <span>{{ effectiveSessionId ? "当前会话" : "暂无会话" }}</span>
          </div>
          <div class="cards log-list">
            <article v-for="event in events" :key="event.id" class="card log-card">
              <div class="card-head">
                <span class="badge" :class="{ danger: event.level === 'error' }">{{ event.level }}</span>
                <span>{{ event.createdAt }}</span>
              </div>
              <p>{{ event.message }}</p>
            </article>
            <p v-if="!events.length" class="hint">暂无同步日志</p>
          </div>
        </section>

        <section v-if="activeTab === 'local'" class="tab-pane">
          <div class="toutiao-list">
            <article v-for="entry in renderedItems" :key="entry.item.id" class="toutiao-item">
              <div v-if="entry.originalHtml" class="toutiao-original-card" v-html="entry.originalHtml"></div>
              <div v-else class="toutiao-body">
                <h3>{{ entry.item.title }}</h3>
                <p>{{ entry.originalLine }}</p>
              </div>
              <div v-if="!entry.originalHtml && entry.listImage" class="toutiao-thumb">
                <img :src="entry.listImage" alt="" />
                <span v-if="entry.item.contentType === 'video'" class="video-mark">▶</span>
              </div>
              <div class="toutiao-meta">
                <span v-if="!entry.originalHtml" :title="entry.item.downloadError || ''">
                  {{ getDownloadStatusText(entry.item) }}
                </span>
                <button
                  v-if="entry.item.articlePath"
                  class="link-button"
                  @click="openItemFile(entry.item.id, 'article')"
                >
                  打开文章
                </button>
                <button
                  v-if="entry.item.videoPath"
                  class="link-button"
                  @click="openItemFile(entry.item.id, 'video')"
                >
                  打开视频
                </button>
                <button class="link-button" @click="openItemDir(entry.item.id)">打开文件夹</button>
              </div>
              <div class="item-tags">
                <button
                  v-for="tag in entry.item.tags"
                  :key="`${entry.item.id}-${tag}`"
                  class="item-tag"
                  type="button"
                  title="点击删除标签"
                  @click="handleRemoveTag(entry.item, tag)"
                >
                  {{ tag }} <span>×</span>
                </button>
                <form class="tag-add" @submit.prevent="handleAddTag(entry.item)">
                  <input
                    v-model="tagDrafts[entry.item.id]"
                    type="text"
                    placeholder="添加标签"
                  />
                  <button type="submit">添加</button>
                </form>
              </div>
            </article>
            <p v-if="!renderedItems.length" class="empty-state">暂无内容</p>
          </div>
          <div v-if="totalItems > PAGE_SIZE" class="pagination" aria-label="收藏列表分页">
            <span class="page-summary">第 {{ pageStart }}-{{ pageEnd }} 条 / 共 {{ totalItems }} 条</span>
            <div class="page-actions">
              <button class="secondary" type="button" :disabled="currentPage <= 1 || loading" @click="goToPage(1)">
                首页
              </button>
              <button class="secondary" type="button" :disabled="currentPage <= 1 || loading" @click="goToPage(currentPage - 1)">
                上一页
              </button>
              <span class="page-current">{{ currentPage }} / {{ pageCount }}</span>
              <button class="secondary" type="button" :disabled="currentPage >= pageCount || loading" @click="goToPage(currentPage + 1)">
                下一页
              </button>
              <button class="secondary" type="button" :disabled="currentPage >= pageCount || loading" @click="goToPage(pageCount)">
                末页
              </button>
            </div>
          </div>
        </section>
      </div>
    </section>
  </main>
</template>

<style scoped>
.hero-logo {
  width: 64px;
  height: 64px;
  display: block;
  border-radius: 14px;
}
</style>
