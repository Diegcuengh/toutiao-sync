<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import appLogo from "./assets/logo.png";
import {
  bootstrapApp,
  checkToutiaoLogin,
  chooseDataDirectory,
  diagnosePage,
  getUserProfile,
  launchDebugChrome,
  listSessions,
  listSyncEvents,
  migrateDataDirectory,
  openDownloadDir,
  openItemDir,
  openItemFile,
  searchItemsWithType,
  startSync,
  stopSync,
} from "./lib/api";
import type { AppBootstrap, ContentItem, ContentTypeFilter, LoginStatus, PageDiagnosis, SyncEvent, SyncSession, SyncSource, UserProfile } from "./types";

const bootstrap = ref<AppBootstrap | null>(null);
const sessions = ref<SyncSession[]>([]);
const events = ref<SyncEvent[]>([]);
const items = ref<ContentItem[]>([]);
const profile = ref<UserProfile | null>(null);
const query = ref("");
const loading = ref(false);
const syncing = ref(false);
const loginChecking = ref(false);
const activeTab = ref<"history" | "logs" | "local">("local");
const syncSource = ref<SyncSource>("favorites");
const searchSource = ref<"all" | SyncSource>("favorites");
const searchContentType = ref<ContentTypeFilter>("all");
const selectedSessionId = ref<string>("");
const diagnosis = ref<PageDiagnosis | null>(null);
const loginStatus = ref<LoginStatus | null>(null);
const settingsOpen = ref(false);
const error = ref("");
let timer: number | undefined;

const latestSession = computed(() => sessions.value[0] ?? null);
const effectiveSessionId = computed(() => selectedSessionId.value || latestSession.value?.id || "");
const runningSession = computed(() => sessions.value.find((session) => session.status === "running") ?? null);

function parseRawJson(item: ContentItem) {
  try {
    return JSON.parse(item.rawJson || "{}");
  } catch {
    return {};
  }
}

function getListText(item: ContentItem) {
  const raw = parseRawJson(item);
  const html = raw?.list?.listHtml || "";
  if (typeof html !== "string" || !html.trim()) {
    return item.summary || item.title;
  }
  const doc = new DOMParser().parseFromString(html, "text/html");
  return (doc.body.textContent || item.summary || item.title).replace(/\s+/g, " ").trim();
}

function getListImage(item: ContentItem) {
  const raw = parseRawJson(item);
  const html = raw?.list?.listHtml || "";
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

async function loadAll() {
  loading.value = true;
  error.value = "";
  try {
    bootstrap.value = await bootstrapApp();
    document.title = bootstrap.value.appTitle;
    sessions.value = await listSessions();
    profile.value = await getUserProfile();
    if (!selectedSessionId.value && sessions.value[0]) {
      selectedSessionId.value = sessions.value[0].id;
    }
    events.value = await listSyncEvents(effectiveSessionId.value || undefined);
    items.value = await searchItemsWithType(
      query.value,
      searchSource.value === "all" ? undefined : searchSource.value,
      searchContentType.value === "all" ? undefined : searchContentType.value,
    );
    syncing.value = sessions.value.some((session) => session.status === "running");
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  } finally {
    loading.value = false;
  }
}

async function handleSync() {
  syncing.value = true;
  error.value = "";
  try {
    await handleCheckLogin();
    if (!loginStatus.value?.loggedIn) {
      syncing.value = false;
      return;
    }
    const session = await startSync({ source: syncSource.value, mode: "list" });
    selectedSessionId.value = session.id;
    await loadAll();
  } catch (err) {
    syncing.value = false;
    error.value = err instanceof Error ? err.message : String(err);
  }
}

async function handleDownloadContent() {
  syncing.value = true;
  error.value = "";
  try {
    await handleCheckLogin();
    if (!loginStatus.value?.loggedIn) {
      syncing.value = false;
      return;
    }
    const session = await startSync({ source: syncSource.value, mode: "download" });
    selectedSessionId.value = session.id;
    await loadAll();
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
    await loadAll();
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
    loginStatus.value = await checkToutiaoLogin({ source: syncSource.value });
    profile.value = await getUserProfile();
  } catch (err) {
    loginStatus.value = {
      loggedIn: false,
      loginRequired: true,
      source: syncSource.value,
      message: err instanceof Error ? err.message : String(err),
    };
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
      await loadAll();
    }
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  }
}



async function handleSearch() {
  try {
    activeTab.value = "local";
    items.value = await searchItemsWithType(
      query.value,
      searchSource.value === "all" ? undefined : searchSource.value,
      searchContentType.value === "all" ? undefined : searchContentType.value,
    );
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  }
}

watch(effectiveSessionId, async (value) => {
  if (!value) {
    events.value = [];
    return;
  }
  try {
    events.value = await listSyncEvents(value);
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  }
});

watch(syncSource, () => {
  loginStatus.value = null;
  diagnosis.value = null;
});

onMounted(async () => {
  await loadAll();
  await handleCheckLogin();
  timer = window.setInterval(loadAll, 3000);
});

onBeforeUnmount(() => {
  if (timer) {
    window.clearInterval(timer);
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
      <button :disabled="syncing" @click="handleSync">
        {{ syncing ? "同步中..." : "同步列表" }}
      </button>
      <button v-if="syncing" class="danger" @click="handleStopSync">停止同步</button>
      <button class="secondary" :disabled="syncing" @click="handleDownloadContent">下载内容</button>
      <button class="secondary" @click="handleDiagnose">诊断当前页面</button>
      <button class="secondary" @click="activeTab = 'history'">日志</button>
      <div class="toolbar-search">
        <span>⌕</span>
        <input
          v-model="query"
          placeholder="搜索"
          @keyup.enter="handleSearch"
        />
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
      </div>
      <div class="settings-actions">
        <button class="secondary" :disabled="syncing" @click="handleChooseDataDirectory">选择数据目录</button>
        <button class="secondary" :disabled="syncing" @click="handleMigrateDataDirectory">迁移数据目录</button>
        <button class="secondary" @click="openDownloadDir">打开下载目录</button>
      </div>
    </section>

    <section class="profile-shell">
      <div class="login-status" :class="{ ok: loginStatus?.loggedIn, danger: loginStatus?.loginRequired }">
        <strong>登录状态：{{ loginChecking ? "检查中..." : loginStatus?.loggedIn ? "已登录" : "未登录" }}</strong>
        <span>{{ loginStatus?.message || "点击“刷新”检查当前是否已登录今日头条" }}</span>
        <button class="status-refresh" type="button" :disabled="loginChecking" @click="handleCheckLogin">
          {{ loginChecking ? "刷新中..." : "刷新" }}
        </button>
      </div>
      <div class="profile-banner">
        <div class="profile-avatar">
          <img v-if="profile?.avatarUrl" :src="profile.avatarUrl" alt="" />
        </div>
        <div class="profile-info">
          <h1>{{ profile?.name || "未登录" }}</h1>
          <div class="profile-stats">
            <span><strong>{{ profile?.likes || "-" }}</strong>获赞</span>
            <span><strong>{{ profile?.followers || "-" }}</strong>粉丝</span>
            <span><strong>{{ profile?.following || "-" }}</strong>关注</span>
          </div>
          <p>简介：{{ profile?.bio || "同步列表后自动获取" }}</p>
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
          @click="activeTab = 'local'; searchSource = 'all'; handleSearch()"
        >
          全部
        </button>
        <button
          class="site-tab"
          :class="{ active: activeTab === 'local' && searchSource === 'favorites' }"
          role="tab"
          :aria-selected="activeTab === 'local' && searchSource === 'favorites'"
          @click="activeTab = 'local'; syncSource = 'favorites'; searchSource = 'favorites'; handleSearch()"
        >
          收藏
        </button>
        <button
          class="site-tab"
          :class="{ active: activeTab === 'local' && searchSource === 'likes' }"
          role="tab"
          :aria-selected="activeTab === 'local' && searchSource === 'likes'"
          @click="activeTab = 'local'; syncSource = 'likes'; searchSource = 'likes'; handleSearch()"
        >
          喜欢
        </button>
        <button class="site-tab spacer" type="button"></button>
        <button class="site-search" type="button" @click="handleSearch">⌕</button>
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
            <article v-for="item in items" :key="item.id" class="toutiao-item">
              <div class="toutiao-body">
                <h3>{{ item.title }}</h3>
                <p>{{ getOriginalLine(item) }}</p>
                <div class="toutiao-meta">
                  <span>{{ item.downloaded ? "已下载" : "未下载" }}</span>
                  <span v-if="item.author">{{ item.author }}</span>
                  <span>{{ item.syncedAt }}</span>
                  <span>{{ item.source === "favorites" ? "收藏" : "点赞" }}</span>
                  <span>{{ item.contentType === "video" ? "视频" : "文章" }}</span>
                  <a :href="item.sourceUrl" target="_blank" rel="noreferrer">原文</a>
                  <button
                    v-if="item.articlePath"
                    class="link-button"
                    @click="openItemFile(item.id, 'article')"
                  >
                    打开文章
                  </button>
                  <button
                    v-if="item.videoPath"
                    class="link-button"
                    @click="openItemFile(item.id, 'video')"
                  >
                    打开视频
                  </button>
                  <button v-if="item.localDir" class="link-button" @click="openItemDir(item.id)">本地目录</button>
                </div>
              </div>
              <div v-if="getListImage(item)" class="toutiao-thumb">
                <img :src="getListImage(item)" alt="" />
                <span v-if="item.contentType === 'video'" class="video-mark">▶</span>
              </div>
            </article>
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
