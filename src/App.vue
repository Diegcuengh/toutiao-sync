<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import appLogo from "./assets/logo.png";
import {
  bootstrapApp,
  checkToutiaoLogin,
  chooseDataDirectory,
  diagnosePage,
  launchDebugChrome,
  listSessions,
  listSyncEvents,
  migrateDataDirectory,
  openDownloadDir,
  openItemDir,
  openItemFile,
  searchItemsWithType,
  startSync,
} from "./lib/api";
import type { AppBootstrap, ContentItem, ContentTypeFilter, LoginStatus, PageDiagnosis, SyncEvent, SyncSession, SyncSource } from "./types";

const bootstrap = ref<AppBootstrap | null>(null);
const sessions = ref<SyncSession[]>([]);
const events = ref<SyncEvent[]>([]);
const items = ref<ContentItem[]>([]);
const query = ref("");
const loading = ref(false);
const syncing = ref(false);
const loginChecking = ref(false);
const activeTab = ref<"history" | "logs" | "local">("history");
const syncSource = ref<SyncSource>("favorites");
const searchSource = ref<"all" | SyncSource>("all");
const searchContentType = ref<ContentTypeFilter>("all");
const selectedSessionId = ref<string>("");
const diagnosis = ref<PageDiagnosis | null>(null);
const loginStatus = ref<LoginStatus | null>(null);
const error = ref("");
let timer: number | undefined;

const latestSession = computed(() => sessions.value[0] ?? null);
const effectiveSessionId = computed(() => selectedSessionId.value || latestSession.value?.id || "");

async function loadAll() {
  loading.value = true;
  error.value = "";
  try {
    bootstrap.value = await bootstrapApp();
    document.title = bootstrap.value.appTitle;
    sessions.value = await listSessions();
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
    const session = await startSync({ source: syncSource.value, mode: "incremental" });
    selectedSessionId.value = session.id;
    await loadAll();
  } catch (err) {
    syncing.value = false;
    error.value = err instanceof Error ? err.message : String(err);
  }
}

async function handleVerifySync() {
  syncing.value = true;
  error.value = "";
  try {
    await handleCheckLogin();
    if (!loginStatus.value?.loggedIn) {
      syncing.value = false;
      return;
    }
    const session = await startSync({ source: syncSource.value, mode: "verify", maxItems: 3 });
    selectedSessionId.value = session.id;
    await loadAll();
  } catch (err) {
    syncing.value = false;
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

async function handleLaunchDebugChrome() {
  error.value = "";
  try {
    await launchDebugChrome();
    loginStatus.value = {
      loggedIn: false,
      loginRequired: true,
      source: syncSource.value,
      message: "已打开 Chrome，请登录今日头条；登录后点击“检查登录”",
    };
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  }
}

async function handleSearch() {
  try {
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
    <section class="panel hero">
      <div>
        <img :src="appLogo" alt="今日收藏" class="hero-logo" />
        <h1>今日头条收藏/喜欢同步</h1>
        <p class="hint">
          默认数据目录是 `D:\toutiao-sync`；你也可以改目录或迁移目录。当前实现按增量入库：已入库内容会跳过，文章保存为 HTML/JSON，视频下载到本地目录。
        </p>
      </div>
      <div v-if="bootstrap" class="meta-grid">
        <span>版本：{{ bootstrap.buildLabel }}</span>
        <span>数据库：{{ bootstrap.dbPath }}</span>
        <span>数据目录：{{ bootstrap.dataDir }}</span>
        <span>下载目录：{{ bootstrap.downloadDir }}</span>
      </div>
      <p v-if="error" class="error">{{ error }}</p>
    </section>

    <section class="panel sync-panel">
      <div class="section-head">
        <h2>同步操作</h2>
        <span>{{ syncing ? "同步中" : "就绪" }}</span>
      </div>
      <div class="login-status" :class="{ ok: loginStatus?.loggedIn, danger: loginStatus?.loginRequired }">
        <strong>登录状态：{{ loginChecking ? "检查中..." : loginStatus?.loggedIn ? "已登录" : "未登录" }}</strong>
        <span>{{ loginStatus?.message || "点击“检查登录”查看当前 Chrome 是否已登录今日头条" }}</span>
        <a v-if="loginStatus?.pageUrl" :href="loginStatus.pageUrl" target="_blank" rel="noreferrer">当前页面</a>
      </div>
      <div class="hero-actions">
        <select v-model="syncSource">
          <option value="favorites">同步收藏</option>
          <option value="likes">同步喜欢</option>
        </select>
        <button :disabled="syncing" @click="handleSync">
          {{ syncing ? "同步中..." : "开始增量同步" }}
        </button>
        <button class="secondary" :disabled="syncing" @click="handleVerifySync">先验证 3 条</button>
        <button class="secondary" :disabled="loginChecking" @click="handleCheckLogin">
          {{ loginChecking ? "检查中..." : "检查登录" }}
        </button>
        <button class="secondary" @click="handleLaunchDebugChrome">启动调试 Chrome</button>
        <button class="secondary" @click="handleDiagnose">诊断当前页面</button>
        <button class="secondary" :disabled="syncing" @click="handleChooseDataDirectory">选择数据目录</button>
        <button class="secondary" :disabled="syncing" @click="handleMigrateDataDirectory">迁移数据目录</button>
        <button class="secondary" @click="openDownloadDir">打开下载目录</button>
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

    <section class="tab-shell">
      <div class="tabs content-tabs" role="tablist" aria-label="同步内容">
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
        <button
          class="tab-button"
          :class="{ active: activeTab === 'local' }"
          role="tab"
          :aria-selected="activeTab === 'local'"
          @click="activeTab = 'local'"
        >
          本地数据
        </button>
      </div>

      <div class="panel tab-content">
        <section v-if="activeTab === 'history'" class="tab-pane history-panel">
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
          <div class="section-head">
            <h2>本地内容</h2>
            <div class="search-bar">
              <select v-model="searchSource" @change="handleSearch">
                <option value="all">全部来源</option>
                <option value="favorites">只看收藏</option>
                <option value="likes">只看喜欢</option>
              </select>
              <select v-model="searchContentType" @change="handleSearch">
                <option value="all">全部类型</option>
                <option value="article">只看文章</option>
                <option value="video">只看视频</option>
              </select>
              <input
                v-model="query"
                placeholder="搜索标题、作者、摘要、正文"
                @keyup.enter="handleSearch"
              />
              <button class="secondary" @click="handleSearch">搜索</button>
            </div>
          </div>
          <div class="cards">
            <article v-for="item in items" :key="item.id" class="card">
              <div class="card-head">
                <span class="badge">{{ item.contentType }}</span>
                <span>{{ item.source === "favorites" ? "收藏" : "喜欢" }}</span>
              </div>
              <h3>{{ item.title }}</h3>
              <p>{{ (item.summary || item.contentText || "无摘要").slice(0, 220) }}</p>
              <div class="card-meta">
                <span>{{ item.author || "未知作者" }}</span>
                <span>{{ item.syncedAt }}</span>
                <span>{{ item.downloaded ? "已下载" : "仅索引" }}</span>
                <span>{{ item.coverPath ? "封面已存本地" : "无本地封面" }}</span>
              </div>
              <div class="card-actions">
                <a :href="item.sourceUrl" target="_blank" rel="noreferrer">原文</a>
                <button
                  v-if="item.articlePath"
                  class="secondary"
                  @click="openItemFile(item.id, 'article')"
                >
                  打开文章
                </button>
                <button
                  v-if="item.videoPath"
                  class="secondary"
                  @click="openItemFile(item.id, 'video')"
                >
                  打开视频
                </button>
                <button
                  v-if="item.coverPath"
                  class="secondary"
                  @click="openItemFile(item.id, 'cover')"
                >
                  打开封面
                </button>
                <button class="secondary" @click="openItemFile(item.id, 'raw')">打开 JSON</button>
                <button class="secondary" @click="openItemDir(item.id)">本地目录</button>
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
  width: 96px;
  height: 96px;
  display: block;
  border-radius: 20px;
  margin-bottom: 12px;
}
</style>


